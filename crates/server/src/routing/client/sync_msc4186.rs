use std::cmp;
use std::time::Duration;

use salvo::oapi::extract::*;
use salvo::prelude::*;

use crate::core::client::sync_events::v5::*;
use crate::data;
use crate::routing::prelude::*;

fn long_poll_timeout(timeout: Option<Duration>) -> Duration {
    let default = Duration::from_secs(30);
    cmp::min(timeout.unwrap_or(default), default)
}

fn has_list_count_changes(
    response: &SyncEventsResBody,
    previous_counts: &std::collections::BTreeMap<String, usize>,
) -> bool {
    response
        .lists
        .iter()
        .any(|(list_id, list)| previous_counts.get(list_id) != Some(&list.count))
}

/// `POST /_matrix/client/unstable/org.matrix.simplified_msc3575/sync`
/// ([MSC4186])
///
/// A simplified version of sliding sync ([MSC3575]).
///
/// Get all new events in a sliding window of rooms since the last sync or a
/// given point in time.
///
/// [MSC3575]: https://github.com/matrix-org/matrix-spec-proposals/pull/3575
/// [MSC4186]: https://github.com/matrix-org/matrix-spec-proposals/pull/4186
#[handler]
pub(super) async fn sync_events_v5(
    _aa: AuthArgs,
    args: SyncEventsReqArgs,
    req_body: JsonBody<SyncEventsReqBody>,
    depot: &mut Depot,
) -> JsonResult<SyncEventsResBody> {
    let authed = depot.authed_info()?;
    let sender_id = authed.user_id();
    let device_id = authed.device_id();

    let since_sn: i64 = args
        .pos
        .as_ref()
        .and_then(|string| string.parse().ok())
        .unwrap_or_default();

    let mut req_body = req_body.into_inner();

    let _conn_id = req_body.conn_id.clone();

    if since_sn == 0 {
        crate::sync_v5::forget_sync_request_connection(
            sender_id.to_owned(),
            device_id.to_owned(),
            req_body.conn_id.to_owned(),
        )
    }

    // Get sticky parameters from cache
    let (known_rooms, previous_list_counts) = crate::sync_v5::update_sync_request_with_cache(
        sender_id.to_owned(),
        device_id.to_owned(),
        &mut req_body,
    );

    let mut res_body =
        crate::sync_v5::sync_events(sender_id, device_id, since_sn, &req_body, &known_rooms)
            .await?;

    if since_sn > data::curr_sn()?
        || (args.pos.is_some()
            && res_body.is_empty_for_long_poll()
            && !has_list_count_changes(&res_body, &previous_list_counts))
    {
        let duration = long_poll_timeout(args.timeout);
        let watcher = crate::watcher::watch(sender_id, device_id);
        _ = tokio::time::timeout(duration, watcher).await;
        res_body =
            crate::sync_v5::sync_events(sender_id, device_id, since_sn, &req_body, &known_rooms)
                .await?;
    }

    crate::sync_v5::update_sync_list_counts(
        sender_id.to_owned(),
        device_id.to_owned(),
        req_body.conn_id.clone(),
        res_body
            .lists
            .iter()
            .map(|(list_id, list)| (list_id.clone(), list.count))
            .collect(),
    );

    trace!(
        rooms=?res_body.rooms.len(),
        account_data=?res_body.extensions.account_data.rooms.len(),
        receipts=?res_body.extensions.receipts.rooms.len(),
        "responding to request with"
    );
    json_ok(res_body)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::time::Duration;

    use super::{has_list_count_changes, long_poll_timeout};
    use crate::core::client::sync_events::v5::{SyncEventsResBody, SyncList};

    #[test]
    fn long_poll_timeout_honors_zero() {
        assert_eq!(
            long_poll_timeout(Some(Duration::from_secs(0))),
            Duration::from_secs(0)
        );
    }

    #[test]
    fn long_poll_timeout_is_capped_to_default() {
        assert_eq!(long_poll_timeout(None), Duration::from_secs(30));
        assert_eq!(
            long_poll_timeout(Some(Duration::from_secs(90))),
            Duration::from_secs(30)
        );
    }

    #[test]
    fn list_count_changes_are_still_treated_as_meaningful() {
        let mut response = SyncEventsResBody::new("42".to_owned());
        response
            .lists
            .insert("all_rooms".to_owned(), SyncList { count: 2, ops: Vec::new() });

        assert!(has_list_count_changes(
            &response,
            &BTreeMap::from([("all_rooms".to_owned(), 1)])
        ));
        assert!(!has_list_count_changes(
            &response,
            &BTreeMap::from([("all_rooms".to_owned(), 2)])
        ));
    }
}
