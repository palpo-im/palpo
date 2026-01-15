use std::collections::BTreeMap;
use std::fmt::Debug;

use super::{GetUrlOrigin, minimum_valid_ts};
use crate::AppResult;
use crate::core::directory::QueryCriteria;
use crate::core::federation::directory::{
    RemoteServerKeysBatchReqBody, RemoteServerKeysBatchResBody, RemoteServerKeysReqArgs,
    RemoteServerKeysResBody, ServerKeysResBody, remote_server_keys_batch_request,
    remote_server_keys_request, server_keys_request,
};
use crate::core::federation::discovery::ServerSigningKeys;
use crate::core::{
    MatrixError, OwnedServerName, OwnedServerSigningKeyId, ServerName, ServerSigningKeyId,
};

type Batch = BTreeMap<OwnedServerName, BTreeMap<OwnedServerSigningKeyId, QueryCriteria>>;

pub(super) async fn batch_notary_request<'a, S, K>(
    notary: &ServerName,
    batch: S,
) -> AppResult<Vec<ServerSigningKeys>>
where
    S: Iterator<Item = (&'a ServerName, K)> + Send,
    K: Iterator<Item = &'a ServerSigningKeyId> + Send,
{
    let criteria = QueryCriteria {
        minimum_valid_until_ts: Some(minimum_valid_ts()),
    };

    let mut server_keys = batch.fold(Batch::new(), |mut batch, (server, key_ids)| {
        batch
            .entry(server.into())
            .or_default()
            .extend(key_ids.map(|key_id| (key_id.into(), criteria.clone())));

        batch
    });

    debug_assert!(!server_keys.is_empty(), "empty batch request to notary");

    let mut results = Vec::new();
    while let Some(batch) = server_keys
        .keys()
        .rev()
        .take(crate::config::get().trusted_server_batch_size)
        .next_back()
        .cloned()
    {
        let origin = batch.origin().await;
        let request = remote_server_keys_batch_request(
            &origin,
            RemoteServerKeysBatchReqBody {
                server_keys: server_keys.split_off(&batch),
            },
        )?
        .into_inner();

        debug!(
            ?notary,
            ?batch,
            remaining = %server_keys.len(),
            "notary request"
        );

        let response = crate::sending::send_federation_request(notary, request, None)
            .await?
            .json::<RemoteServerKeysBatchResBody>()
            .await?
            .server_keys;

        results.extend(response);
    }

    Ok(results)
}

pub async fn notary_request(
    notary: &ServerName,
    target: &ServerName,
) -> AppResult<impl Iterator<Item = ServerSigningKeys> + Clone + Debug + Send> {
    let request = remote_server_keys_request(
        &notary.origin().await,
        RemoteServerKeysReqArgs {
            server_name: target.into(),
            minimum_valid_until_ts: minimum_valid_ts(),
        },
    )?
    .into_inner();

    let response = crate::sending::send_federation_request(notary, request, None)
        .await?
        .json::<RemoteServerKeysResBody>()
        .await?
        .server_keys;

    Ok(response.into_iter())
}

pub async fn server_request(target: &ServerName) -> AppResult<ServerSigningKeys> {
    let request = server_keys_request(&target.origin().await)?.into_inner();
    let server_signing_key = crate::sending::send_federation_request(target, request, None)
        .await?
        .json::<ServerKeysResBody>()
        .await?
        .0;

    if server_signing_key.server_name != target {
        tracing::warn!(  requested = ?target,
            response = ?server_signing_key.server_name,
            "Server responded with bogus server_name");
        return Err(MatrixError::unknown("Server responded with bogus server_name").into());
    }

    Ok(server_signing_key)
}
