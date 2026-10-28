//! Durable per-client social presence. All publication uses the existing presence stream lock.
use crate::core::presence::revised::{self, Persistent, State, Status};
use crate::core::{UnixMillis, UserId};
use crate::{AppResult, MatrixError, data};
use diesel::{
    QueryableByName, sql_query,
    sql_types::{BigInt, Jsonb, Nullable, Text},
};
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use serde_json::Value;

#[derive(QueryableByName)]
struct Client {
    #[diesel(sql_type = Text)]
    state: String,
}
#[derive(QueryableByName)]
struct JsonRow {
    #[diesel(sql_type = Jsonb)]
    value: Value,
}
#[derive(QueryableByName)]
struct UserRow {
    #[diesel(sql_type = Text)]
    user_id: String,
}

pub async fn touch(user: &UserId, client: &str, state: &str, appservice: bool) -> AppResult<()> {
    if !crate::config::get().presence.allow_local {
        return Ok(());
    }
    let state = State::parse(state, true)
        .ok_or_else(|| MatrixError::invalid_param("Unknown presence state"))?;
    let client = if appservice || client.starts_with("device:") {
        client.to_owned()
    } else {
        format!("device:{client}")
    };
    let expires = (!appservice).then_some(UnixMillis::now().0 as i64 + 300_000);
    let mut conn = data::connect().await?;
    let changed = conn.transaction::<_, crate::AppError, _>(async |conn| {
        data::user::lock_presence_stream(conn).await?;
        sql_query("INSERT INTO revised_presence_clients (user_id,client_id,state,expires_at) VALUES ($1,$2,$3,$4) ON CONFLICT (user_id,client_id) DO UPDATE SET state=EXCLUDED.state,expires_at=EXCLUDED.expires_at")
            .bind::<Text,_>(user.as_str()).bind::<Text,_>(&client).bind::<Text,_>(state.as_str())
            .bind::<Nullable<BigInt>,_>(expires).execute(conn).await?;
        recompute(conn, user).await
    }).await?;
    super::recipients::wake_recipient_servers(user).await?;
    if changed {
        crate::sending::send_presence_appservices(user).await?;
    }
    Ok(())
}

pub async fn refresh(user: &UserId) -> AppResult<()> {
    let mut conn = data::connect().await?;
    let changed = conn
        .transaction::<_, crate::AppError, _>(async |conn| {
            data::user::lock_presence_stream(conn).await?;
            recompute(conn, user).await
        })
        .await?;
    super::recipients::wake_recipient_servers(user).await?;
    if changed {
        crate::sending::send_presence_appservices(user).await?;
    }
    Ok(())
}

/// Legacy clients can still edit their message without erasing extensible status or an override.
pub async fn legacy_message(user: &UserId, message: String) -> AppResult<()> {
    for _ in 0..8 {
        let rows = sql_query("SELECT json_data::jsonb AS value FROM user_datas WHERE user_id=$1 AND room_id IS NULL AND data_type=$2 AND NOT is_deleted ORDER BY id DESC LIMIT 1")
            .bind::<Text,_>(user.as_str()).bind::<Text,_>(revised::PERSISTENT)
            .load::<JsonRow>(&mut data::connect().await?).await?;
        let original = rows.into_iter().next().map(|row| row.value);
        let mut updated = original.clone().unwrap_or_else(|| serde_json::json!({}));
        serde_json::from_value::<Persistent>(updated.clone())?;
        if updated.get("status").is_none() {
            updated["status"] = serde_json::json!({});
        }
        updated["status"]["msg"] = Value::String(message.clone());
        if data::user::set_data_if_unchanged(
            user,
            None,
            revised::PERSISTENT,
            original.as_ref(),
            updated,
        )
        .await?
        {
            return Ok(());
        }
    }
    Err(crate::AppError::internal(
        "Concurrent presence settings updates; retry the request",
    ))
}

async fn recompute(conn: &mut AsyncPgConnection, user: &UserId) -> AppResult<bool> {
    sql_query("DELETE FROM revised_presence_clients WHERE user_id=$1 AND expires_at <= $2")
        .bind::<Text, _>(user.as_str())
        .bind::<BigInt, _>(UnixMillis::now().0 as i64)
        .execute(conn)
        .await?;
    let rows = sql_query("SELECT json_data::jsonb AS value FROM user_datas WHERE user_id=$1 AND room_id IS NULL AND data_type=$2 AND NOT is_deleted ORDER BY id DESC LIMIT 1")
        .bind::<Text,_>(user.as_str()).bind::<Text,_>(revised::PERSISTENT).load::<JsonRow>(conn).await?;
    let persistent = rows
        .into_iter()
        .next()
        .map(|r| serde_json::from_value::<Persistent>(r.value))
        .transpose()?
        .unwrap_or_default();
    let clients = sql_query("SELECT state FROM revised_presence_clients c WHERE user_id=$1 AND (expires_at IS NULL OR expires_at > $2) AND NOT EXISTS (SELECT 1 FROM revised_presence_outages o WHERE o.destination=c.client_id AND o.failed_at <= $2-300000)")
        .bind::<Text,_>(user.as_str()).bind::<BigInt,_>(UnixMillis::now().0 as i64).load::<Client>(conn).await?;
    let state = revised::resolve(
        clients.iter().filter_map(|c| State::parse(&c.state, true)),
        &persistent,
    );
    publish(conn, user, state, &persistent.status).await
}

async fn publish(
    conn: &mut AsyncPgConnection,
    user: &UserId,
    state: State,
    status: &Status,
) -> AppResult<bool> {
    let now = UnixMillis::now().0 as i64;
    let status = serde_json::to_value(status)?;
    // A status-only update must advance the stream too. Keep observer time across unchanged states.
    let changed = sql_query("INSERT INTO user_presences (user_id,state,status_msg,currently_active,last_active_at,updated_at,revised_status) VALUES ($1,$2,$3,$4,$5,$5,$7) ON CONFLICT (user_id) DO UPDATE SET state=EXCLUDED.state,status_msg=EXCLUDED.status_msg,currently_active=EXCLUDED.currently_active,last_active_at=CASE WHEN user_presences.state=$6 AND EXCLUDED.state<>$6 THEN $5 ELSE user_presences.last_active_at END,updated_at=$5,revised_status=EXCLUDED.revised_status,occur_sn=nextval('occur_sn_seq') WHERE user_presences.state IS DISTINCT FROM EXCLUDED.state OR user_presences.status_msg IS DISTINCT FROM EXCLUDED.status_msg OR user_presences.revised_status IS DISTINCT FROM EXCLUDED.revised_status")
        .bind::<Text,_>(user.as_str()).bind::<Text,_>(state.as_str())
        .bind::<Nullable<Text>,_>(status.get("msg").and_then(Value::as_str))
        .bind::<diesel::sql_types::Bool,_>(state == State::Active).bind::<BigInt,_>(now)
        .bind::<Text,_>(State::Active.as_str()).bind::<Jsonb,_>(&status).execute(conn).await?;
    Ok(changed > 0)
}

pub async fn incoming(
    user: &UserId,
    state: &str,
    active: bool,
    status: Option<Value>,
    legacy: Option<String>,
) -> AppResult<()> {
    let state = State::parse(state, active).unwrap_or(State::Offline);
    let status: Status = match status {
        Some(value) => serde_json::from_value(value)?,
        None => Status {
            msg: legacy,
            ..Default::default()
        },
    };
    let mut conn = data::connect().await?;
    conn.transaction::<_, crate::AppError, _>(async |conn| {
        data::user::lock_presence_stream(conn).await?;
        publish(conn, user, state, &status).await.map(|_| ())
    })
    .await
}

pub async fn project(user: &UserId, value: &mut Value, revised_states: bool) -> AppResult<()> {
    let rows = sql_query("SELECT revised_status AS value FROM user_presences WHERE user_id=$1")
        .bind::<Text, _>(user.as_str())
        .load::<JsonRow>(&mut data::connect().await?)
        .await?;
    let status: Status = if let Some(status) = value.get(revised::STATUS) {
        serde_json::from_value(status.clone())?
    } else {
        rows.into_iter()
            .next()
            .map(|r| serde_json::from_value(r.value))
            .transpose()?
            .unwrap_or_else(|| Status {
                msg: value
                    .get("status_msg")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                ..Default::default()
            })
    };
    let mut state = State::parse(
        value["presence"].as_str().unwrap_or("offline"),
        value["currently_active"].as_bool().unwrap_or(false),
    )
    .unwrap_or(State::Offline);
    let outages = sql_query("SELECT destination AS user_id FROM revised_presence_outages WHERE destination=$1 AND notified")
        .bind::<Text,_>(format!("remote:{}", user.server_name())).load::<UserRow>(&mut data::connect().await?).await?;
    if !outages.is_empty() {
        state = State::Offline;
    }
    revised::project(value, state, &status, revised_states);
    if state == State::Active
        && let Some(value) = value.as_object_mut()
    {
        value.remove("last_active_ago");
    }
    Ok(())
}

pub fn start() {
    tokio::spawn(async {
        if let Err(error) = recover().await {
            tracing::warn!(?error, "presence recovery failed");
        }
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            interval.tick().await;
            if let Err(error) = expire().await {
                tracing::warn!(?error, "presence expiry failed");
            }
        }
    });
}

async fn recover() -> AppResult<()> {
    let users = sql_query("SELECT DISTINCT user_id FROM revised_presence_clients UNION SELECT DISTINCT user_id FROM user_datas WHERE room_id IS NULL AND data_type=$1 AND NOT is_deleted UNION SELECT user_id FROM user_presences WHERE right(user_id,length($2)+1)=':'||$2")
        .bind::<Text,_>(revised::PERSISTENT).bind::<Text,_>(crate::config::server_name().as_str()).load::<UserRow>(&mut data::connect().await?).await?;
    for user in users {
        refresh(&UserId::parse(user.user_id)?).await?;
    }
    Ok(())
}

async fn expire() -> AppResult<()> {
    let users = sql_query("SELECT DISTINCT user_id FROM revised_presence_clients WHERE expires_at <= $1 AND state <> 'offline'")
        .bind::<BigInt,_>(UnixMillis::now().0 as i64).load::<UserRow>(&mut data::connect().await?).await?;
    for row in users {
        let user = UserId::parse(&row.user_id)?;
        refresh(&user).await?;
    }
    let outages = sql_query("SELECT destination AS user_id FROM revised_presence_outages WHERE NOT notified AND failed_at <= $1")
        .bind::<BigInt,_>(UnixMillis::now().0 as i64 - 300_000).load::<UserRow>(&mut data::connect().await?).await?;
    for outage in outages {
        publish_outage(&outage.user_id, false).await?;
    }
    Ok(())
}

/// Observe outbound delivery only; brief failures do not flap remote presence.
pub async fn delivery(kind: &crate::sending::OutgoingKind, success: bool) -> AppResult<()> {
    use crate::sending::OutgoingKind;
    let destination = match kind {
        OutgoingKind::Normal(server) => format!("remote:{server}"),
        OutgoingKind::Appservice(id) => format!("as:{id}"),
        _ => return Ok(()),
    };
    let mut conn = data::connect().await?;
    if success {
        publish_outage(&destination, true).await?;
    } else {
        sql_query("INSERT INTO revised_presence_outages (destination,failed_at) VALUES ($1,$2) ON CONFLICT DO NOTHING")
            .bind::<Text,_>(&destination).bind::<BigInt,_>(UnixMillis::now().0 as i64).execute(&mut conn).await?;
    }
    Ok(())
}

async fn publish_outage(destination: &str, recovered: bool) -> AppResult<()> {
    let mut conn = data::connect().await?;
    if let Some(server) = destination.strip_prefix("remote:") {
        let changed = conn.transaction::<_, crate::AppError, _>(async |conn| {
            data::user::lock_presence_stream(conn).await?;
            let changed = if recovered {
                sql_query("DELETE FROM revised_presence_outages WHERE destination=$1")
                    .bind::<Text,_>(destination).execute(conn).await?
            } else {
                sql_query("UPDATE revised_presence_outages SET notified=TRUE WHERE destination=$1 AND NOT notified")
                    .bind::<Text,_>(destination).execute(conn).await?
            };
            if changed == 0 { return Ok(false); }
            sql_query("UPDATE user_presences SET occur_sn=nextval('occur_sn_seq'),last_active_at=CASE WHEN state=$2 AND NOT $3 THEN $4 ELSE last_active_at END WHERE right(user_id,length($1)+1)=':'||$1")
                .bind::<Text,_>(server).bind::<Text,_>(State::Active.as_str())
                .bind::<diesel::sql_types::Bool,_>(recovered).bind::<BigInt,_>(UnixMillis::now().0 as i64).execute(conn).await?;
            Ok(true)
        }).await?;
        if !changed {
            return Ok(());
        }
        let users = sql_query(
            "SELECT user_id FROM user_presences WHERE right(user_id,length($1)+1)=':'||$1",
        )
        .bind::<Text, _>(server)
        .load::<UserRow>(&mut conn)
        .await?;
        for user in users {
            crate::sending::send_presence_appservices(&UserId::parse(user.user_id)?).await?;
        }
    } else {
        if recovered {
            sql_query("DELETE FROM revised_presence_outages WHERE destination=$1")
                .bind::<Text, _>(destination)
                .execute(&mut conn)
                .await?;
        }
        let users = sql_query("SELECT user_id FROM revised_presence_clients WHERE client_id=$1")
            .bind::<Text, _>(destination)
            .load::<UserRow>(&mut conn)
            .await?;
        for user in users {
            refresh(&UserId::parse(user.user_id)?).await?;
        }
        if !recovered {
            sql_query("UPDATE revised_presence_outages SET notified=TRUE WHERE destination=$1")
                .bind::<Text, _>(destination)
                .execute(&mut conn)
                .await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn durable_multidevice_overrides_status_and_remote_recovery() {
        crate::test_database::init();
        let user = UserId::parse("@presence:example.org").unwrap();
        let mut conn = data::connect().await.unwrap();
        sql_query("INSERT INTO revised_presence_clients (user_id,client_id,state,expires_at) VALUES ($1,'one',$2,NULL),($1,'two',$3,NULL)")
            .bind::<Text,_>(user.as_str()).bind::<Text,_>(State::Active.as_str()).bind::<Text,_>(State::Busy.as_str())
            .execute(&mut conn).await.unwrap();
        recompute(&mut conn, &user).await.unwrap();
        let state = sql_query("SELECT state FROM user_presences WHERE user_id=$1")
            .bind::<Text, _>(user.as_str())
            .get_result::<Client>(&mut conn)
            .await
            .unwrap();
        assert_eq!(state.state, State::Busy.as_str());
        data::user::set_data(&user, None, revised::PERSISTENT, json!({"state_override":State::Idle.as_str(),"status":{"msg":"hello","example.icon":"star"}})).await.unwrap();
        recompute(&mut conn, &user).await.unwrap();
        drop(conn); // Reopen the connection: no in-memory state is needed for reconstruction.
        let mut conn = data::connect().await.unwrap();
        recompute(&mut conn, &user).await.unwrap();
        let state = sql_query("SELECT state FROM user_presences WHERE user_id=$1")
            .bind::<Text, _>(user.as_str())
            .get_result::<Client>(&mut conn)
            .await
            .unwrap();
        assert_eq!(state.state, State::Idle.as_str());
        let mut wire = json!({"presence":state.state});
        project(&user, &mut wire, false).await.unwrap();
        assert_eq!(wire["presence"], "unavailable");
        assert_eq!(wire[revised::STATUS]["example.icon"], "star");
        legacy_message(&user, "legacy message".into())
            .await
            .unwrap();
        let saved: Persistent = data::user::get_data(&user, None, revised::PERSISTENT)
            .await
            .unwrap();
        assert_eq!(saved.status.msg.as_deref(), Some("legacy message"));
        assert_eq!(saved.status.extensions["example.icon"], "star");
        assert_eq!(saved.state_override.as_deref(), Some(State::Idle.as_str()));
        data::user::set_data(&user, None, revised::PERSISTENT, json!({}))
            .await
            .unwrap();
        recompute(&mut conn, &user).await.unwrap();
        wire = json!({"presence":State::Busy.as_str()});
        project(&user, &mut wire, true).await.unwrap();
        assert_eq!(wire["status_msg"], "");
        sql_query("UPDATE revised_presence_clients SET expires_at=0 WHERE user_id=$1")
            .bind::<Text, _>(user.as_str())
            .execute(&mut conn)
            .await
            .unwrap();
        recompute(&mut conn, &user).await.unwrap();
        let state = sql_query("SELECT state FROM user_presences WHERE user_id=$1")
            .bind::<Text, _>(user.as_str())
            .get_result::<Client>(&mut conn)
            .await
            .unwrap();
        assert_eq!(state.state, "offline");
        incoming(
            &user,
            State::Active.as_str(),
            true,
            Some(json!({"msg":"remote","example.icon":"moon"})),
            Some("old".into()),
        )
        .await
        .unwrap();
        sql_query("INSERT INTO revised_presence_outages (destination,failed_at,notified) VALUES ('remote:example.org',0,TRUE)").execute(&mut conn).await.unwrap();
        let mut wire = json!({"presence":State::Active.as_str()});
        project(&user, &mut wire, true).await.unwrap();
        assert_eq!(wire["presence"], "offline");
        assert_eq!(wire["status_msg"], "remote");
        delivery(
            &crate::sending::OutgoingKind::Normal(user.server_name().to_owned()),
            true,
        )
        .await
        .unwrap();
        wire["presence"] = json!(State::Active.as_str());
        project(&user, &mut wire, true).await.unwrap();
        assert_eq!(wire["presence"], State::Active.as_str());
    }
}
