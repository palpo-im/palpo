
use std::collections::{BTreeMap};
use std::time::Duration;

use tokio::sync::RwLock;

use crate::core::events::room::member::{MembershipState, RoomMemberEventContent};
use crate::core::identifiers::*;
use crate::core::serde::{
    CanonicalJsonObject, CanonicalJsonValue, RawJsonValue,
};
use crate::core::{UnixMillis, federation};
use crate::room::state::{self, CompressedEvent};
use crate::{AppError, AppResult, MatrixError, SigningKeys};

mod banned;
mod forget;
mod invite;
mod join;
mod knock;
mod leave;
pub use banned::*;
pub use forget::*;
pub use invite::*;
pub use join::*;
pub use knock::*;
pub use leave::*;

async fn validate_and_add_event_id(
    pdu: &RawJsonValue,
    room_version: &RoomVersionId,
    pub_key_map: &RwLock<BTreeMap<String, SigningKeys>>,
) -> AppResult<(OwnedEventId, CanonicalJsonObject)> {
    let mut value: CanonicalJsonObject = serde_json::from_str(pdu.get()).map_err(|e| {
        error!("Invalid PDU in server response: {:?}: {:?}", pdu, e);
        AppError::public("Invalid PDU in server response")
    })?;
    let event_id = EventId::parse(format!(
        "${}",
        crate::core::signatures::reference_hash(&value, room_version).expect("palpo can calculate reference hashes")
    ))
    .expect("palpo's reference hash~es are valid event ids");

    // TODO
    // let back_off = |id| match crate::BAD_EVENT_RATE_LIMITER.write().unwrap().entry(id) {
    //     Entry::Vacant(e) => {
    //         e.insert((Instant::now(), 1));
    //     }
    //     Entry::Occupied(mut e) => *e.get_mut() = (Instant::now(), e.get().1 + 1),
    // };

    if let Some((time, tries)) = crate::BAD_EVENT_RATE_LIMITER.read().unwrap().get(&event_id) {
        // Exponential backoff
        let mut min_elapsed_duration = Duration::from_secs(30) * (*tries) * (*tries);
        if min_elapsed_duration > Duration::from_secs(60 * 60 * 24) {
            min_elapsed_duration = Duration::from_secs(60 * 60 * 24);
        }

        if time.elapsed() < min_elapsed_duration {
            debug!("Backing off from {}", event_id);
            return Err(AppError::public("bad event, still backing off"));
        }
    }

    let origin_server_ts = value.get("origin_server_ts").ok_or_else(|| {
        error!("Invalid PDU, no origin_server_ts field");
        MatrixError::missing_param("Invalid PDU, no origin_server_ts field")
    })?;

    let origin_server_ts: UnixMillis = {
        let ts = origin_server_ts
            .as_integer()
            .ok_or_else(|| MatrixError::invalid_param("origin_server_ts must be an integer"))?;

        UnixMillis(
            ts.try_into()
                .map_err(|_| MatrixError::invalid_param("Time must be after the unix epoch"))?,
        )
    };

    let unfiltered_keys = (*pub_key_map.read().await).clone();

    let keys = crate::filter_keys_server_map(unfiltered_keys, origin_server_ts, room_version);

    // TODO
    // if let Err(e) = crate::core::signatures::verify_event(&keys, &value, room_version) {
    //     warn!("Event {} failed verification {:?} {}", event_id, pdu, e);
    //     back_off(event_id);
    //     return Err(AppError::public("Event failed verification."));
    // }

    value.insert(
        "event_id".to_owned(),
        CanonicalJsonValue::String(event_id.as_str().to_owned()),
    );

    Ok((event_id, value))
}
