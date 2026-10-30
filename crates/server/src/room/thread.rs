use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::core::client::room::IncludeThreads;
use crate::core::events::relation::BundledThread;
use crate::core::identifiers::*;
use crate::core::serde::CanonicalJsonValue;
use crate::data::room::DbThread;
use crate::room::timeline;
use crate::{AppResult, MatrixError, SnPduEvent, data};

mod pagination;

const THREAD_SCAN_BUDGET: usize = 512;
const THREAD_SCAN_BATCH: usize = 128;

#[derive(Serialize, Deserialize)]
pub(crate) struct ThreadCursor {
    room_id: OwnedRoomId,
    last_sn: i64,
    pub(crate) event_id: OwnedEventId,
}

impl ThreadCursor {
    fn parse(token: &str, room_id: &RoomId) -> Result<Self, MatrixError> {
        let invalid = || MatrixError::invalid_param("Invalid thread pagination token.");
        if token.len() > 4096 {
            return Err(invalid());
        }
        let bytes = URL_SAFE_NO_PAD
            .decode(token.strip_prefix("t1.").ok_or_else(invalid)?)
            .map_err(|_| invalid())?;
        let cursor: Self = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        if cursor.room_id != room_id || cursor.last_sn < 0 {
            return Err(invalid());
        }
        Ok(cursor)
    }

    #[cfg(test)]
    fn encode(&self) -> String {
        format!(
            "t1.{}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(self).expect("thread cursor serializes"))
        )
    }
}

pub(crate) async fn parse_cursor(
    token: &str,
    room: &RoomId,
    user: &UserId,
    include: &IncludeThreads,
) -> AppResult<ThreadCursor> {
    if token.starts_with("t2.") {
        return pagination::decode(token, room, user, include);
    }
    let cursor = ThreadCursor::parse(token, room)?;
    let root = match timeline::get_pdu(&cursor.event_id).await {
        Ok(pdu) => pdu,
        Err(error) if error.is_not_found() => return Err(pagination::invalid()),
        Err(error) => return Err(error),
    };
    if root.room_id != room || !root.user_can_see(user).await? {
        return Err(pagination::invalid());
    }
    // Legacy tokens contain a visible anchor and its activity at issuance. A
    // later reply may have moved threads.last_sn, so validate the historical
    // activity by its indexed sequence rather than comparing mutable state.
    let activity_id = match crate::event::get_event_id_by_sn(cursor.last_sn).await {
        Ok(id) => id,
        Err(error) if error.is_not_found() => return Err(pagination::invalid()),
        Err(error) => return Err(error),
    };
    let activity = match timeline::get_pdu(&activity_id).await {
        Ok(pdu) => pdu,
        Err(error) if error.is_not_found() => return Err(pagination::invalid()),
        Err(error) => return Err(error),
    };
    let content: serde_json::Value = serde_json::from_str(activity.content.get())?;
    if activity.room_id != room
        || (activity.event_id != root.event_id
            && !(content["m.relates_to"]["rel_type"] == "m.thread"
                && content["m.relates_to"]["event_id"] == root.event_id.as_str()))
    {
        return Err(pagination::invalid());
    }
    Ok(cursor)
}

pub async fn get_threads(
    room_id: &RoomId,
    user_id: &UserId,
    include: &IncludeThreads,
    limit: i64,
    from_token: Option<&str>,
) -> AppResult<(Vec<(OwnedEventId, SnPduEvent)>, Option<String>)> {
    if !(1..=100).contains(&limit) {
        return Err(MatrixError::invalid_param("Thread limit must be between 1 and 100.").into());
    }
    let cursor = if let Some(token) = from_token {
        Some(parse_cursor(token, room_id, user_id, include).await?)
    } else {
        None
    };
    let mut scan_cursor = cursor;
    let mut visible = Vec::with_capacity((limit + 1) as usize);
    let mut examined = 0;
    let mut exhausted = false;
    // A private room may have arbitrarily many invisible roots. Bound the total
    // visibility work, not just each query, and encrypt internal continuation.
    'scan: while examined < THREAD_SCAN_BUDGET {
        let batch_limit = THREAD_SCAN_BATCH.min(THREAD_SCAN_BUDGET - examined);
        let items = data::room::list_threads(
            room_id,
            scan_cursor.as_ref().map(|c| (c.last_sn, &*c.event_id)),
            batch_limit as i64,
        )
        .await?;
        let last_batch = items.len() < batch_limit;
        for (event_id, last_sn) in items {
            examined += 1;
            scan_cursor = Some(ThreadCursor {
                room_id: room_id.to_owned(),
                last_sn,
                event_id: event_id.clone(),
            });
            let pdu = timeline::get_pdu(&event_id).await?;
            if !pdu.user_can_see(user_id).await? {
                continue;
            }
            visible.push((event_id, last_sn, pdu));
            if visible.len() as i64 > limit {
                break 'scan;
            }
        }
        if last_batch {
            exhausted = true;
            break;
        }
    }
    let has_more = visible.len() as i64 > limit;
    visible.truncate(limit as usize);
    let next_cursor = if has_more {
        // Restart after the last RETURNED visible root, not after lookahead.
        // Otherwise the next page would lose that unreturned visible event.
        visible.last().map(|(event_id, last_sn, _)| ThreadCursor {
            room_id: room_id.to_owned(),
            last_sn: *last_sn,
            event_id: event_id.clone(),
        })
    } else if !exhausted {
        scan_cursor
    } else {
        None
    };
    let next_token = next_cursor
        .map(|cursor| pagination::encode(cursor, room_id, user_id, include))
        .transpose()?;
    let events = visible
        .into_iter()
        .map(|(event_id, _, pdu)| (event_id, pdu))
        .collect();
    Ok((events, next_token))
}

pub async fn add_to_thread(thread_id: &EventId, pdu: &SnPduEvent) -> AppResult<()> {
    let (root_pdu, mut root_pdu_json) = timeline::get_pdu_and_data(thread_id).await?;

    if let CanonicalJsonValue::Object(unsigned) = root_pdu_json
        .entry("unsigned".to_owned())
        .or_insert_with(|| CanonicalJsonValue::Object(Default::default()))
    {
        if let Some(mut relations) = unsigned
            .get("m.relations")
            .and_then(|r| r.as_object())
            .and_then(|r| r.get("m.thread"))
            .and_then(|relations| {
                serde_json::from_value::<BundledThread>(relations.clone().into()).ok()
            })
        {
            // Thread already existed
            relations.count += 1;
            relations.latest_event = pdu.to_message_like_event_without_transaction_id();

            let content = serde_json::to_value(relations).expect("to_value always works");

            unsigned.insert(
                "m.relations".to_owned(),
                json!({ "m.thread": content })
                    .try_into()
                    .expect("thread is valid json"),
            );
        } else {
            // New thread
            let relations = BundledThread {
                latest_event: pdu.to_message_like_event_without_transaction_id(),
                count: 1,
                current_user_participated: true,
            };

            let content = serde_json::to_value(relations).expect("to_value always works");

            unsigned.insert(
                "m.relations".to_owned(),
                json!({ "m.thread": content })
                    .try_into()
                    .expect("thread is valid json"),
            );
        }

        timeline::replace_pdu(thread_id, &root_pdu_json).await?;
    }

    data::room::set_event_thread_id(&pdu.event_id, thread_id).await?;

    data::room::upsert_thread(DbThread {
        event_id: root_pdu.event_id.clone(),
        event_sn: root_pdu.event_sn,
        room_id: root_pdu.room_id.clone(),
        last_id: pdu.event_id.clone(),
        last_sn: pdu.event_sn,
    })
    .await?;
    Ok(())
}

#[cfg(test)]
mod pagination_tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn thread_cursors_validate_version_room_and_position() {
        let room = crate::core::room_id!("!thread:example.test");
        let cursor = ThreadCursor {
            room_id: room.to_owned(),
            last_sn: 100,
            event_id: crate::core::event_id!("$root").to_owned(),
        };
        let decoded = ThreadCursor::parse(&cursor.encode(), room).unwrap();
        assert_eq!(decoded.last_sn, 100);
        assert_eq!(decoded.event_id, cursor.event_id);
        assert!(
            ThreadCursor::parse(
                &cursor.encode(),
                crate::core::room_id!("!other:example.test")
            )
            .is_err()
        );
        for token in ["99", "t2.invalid", "t1.???", "t1.e30"] {
            assert!(ThreadCursor::parse(token, room).is_err());
        }
        let negative = ThreadCursor {
            last_sn: -1,
            ..cursor
        };
        assert!(ThreadCursor::parse(&negative.encode(), room).is_err());
    }

    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn database_thread_activity_pagination_has_no_gaps_or_duplicates() {
        crate::test_database::init();
        let room = crate::core::room_id!("!thread-pagination:example.test");
        let fixtures = [
            ("$oldest-root", 10, 100),
            ("$middle-root", 20, 90),
            ("$newest-root", 30, 80),
            ("$tie-z", 40, 80),
        ];
        for (root, event_sn, last_sn) in fixtures {
            let event_id = EventId::parse(root).unwrap();
            data::room::upsert_thread(DbThread {
                event_id: event_id.clone(),
                event_sn,
                room_id: room.to_owned(),
                last_id: EventId::parse(format!("$reply{last_sn}-{event_sn}")).unwrap(),
                last_sn,
            })
            .await
            .unwrap();
        }
        let mut token: Option<String> = None;
        let mut seen = Vec::new();
        loop {
            let cursor = token
                .as_deref()
                .map(|token| ThreadCursor::parse(token, room).unwrap());
            let page = data::room::list_threads(
                room,
                cursor.as_ref().map(|c| (c.last_sn, &*c.event_id)),
                1,
            )
            .await
            .unwrap();
            let Some((id, last_sn)) = page.first() else {
                break;
            };
            seen.push(id.to_string());
            token = Some(
                ThreadCursor {
                    room_id: room.to_owned(),
                    event_id: id.clone(),
                    last_sn: *last_sn,
                }
                .encode(),
            );
            if seen.len() == 1 {
                // A new reply in an already returned thread must not make it
                // reappear when the client follows its previous cursor.
                data::room::upsert_thread(DbThread {
                    event_id: id.clone(),
                    event_sn: 10,
                    room_id: room.to_owned(),
                    last_id: crate::core::event_id!("$new-reply").to_owned(),
                    last_sn: 110,
                })
                .await
                .unwrap();
            }
            assert!(
                seen.len() <= fixtures.len(),
                "pagination does not terminate"
            );
        }
        assert_eq!(
            seen,
            ["$oldest-root", "$middle-root", "$tie-z", "$newest-root"]
        );
        assert_eq!(seen.iter().collect::<HashSet<_>>().len(), fixtures.len());
    }
}
