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
        response.lists.insert(
            "all_rooms".to_owned(),
            SyncList {
                count: 2,
                ops: Vec::new(),
            },
        );

        assert!(has_list_count_changes(
            &response,
            &BTreeMap::from([("all_rooms".to_owned(), 1)])
        ));
        assert!(!has_list_count_changes(
            &response,
            &BTreeMap::from([("all_rooms".to_owned(), 2)])
        ));
    }

    /// Walks through the full handler-level decision sequence for the bug from
    /// the field report:
    ///
    ///   1. Step 1 – fresh sync: response carries `count + ops`.
    ///   2. The handler caches `previous_list_counts = { all_rooms: count }`.
    ///   3. Step 2 – idle re-poll (`since_sn > curr_sn`): server takes the fast-path. After the
    ///      fix, the response body is shaped like `{ pos, lists: { all_rooms: { count } } }` (no
    ///      ops).
    ///   4. The handler must: a. treat the body as long-poll-empty; b. NOT regard it as a
    ///      list-count change (otherwise long-polling would be skipped); c. write the same `count`
    ///      back into the cache so subsequent requests stay consistent.
    ///   5. The serialized JSON must include `count` so a client like matrix-rust-sdk can drive
    ///      `SlidingSyncList::is_fully_loaded`.
    ///
    /// Before the fix, step 3 returned `{ pos }` with no `lists` field, the
    /// SDK saw `count = None`, and it tight-polled the server.
    #[test]
    fn idle_repoll_preserves_count_and_long_poll_semantics() {
        // Step 1 – fresh sync response.
        let mut step1 = SyncEventsResBody::new("200".to_owned());
        step1.lists.insert(
            "all_rooms".to_owned(),
            SyncList {
                count: 2,
                ops: Vec::new(),
            },
        );

        // Cache update that the handler performs after step 1.
        let cached_counts: BTreeMap<String, usize> = step1
            .lists
            .iter()
            .map(|(id, l)| (id.clone(), l.count))
            .collect();
        assert_eq!(cached_counts.get("all_rooms"), Some(&2));

        // Step 2 – fast-path response after the fix (count-only list, no ops,
        // no rooms, no extension data).
        let mut step2 = SyncEventsResBody::new("200".to_owned());
        step2.lists.insert(
            "all_rooms".to_owned(),
            SyncList {
                count: 2,
                ops: Vec::new(),
            },
        );

        // (a) handler treats this as "no incremental updates"
        assert!(step2.is_empty_for_long_poll());
        // (b) but recognises it carries no list-count change either, so the
        //     long-poll guard fires (the request should hang on the watcher
        //     instead of returning immediately).
        assert!(!has_list_count_changes(&step2, &cached_counts));

        // (c) cache write after step 2 is idempotent.
        let cached_counts_after: BTreeMap<String, usize> = step2
            .lists
            .iter()
            .map(|(id, l)| (id.clone(), l.count))
            .collect();
        assert_eq!(cached_counts_after, cached_counts);

        // (d) wire shape: `lists.all_rooms.count` is present, ops omitted.
        let json = serde_json::to_value(&step2).unwrap();
        assert_eq!(json["lists"]["all_rooms"]["count"], 2);
        assert!(json["lists"]["all_rooms"].get("ops").is_none());
        // The pre-fix bug repro: the response would have been `{ "pos": "200" }`
        // with no `lists` key at all.
        assert!(
            json.get("lists")
                .is_some_and(|v| !v.as_object().unwrap().is_empty())
        );
    }

    /// When the user joins (or leaves) a room between two idle re-polls, the
    /// server-side `count` must reflect the new size and the handler must NOT
    /// long-poll the request — the count change itself is a meaningful update.
    /// Mirrors the bug doc's `incremental_sync_count_reflects_real_total_after_join`.
    #[test]
    fn count_change_after_room_join_breaks_out_of_long_poll() {
        let cached_counts = BTreeMap::from([("all_rooms".to_owned(), 2)]);

        // Idle re-poll AFTER the user joined a third room: fast-path emits
        // count=3 with no ops (ops would arrive on the next non-fast-path
        // sync, when the client re-issues with a fresh pos).
        let mut response = SyncEventsResBody::new("201".to_owned());
        response.lists.insert(
            "all_rooms".to_owned(),
            SyncList {
                count: 3,
                ops: Vec::new(),
            },
        );

        // Long-poll guard: the body is technically empty for long-poll
        // purposes (no rooms, no extensions), BUT the count change must
        // promote the response to "meaningful" so we return immediately.
        assert!(response.is_empty_for_long_poll());
        assert!(has_list_count_changes(&response, &cached_counts));
    }

    /// A list that the client just introduced (no entry in the previous
    /// counts cache) is also a count change and must be returned to the
    /// client immediately, not hidden behind the long-poll guard.
    #[test]
    fn newly_introduced_list_is_treated_as_count_change() {
        // The cache has counts for an unrelated list only.
        let cached_counts = BTreeMap::from([("dms".to_owned(), 0)]);

        let mut response = SyncEventsResBody::new("200".to_owned());
        response.lists.insert(
            "all_rooms".to_owned(),
            SyncList {
                count: 2,
                ops: Vec::new(),
            },
        );

        assert!(has_list_count_changes(&response, &cached_counts));
    }
}
