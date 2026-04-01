use std::borrow::Borrow;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use futures_util::StreamExt;
use futures_util::stream::FuturesUnordered;
use tokio::time::{Instant, timeout_at};

use super::{
    add_signing_keys, batch_notary_request, key_exists, server_request, verify_keys_for,
};
use crate::config;
use crate::core::federation::discovery::ServerSigningKeys;
use crate::core::serde::{CanonicalJsonObject, RawJson, RawJsonValue};
use crate::core::{OwnedServerName, OwnedServerSigningKeyId, ServerName, ServerSigningKeyId};

type Batch = BTreeMap<OwnedServerName, Vec<OwnedServerSigningKeyId>>;

pub async fn acquire_events_pubkeys<'a, I>(events: I)
where
    I: Iterator<Item = &'a Box<RawJsonValue>> + Send,
{
    type Batch = BTreeMap<OwnedServerName, BTreeSet<OwnedServerSigningKeyId>>;
    type Signatures = BTreeMap<OwnedServerName, BTreeMap<OwnedServerSigningKeyId, String>>;

    let mut batch = Batch::new();
    events
        .cloned()
        .map(RawJson::<CanonicalJsonObject>::from_raw_value)
        .map(|event| event.get_field::<Signatures>("signatures"))
        .filter_map(|v| v.ok().flatten())
        .flat_map(IntoIterator::into_iter)
        .for_each(|(server, sigs)| {
            batch.entry(server).or_default().extend(sigs.into_keys());
        });

    let batch = batch
        .iter()
        .map(|(server, keys)| (server.borrow(), keys.iter().map(Borrow::borrow)));

    acquire_pubkeys(batch).await;
}

pub async fn acquire_pubkeys<'a, S, K>(batch: S)
where
    S: Iterator<Item = (&'a ServerName, K)> + Send + Clone,
    K: Iterator<Item = &'a ServerSigningKeyId> + Send + Clone,
{
    let notary_only = crate::config::get().only_query_trusted_key_servers;
    let notary_first_always = crate::config::get().query_trusted_key_servers_first;
    let notary_first_on_join = crate::config::get().query_trusted_key_servers_first_on_join;

    let requested_servers = batch.clone().count();
    let requested_keys = batch.clone().flat_map(|(_, key_ids)| key_ids).count();

    debug!("acquire {requested_keys} keys from {requested_servers}");

    let mut missing = acquire_locals(batch).await;
    let mut missing_keys = keys_count(&missing);
    let mut missing_servers = missing.len();
    if missing_servers == 0 {
        return;
    }

    info!("{missing_keys} keys for {missing_servers} servers will be acquired");

    if notary_first_always || notary_first_on_join {
        missing = acquire_notary(missing.into_iter()).await;
        missing_keys = keys_count(&missing);
        missing_servers = missing.len();
        if missing_keys == 0 {
            return;
        }

        warn!("missing {missing_keys} keys for {missing_servers} servers from all notaries first");
    }

    if !notary_only {
        missing = acquire_origins(missing.into_iter()).await;
        missing_keys = keys_count(&missing);
        missing_servers = missing.len();
        if missing_keys == 0 {
            return;
        }

        tracing::warn!("missing {missing_keys} keys for {missing_servers} servers unreachable");
    }

    if !notary_first_always && !notary_first_on_join {
        missing = acquire_notary(missing.into_iter()).await;
        missing_keys = keys_count(&missing);
        missing_servers = missing.len();
        if missing_keys == 0 {
            return;
        }

        tracing::warn!(
            "still missing {missing_keys} keys for {missing_servers} servers from all notaries."
        );
    }

    if missing_keys > 0 {
        warn!(
            "did not obtain {missing_keys} keys for {missing_servers} servers out of {requested_keys} total keys for \
			 {requested_servers} total servers."
        );
    }

    for (server, key_ids) in missing {
        tracing::warn!(?server, ?key_ids, "missing");
    }
}

async fn acquire_locals<'a, S, K>(batch: S) -> Batch
where
    S: Iterator<Item = (&'a ServerName, K)> + Send,
    K: Iterator<Item = &'a ServerSigningKeyId> + Send,
{
    let mut missing = Batch::new();
    for (server, key_ids) in batch {
        let available_keys = verify_keys_for(server);
        let missing_key_ids = missing_local_key_ids(&available_keys, key_ids);
        if !missing_key_ids.is_empty() {
            missing.insert(server.into(), missing_key_ids);
        }
    }

    missing
}

fn missing_local_key_ids<'a, K>(
    available_keys: &super::VerifyKeys,
    key_ids: K,
) -> Vec<OwnedServerSigningKeyId>
where
    K: Iterator<Item = &'a ServerSigningKeyId>,
{
    key_ids
        .filter(|key_id| !available_keys.contains_key(*key_id))
        .map(Into::into)
        .collect()
}

async fn acquire_origins<I>(batch: I) -> Batch
where
    I: Iterator<Item = (OwnedServerName, Vec<OwnedServerSigningKeyId>)> + Send,
{
    let timeout = Instant::now()
        .checked_add(Duration::from_secs(45))
        .expect("timeout overflows");

    let mut requests: FuturesUnordered<_> = batch
        .map(|(origin, key_ids)| acquire_origin(origin, key_ids, timeout))
        .collect();

    let mut missing = Batch::new();
    while let Some((origin, key_ids)) = requests.next().await {
        if !key_ids.is_empty() {
            missing.insert(origin, key_ids);
        }
    }

    missing
}

async fn acquire_origin(
    origin: OwnedServerName,
    mut key_ids: Vec<OwnedServerSigningKeyId>,
    timeout: Instant,
) -> (OwnedServerName, Vec<OwnedServerSigningKeyId>) {
    match timeout_at(timeout, server_request(&origin)).await {
        Err(e) => tracing::warn!(?origin, "timed out: {e}"),
        Ok(Err(e)) => tracing::error!(?origin, "{e}"),
        Ok(Ok(server_keys)) => {
            trace!(
                %origin,
                ?key_ids,
                ?server_keys,
                "received server_keys"
            );

            let _ = add_signing_keys(server_keys.clone());
            key_ids.retain(|key_id| !key_exists(&server_keys, key_id));
        }
    }

    (origin, key_ids)
}

async fn acquire_notary<I>(batch: I) -> Batch
where
    I: Iterator<Item = (OwnedServerName, Vec<OwnedServerSigningKeyId>)> + Send,
{
    let mut missing: Batch = batch.collect();
    let conf = config::get();
    for notary in &conf.trusted_servers {
        let missing_keys = keys_count(&missing);
        let missing_servers = missing.len();
        debug!(
            "Asking notary {notary} for {missing_keys} missing keys from {missing_servers} servers"
        );

        let batch = missing
            .iter()
            .map(|(server, keys)| (server.borrow(), keys.iter().map(Borrow::borrow)));

        match batch_notary_request(notary, batch).await {
            Err(e) => error!("failed to contact notary {notary:?}: {e}"),
            Ok(results) => {
                for server_keys in results {
                    acquire_notary_result(&mut missing, server_keys).await;
                }
            }
        }
    }

    missing
}

async fn acquire_notary_result(missing: &mut Batch, server_keys: ServerSigningKeys) {
    let server = &server_keys.server_name;
    let _ = add_signing_keys(server_keys.clone());

    if let Some(key_ids) = missing.get_mut(server) {
        key_ids.retain(|key_id| key_exists(&server_keys, key_id));
        if key_ids.is_empty() {
            missing.remove(server);
        }
    }
}

fn keys_count(batch: &Batch) -> usize {
    batch.iter().flat_map(|(_, key_ids)| key_ids.iter()).count()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::missing_local_key_ids;
    use crate::core::federation::discovery::VerifyKey;
    use crate::core::serde::Base64;
    use crate::core::OwnedServerSigningKeyId;

    #[test]
    fn acquire_locals_treats_our_configured_signing_key_as_available() {
        let local_key_id: OwnedServerSigningKeyId =
            "ed25519:local"
                .try_into()
                .expect("configured key id should be valid");
        let available_keys = BTreeMap::from([(
            local_key_id.clone(),
            VerifyKey {
                key: Base64::new(vec![1, 2, 3]),
            },
        )]);

        let missing = missing_local_key_ids(
            &available_keys,
            std::iter::once(local_key_id.as_ref()),
        );

        assert!(
            missing.is_empty(),
            "the local configured signing key should not be marked missing"
        );
    }
}
