use std::collections::BTreeMap;
use std::fmt::Debug;

use super::{GetUrlOrigin, minimum_valid_ts};
use crate::AppResult;
use crate::core::directory::QueryCriteria;
use crate::core::federation::directory::{
    RemoteServerKeysBatchReqBody, RemoteServerKeysReqArgs, remote_server_keys_batch_request,
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
        // The queried server is a body field, never the HTTP destination.
        let origin = notary.origin().await;
        let requested = server_keys.split_off(&batch);
        let requested_servers = requested
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        let request = remote_server_keys_batch_request(
            &origin,
            RemoteServerKeysBatchReqBody {
                server_keys: requested,
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
            .json::<serde_json::Value>()
            .await?;

        results.extend(super::trust::notary_documents(notary, response, &requested_servers).await?);
    }

    Ok(results)
}

pub async fn notary_request(
    notary: &ServerName,
    target: &ServerName,
) -> AppResult<impl Iterator<Item = ServerSigningKeys> + Clone + Debug + Send> {
    notary_request_at(notary, target, minimum_valid_ts()).await
}

pub(super) async fn notary_request_at(
    notary: &ServerName,
    target: &ServerName,
    minimum_valid_until_ts: crate::core::UnixMillis,
) -> AppResult<impl Iterator<Item = ServerSigningKeys> + Clone + Debug + Send> {
    let request = remote_server_keys_request(
        &notary.origin().await,
        RemoteServerKeysReqArgs {
            server_name: target.into(),
            minimum_valid_until_ts,
        },
    )?
    .into_inner();

    let response = crate::sending::send_federation_request(notary, request, None)
        .await?
        .json::<serde_json::Value>()
        .await?;

    let requested = std::collections::BTreeSet::from([target.to_owned()]);
    Ok(super::trust::notary_documents(notary, response, &requested)
        .await?
        .into_iter())
}

pub async fn server_request(target: &ServerName) -> AppResult<ServerSigningKeys> {
    // Refuse to act as a notary for IP-literal server names. The Matrix
    // `server_name` grammar permits IP literals, but allowing them lets an
    // unauthenticated caller use the notary endpoints to probe arbitrary
    // hosts (including the homeserver's internal network) by IP and port.
    // DNS-name targets that resolve to denylisted IPs are blocked at
    // connect time by the federation client's safe DNS resolver.
    if target.is_ip_literal() {
        return Err(MatrixError::forbidden(
            "federation requests to IP-literal server names are not allowed",
            None,
        )
        .into());
    }

    let request = server_keys_request(&target.origin().await)?.into_inner();
    let server_signing_key = crate::sending::send_federation_request(target, request, None)
        .await?
        .json::<serde_json::Value>()
        .await?;

    let keys = super::trust::origin(&server_signing_key, target)?;
    super::trust::persist(&server_signing_key, &keys, "direct", target).await?;
    Ok(keys)
}
