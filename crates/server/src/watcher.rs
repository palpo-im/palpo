use std::time::Duration;

use diesel_async::RunQueryDsl;

use crate::AppResult;
use crate::core::Seqnum;
use crate::core::identifiers::*;
use crate::data::connect;

#[derive(Debug, diesel::QueryableByName)]
struct WatchSnapshot {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    typing_sn: i64,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    inbox_id: i64,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    key_change_id: i64,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    membership_id: i64,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    event_sn: i64,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    account_data_sn: i64,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    profile_sn: i64,
}

impl WatchSnapshot {
    fn has_updates_since(&self, previous: &Self) -> bool {
        self.typing_sn > previous.typing_sn
            || self.inbox_id > previous.inbox_id
            || self.key_change_id > previous.key_change_id
            || self.membership_id > previous.membership_id
            || self.event_sn > previous.event_sn
            || self.account_data_sn > previous.account_data_sn
            || self.profile_sn > previous.profile_sn
    }
}

async fn load_watch_snapshot(
    user_id: &UserId,
    device_id: &DeviceId,
    profiles: bool,
) -> AppResult<WatchSnapshot> {
    // One statement gives all streams a consistent snapshot instead of issuing
    // seven sequential round trips for each idle device on every poll.
    diesel::sql_query(
        "WITH joined_rooms AS (
            SELECT room_id FROM (
                SELECT DISTINCT ON (room_id) room_id, membership FROM room_users
                WHERE user_id = $1 ORDER BY room_id, id DESC
            ) AS memberships WHERE membership = 'join'
        )
        SELECT
          COALESCE((SELECT MAX(occur_sn) FROM room_typings WHERE room_id IN (SELECT room_id FROM joined_rooms)), 0) AS typing_sn,
          COALESCE((SELECT MAX(id) FROM device_inboxes WHERE user_id = $1 AND device_id = $2), 0) AS inbox_id,
          COALESCE((SELECT MAX(id) FROM e2e_key_changes WHERE user_id = $1), 0) AS key_change_id,
          COALESCE((SELECT MAX(id) FROM room_users WHERE user_id = $1), 0) AS membership_id,
          COALESCE((SELECT MAX(event_sn) FROM event_points WHERE frame_id IS NOT NULL AND room_id IN (SELECT room_id FROM joined_rooms)), 0) AS event_sn,
          COALESCE((SELECT MAX(occur_sn) FROM user_datas WHERE user_id = $1), 0) AS account_data_sn,
          CASE WHEN $3 THEN COALESCE((
            SELECT MAX(occur_sn) FROM user_profile_changes WHERE user_id = $1 OR user_id IN (
                SELECT user_id FROM (
                    SELECT DISTINCT ON (user_id, room_id) user_id, room_id, membership
                    FROM room_users WHERE room_id IN (SELECT room_id FROM joined_rooms)
                    ORDER BY user_id, room_id, id DESC
                ) AS shared_members WHERE membership = 'join'
            )
          ), 0) ELSE 0 END AS profile_sn",
    )
    .bind::<diesel::sql_types::Text, _>(user_id.as_str())
    .bind::<diesel::sql_types::Text, _>(device_id.as_str())
    .bind::<diesel::sql_types::Bool, _>(profiles)
    .get_result(&mut connect().await?).await.map_err(Into::into)
}

fn profile_change_is_ready(
    profile_updates: bool,
    profile_after_sn: Option<Seqnum>,
    latest_change_sn: Seqnum,
) -> bool {
    profile_updates && profile_after_sn.is_some_and(|after_sn| latest_change_sn >= after_sn)
}

pub async fn watch(
    user_id: &UserId,
    device_id: &DeviceId,
    profile_updates: bool,
    profile_after_sn: Option<Seqnum>,
) -> AppResult<()> {
    let baseline = load_watch_snapshot(user_id, device_id, profile_updates).await?;
    if profile_change_is_ready(profile_updates, profile_after_sn, baseline.profile_sn) {
        return Ok(());
    }
    tokio::select! {
        _ = crate::ROTATE.watch() => Ok(()),
        result = async {
            for _ in 0..10 {
                tokio::time::sleep(Duration::from_secs(3)).await;
                if load_watch_snapshot(user_id, device_id, profile_updates).await?.has_updates_since(&baseline) {
                    return Ok(());
                }
            }
            Ok(())
        } => result,
    }
}

#[cfg(test)]
mod tests {
    use super::profile_change_is_ready;

    #[test]
    fn profile_changes_at_or_after_the_response_token_are_already_ready() {
        assert!(profile_change_is_ready(true, Some(10), 10));
        assert!(profile_change_is_ready(true, Some(10), 11));
        assert!(!profile_change_is_ready(true, Some(10), 9));
        assert!(!profile_change_is_ready(false, Some(10), 11));
        assert!(!profile_change_is_ready(true, None, 11));
    }
}

#[cfg(test)]
mod database_tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn database_watch_snapshot_covers_streams_without_unrelated_wakeups() {
        crate::test_database::init();
        let user: OwnedUserId = "@watch:example.org".try_into().unwrap();
        let device: &DeviceId = "WATCH".into();
        diesel::sql_query("INSERT INTO room_users (event_id,event_sn,room_id,user_id,user_server_id,sender_id,membership,created_at) VALUES ('$watch-member',1,'!watch:example.org','@watch:example.org','example.org','@watch:example.org','join',0),('$watch-peer',2,'!watch:example.org','@peer:example.org','example.org','@peer:example.org','join',0)")
            .execute(&mut connect().await.unwrap()).await.unwrap();
        let mut previous = load_watch_snapshot(&user, device, true).await.unwrap();
        assert!(!previous.has_updates_since(&previous));
        // A different device's inbox and an unrelated profile are irrelevant.
        diesel::sql_query("INSERT INTO device_inboxes(user_id,device_id,json_data,created_at) VALUES ('@watch:example.org','OTHER','{}',0)")
            .execute(&mut connect().await.unwrap()).await.unwrap();
        diesel::sql_query("INSERT INTO user_profile_changes(occur_sn,user_id,field,value,removed) VALUES (900,'@stranger:example.org','displayname','\"unrelated\"',false)")
            .execute(&mut connect().await.unwrap()).await.unwrap();
        assert!(
            !load_watch_snapshot(&user, device, true)
                .await
                .unwrap()
                .has_updates_since(&previous)
        );
        for sql in [
            "INSERT INTO device_inboxes(user_id,device_id,json_data,created_at) VALUES ('@watch:example.org','WATCH','{}',0)",
            "INSERT INTO room_typings(room_id,user_id,timeout_at,occur_sn) VALUES ('!watch:example.org','@peer:example.org',999999,20)",
            "INSERT INTO e2e_key_changes(user_id,occur_sn,changed_at) VALUES ('@watch:example.org',30,0)",
            "INSERT INTO event_points(event_id,event_sn,room_id,frame_id) VALUES ('$watch-event',40,'!watch:example.org',1)",
            "INSERT INTO user_datas(user_id,data_type,json_data,is_deleted,occur_sn,created_at) VALUES ('@watch:example.org','m.push_rules','{}',false,50,0)",
            "INSERT INTO user_profile_changes(occur_sn,user_id,field,value,removed) VALUES (60,'@watch:example.org','displayname','\"self\"',false)",
            "INSERT INTO user_profile_changes(occur_sn,user_id,field,value,removed) VALUES (70,'@peer:example.org','displayname','\"shared\"',false)",
            "INSERT INTO room_users(event_id,event_sn,room_id,user_id,user_server_id,sender_id,membership,created_at) VALUES ('$watch-new',80,'!new:example.org','@watch:example.org','example.org','@watch:example.org','join',0)",
        ] {
            diesel::sql_query(sql)
                .execute(&mut connect().await.unwrap())
                .await
                .unwrap();
            let current = load_watch_snapshot(&user, device, true).await.unwrap();
            assert!(
                current.has_updates_since(&previous),
                "did not observe {sql}"
            );
            previous = current;
        }
        assert_eq!(previous.profile_sn, 70);
        assert_eq!(
            load_watch_snapshot(&user, device, false)
                .await
                .unwrap()
                .profile_sn,
            0
        );
        // Events in other rooms and events not yet promoted to the timeline do not wake.
        diesel::sql_query("INSERT INTO event_points(event_id,event_sn,room_id,frame_id) VALUES ('$watch-outside',1000,'!outside:example.org',1),('$watch-outlier',1001,'!watch:example.org',NULL)")
            .execute(&mut connect().await.unwrap()).await.unwrap();
        assert!(
            !load_watch_snapshot(&user, device, true)
                .await
                .unwrap()
                .has_updates_since(&previous)
        );
    }
}
