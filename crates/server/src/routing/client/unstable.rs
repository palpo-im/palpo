use salvo::prelude::*;

use crate::core::MatrixError;
use crate::{JsonResult, config, hoops, json_ok};

pub(super) fn router() -> Router {
    Router::with_path("unstable")
        // Public routes (no auth required) — MSC2965 OIDC discovery
        .push(
            Router::with_path("org.matrix.msc2965/auth_issuer").get(auth_issuer),
        )
        .push(
            Router::with_path("org.matrix.msc2965/auth_metadata").get(auth_metadata),
        )
        // Authed routes
        .push(
            Router::new()
                .hoop(hoops::limit_rate)
                .hoop(hoops::auth_by_access_token)
                .push(
                    Router::with_path(
                        "org.matrix.msc3391/user/{user_id}/account_data/{account_type}",
                    )
                    .delete(super::account::delete_account_data_msc3391),
                )
                .push(
                    Router::with_path("org.matrix.simplified_msc3575/sync")
                        .post(super::sync_msc4186::sync_events_v5),
                )
                .push(
                    Router::with_path("im.nheko.summary/rooms/{room_id_or_alias}/summary")
                        .get(super::room::summary::get_summary_msc_3266),
                )
                .push(
                    Router::with_path("uk.timedout.msc4323/admin/lock/{user_id}")
                        .get(super::admin::is_user_locked)
                        .put(super::admin::lock_user),
                )
                .push(
                    Router::with_path("uk.timedout.msc4323/admin/suspend/{user_id}")
                        .get(super::admin::is_user_suspended)
                        .put(super::admin::suspend_user),
                ),
        )
}

/// `GET /_matrix/client/unstable/org.matrix.msc2965/auth_issuer`
///
/// Returns the OIDC issuer that clients should use for authentication (MSC2965).
#[endpoint]
async fn auth_issuer(res: &mut Response) -> JsonResult<serde_json::Value> {
    res.headers_mut().insert(
        "Cache-Control",
        "public, max-age=600, s-maxage=3600, stale-while-revalidate=600"
            .parse()
            .unwrap(),
    );

    let conf = config::get();

    let issuer = if let Some(da) = conf.enabled_delegated_auth() {
        da.issuer.clone()
    } else if let Some(oidc) = conf.enabled_oidc() {
        oidc.mas_issuer.clone()
    } else {
        None
    };

    match issuer {
        Some(issuer) => json_ok(serde_json::json!({ "issuer": issuer })),
        None => Err(MatrixError::not_found(
            "OIDC discovery has not been configured on this homeserver.",
        )
        .into()),
    }
}

/// `GET /_matrix/client/unstable/org.matrix.msc2965/auth_metadata`
/// `GET /_matrix/client/v1/auth_metadata`
///
/// Returns the OAuth 2.0 authorization server metadata (MSC2965/MSC3861).
/// Fetches the metadata from the issuer's `/.well-known/openid-configuration` endpoint.
#[endpoint]
pub(super) async fn auth_metadata(res: &mut Response) -> JsonResult<serde_json::Value> {
    res.headers_mut().insert(
        "Cache-Control",
        "public, max-age=600, s-maxage=3600, stale-while-revalidate=600"
            .parse()
            .unwrap(),
    );

    let conf = config::get();

    let issuer = if let Some(da) = conf.enabled_delegated_auth() {
        da.issuer.clone()
    } else if let Some(oidc) = conf.enabled_oidc() {
        oidc.mas_issuer.clone()
    } else {
        None
    };

    let issuer = match issuer {
        Some(issuer) => issuer,
        None => {
            return Err(MatrixError::not_found(
                "OIDC discovery has not been configured on this homeserver.",
            )
            .into());
        }
    };

    let metadata_url = format!(
        "{}/.well-known/openid-configuration",
        issuer.trim_end_matches('/')
    );

    let client = reqwest::Client::new();
    let response = client.get(&metadata_url).send().await.map_err(|e| {
        tracing::error!("Failed to fetch OIDC metadata from {}: {}", metadata_url, e);
        MatrixError::unknown(format!("Failed to fetch OIDC metadata: {e}"))
    })?;

    if !response.status().is_success() {
        tracing::error!(
            "OIDC metadata endpoint {} returned status {}",
            metadata_url,
            response.status()
        );
        return Err(MatrixError::unknown("Failed to fetch OIDC metadata from issuer.").into());
    }

    let metadata: serde_json::Value = response.json().await.map_err(|e| {
        tracing::error!("Failed to parse OIDC metadata from {}: {}", metadata_url, e);
        MatrixError::unknown(format!("Failed to parse OIDC metadata: {e}"))
    })?;

    json_ok(metadata)
}
