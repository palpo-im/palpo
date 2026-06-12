use std::sync::{Arc, LazyLock, Mutex};

use lru_cache::LruCache;

use super::{CompressedState, StateDiff};
use crate::core::identifiers::*;
use crate::{AppResult, MatrixError, data};

pub static STATE_INFO_CACHE: LazyLock<Mutex<LruCache<i64, Vec<FrameInfo>>>> =
    LazyLock::new(|| Mutex::new(LruCache::new(100_000)));

#[derive(Clone, Default)]
pub struct FrameInfo {
    pub frame_id: i64,
    pub full_state: Arc<CompressedState>,
    pub appended: Arc<CompressedState>,
    pub disposed: Arc<CompressedState>,
}

/// Returns a stack with info on state_hash, full state, added diff and removed diff for the
/// selected state_hash and each parent layer.
pub async fn load_frame_info(frame_id: i64) -> AppResult<Vec<FrameInfo>> {
    if let Some(r) = STATE_INFO_CACHE.lock().unwrap().get_mut(&frame_id) {
        return Ok(r.clone());
    }

    let StateDiff {
        parent_id,
        appended,
        disposed,
    } = super::load_state_diff(frame_id).await?;

    if let Some(parent_id) = parent_id {
        let mut info = Box::pin(load_frame_info(parent_id)).await?;
        let mut full_state = (*info.last().expect("at least one frame").full_state).clone();
        full_state.extend(appended.iter().copied());
        let disposed = (*disposed).clone();
        for r in &disposed {
            full_state.remove(r);
        }

        info.push(FrameInfo {
            frame_id,
            full_state: Arc::new(full_state),
            appended,
            disposed: Arc::new(disposed),
        });
        STATE_INFO_CACHE
            .lock()
            .unwrap()
            .insert(frame_id, info.clone());

        Ok(info)
    } else {
        let info = vec![FrameInfo {
            frame_id,
            full_state: appended.clone(),
            appended,
            disposed,
        }];
        STATE_INFO_CACHE
            .lock()
            .unwrap()
            .insert(frame_id, info.clone());
        Ok(info)
    }
}

pub async fn get_room_frame_id(room_id: &RoomId, until_sn: Option<i64>) -> AppResult<i64> {
    data::room::get_room_frame_id(room_id, until_sn)
        .await?
        .ok_or(MatrixError::not_found("room frame is not found").into())
}

pub async fn get_pdu_frame_id(event_id: &EventId) -> AppResult<i64> {
    data::room::get_pdu_frame_id(event_id)
        .await?
        .ok_or(MatrixError::not_found("pdu frame is not found").into())
}
/// Returns (state_hash, already_existed)
pub async fn ensure_frame(room_id: &RoomId, hash_data: Vec<u8>) -> AppResult<i64> {
    Ok(data::room::ensure_state_frame(room_id, hash_data).await?)
}

pub async fn get_frame_id(room_id: &RoomId, hash_data: &[u8]) -> AppResult<i64> {
    Ok(data::room::get_state_frame_id(room_id, hash_data).await?)
}
