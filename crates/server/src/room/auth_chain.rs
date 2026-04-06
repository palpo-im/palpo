use std::collections::{BTreeSet, HashSet, VecDeque};
use std::fmt::Debug;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Instant;

use diesel::prelude::*;
use lru_cache::LruCache;

use crate::core::Seqnum;
use crate::core::identifiers::*;
use crate::data::connect;
use crate::data::schema::*;
use crate::room::timeline;
use crate::{AppResult, MatrixError};

type Bucket<'a> = BTreeSet<(Seqnum, &'a EventId)>;
static AUTH_CHAIN_CACHE: LazyLock<Mutex<LruCache<Vec<i64>, Arc<Vec<Seqnum>>>>> =
    LazyLock::new(|| Mutex::new(LruCache::new(100_000)));

pub fn get_auth_chain_ids<'a, I>(
    room_id: &'a RoomId,
    starting_event_ids: I,
) -> AppResult<Vec<OwnedEventId>>
where
    I: Iterator<Item = &'a EventId> + Clone + Debug + Send,
{
    let chain_sns = get_auth_chain_sns(room_id, starting_event_ids)?;

    let full_auth_chain = events::table
        .filter(events::sn.eq_any(&chain_sns))
        .order_by(events::sn.asc())
        .select(events::id)
        .load::<OwnedEventId>(&mut connect()?)?;
    Ok(full_auth_chain)
}
pub fn get_auth_chain_sns<'a, I>(
    room_id: &'a RoomId,
    starting_event_ids: I,
) -> AppResult<Vec<Seqnum>>
where
    I: Iterator<Item = &'a EventId> + Clone + Debug + Send,
{
    const NUM_BUCKETS: usize = 50; //TODO: change possible w/o disrupting db?
    const BUCKET: Bucket<'_> = BTreeSet::new();

    let started = Instant::now();
    let starting_events = events::table
        .filter(events::id.eq_any(starting_event_ids.clone()))
        .filter(events::sn.is_not_null())
        .select((events::id, events::sn))
        .load::<(OwnedEventId, Seqnum)>(&mut connect()?)?
        .into_iter()
        .collect::<Vec<_>>();

    let mut buckets = [BUCKET; NUM_BUCKETS];
    for (event_id, event_sn) in &starting_events {
        let index = event_sn % NUM_BUCKETS as i64;
        buckets[index as usize].insert((*event_sn, event_id));
    }

    debug!(
        starting_events = ?starting_events.len(),
        elapsed = ?started.elapsed(),
        "start",
    );

    let mut full_auth_chain: Vec<Seqnum> = Vec::with_capacity(starting_events.len());
    for bucket in buckets {
        let bucket_key: Vec<Seqnum> = bucket.iter().map(|i| i.0).collect();

        if bucket_key.is_empty() {
            continue;
        }

        if let Ok(Some(cached)) = get_cached_auth_chain(&bucket_key) {
            full_auth_chain.extend(cached.to_vec());
            continue;
        }

        let mut bucket_cache: Vec<_> = vec![];
        for (event_sn, event_id) in bucket {
            if let Ok(Some(cached)) = get_cached_auth_chain(&[event_sn]) {
                bucket_cache.extend(cached.to_vec());
                continue;
            }

            let auth_chain = get_event_auth_chain(room_id, event_id)?;
            let _ = cache_auth_chain(vec![event_sn], auth_chain.as_slice());
            bucket_cache.extend(auth_chain);
            debug!(
                ?event_id,
                elapsed = ?started.elapsed(),
                "Cache missed event"
            );
        }

        let _ = cache_auth_chain(bucket_key, bucket_cache.as_slice());
        debug!(
            bucket_cache_length = ?bucket_cache.len(),
            elapsed = ?started.elapsed(),
            "Cache missed bucket",
        );

        full_auth_chain.extend(bucket_cache);
    }
    full_auth_chain.sort_unstable();
    full_auth_chain.dedup();
    debug!(
        chain_length = ?full_auth_chain.len(),
        elapsed = ?started.elapsed(),
        "done",
    );

    Ok(full_auth_chain)
}

#[tracing::instrument(level = "trace", skip(room_id))]
fn get_event_auth_chain(room_id: &RoomId, event_id: &EventId) -> AppResult<Vec<Seqnum>> {
    let mut todo: VecDeque<_> = [event_id.to_owned()].into();
    let mut found = HashSet::new();

    while let Some(event_id) = todo.pop_front() {
        trace!(?event_id, "processing auth event");

        let pdu = timeline::get_pdu(&event_id)?;
        if pdu.room_id != room_id {
            tracing::error!(
                ?event_id,
                ?room_id,
                wrong_room_id = ?pdu.room_id,
                "auth event for incorrect room"
            );
            return Err(MatrixError::forbidden("auth event for incorrect room", None).into());
        }

        let auth_events = events::table
            .filter(events::sn.is_not_null())
            .filter(events::id.eq_any(pdu.auth_events.iter().map(|e| &**e)))
            .select((events::id, events::sn))
            .load::<(OwnedEventId, Seqnum)>(&mut connect()?)?;
        for (auth_event_id, auth_event_sn) in auth_events {
            if found.insert(auth_event_sn) {
                tracing::trace!(
                    ?auth_event_id,
                    ?auth_event_sn,
                    "adding auth event to processing queue"
                );

                todo.push_back(auth_event_id);
            }
        }
    }

    Ok(found.into_iter().collect())
}

fn get_cached_auth_chain(cache_key: &[Seqnum]) -> AppResult<Option<Arc<Vec<Seqnum>>>> {
    // Check RAM cache
    if let Some(result) = AUTH_CHAIN_CACHE.lock().unwrap_or_else(|e| e.into_inner()).get_mut(cache_key) {
        return Ok(Some(Arc::clone(result)));
    }

    let chain_sns = event_auth_chains::table
        .find(cache_key)
        .select(event_auth_chains::chain_sns)
        .first::<Vec<Option<Seqnum>>>(&mut connect()?)
        .optional()?;

    if let Some(chain_sns) = chain_sns {
        let chain_sns: Arc<Vec<Seqnum>> = Arc::new(chain_sns.into_iter().flatten().collect());
        // Cache in RAM
        AUTH_CHAIN_CACHE
            .lock()
            .unwrap()
            .insert(cache_key.to_owned(), chain_sns.clone());

        return Ok(Some(chain_sns));
    }

    Ok(None)
}

pub fn cache_auth_chain(cache_key: Vec<Seqnum>, chain_sns: &[Seqnum]) -> AppResult<()> {
    diesel::insert_into(event_auth_chains::table)
        .values((
            event_auth_chains::cache_key.eq(&cache_key),
            event_auth_chains::chain_sns.eq(chain_sns),
        ))
        .on_conflict(event_auth_chains::cache_key)
        .do_update()
        .set(event_auth_chains::chain_sns.eq(chain_sns))
        .execute(&mut connect()?)
        .ok();

    let chain_sns = chain_sns.to_vec();
    // Cache in RAM
    AUTH_CHAIN_CACHE
        .lock()
        .unwrap()
        .insert(cache_key, Arc::new(chain_sns));

    Ok(())
}
