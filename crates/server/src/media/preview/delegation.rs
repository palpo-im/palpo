//! Explicitly trusted MSC4417 providers; no delegated response enters the public cache.
use reqwest::dns::Resolve;
use serde_json::{Value, json};

use super::*;
use crate::core::appservice::media::{PreviewUrlReqArgs, preview_url_request};

const PROVIDER_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_RESPONSE: usize = 256_000;

fn provider_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        // AS addresses are operator-configured and may be internal. No redirect
        // may forward their homeserver token or a user's private preview URL.
        crate::sending::reqwest_client_builder(config::get())
            .expect("application-service preview transport configuration")
            .redirect(reqwest::redirect::Policy::none())
            .timeout(PROVIDER_TIMEOUT)
            .build()
            .expect("application-service preview client")
    })
}

async fn check_destination(url: &Url) -> AppResult<()> {
    url_guard::ensure_safe_outbound_url(url)?;
    if matches!(url.host(), Some(url::Host::Domain(_)))
        && !config::get().ip_range_denylist.is_empty()
    {
        let resolver =
            resolver::Resolver::new_with_cidr_denylist(Arc::new(RwLock::new(TlsNameMap::new())));
        let name = url
            .host_str()
            .unwrap()
            .parse()
            .map_err(|_| MatrixError::invalid_param("Invalid preview host"))?;
        let mut addresses = tokio::time::timeout(PROVIDER_TIMEOUT, resolver.resolve(name))
            .await
            .map_err(|_| MatrixError::forbidden("Preview destination resolution timed out", None))?
            .map_err(|_| {
                MatrixError::forbidden("Preview destination could not be resolved", None)
            })?;
        if addresses.next().is_none() {
            return Err(MatrixError::forbidden("Preview destination is denied", None).into());
        }
    }
    Ok(())
}

fn validate_preview(value: Value) -> AppResult<Option<Value>> {
    if value.is_null() {
        return Ok(None);
    }
    let invalid = || MatrixError::unknown("Invalid application-service preview response");
    let data = value.as_object().ok_or_else(invalid)?;
    if data.is_empty() {
        return Ok(None);
    }
    if data.iter().any(|(key, value)| {
        !(key.starts_with("og:") || key.starts_with("matrix:"))
            || value.is_array()
            || value.is_object()
    }) {
        return Err(invalid().into());
    }
    if let Some(image) = data.get("og:image").filter(|v| !v.is_null()) {
        let image = image.as_str().ok_or_else(invalid)?;
        if !OwnedMxcUri::from(image).is_valid() {
            return Err(invalid().into());
        }
    }
    for key in ["matrix:image:size", "og:image:width", "og:image:height"] {
        if data
            .get(key)
            .is_some_and(|v| !v.is_null() && v.as_u64().is_none())
        {
            return Err(invalid().into());
        }
    }
    for key in ["og:title", "og:type", "og:description", "og:url"] {
        if data
            .get(key)
            .is_some_and(|v| !v.is_null() && !v.is_string())
        {
            return Err(invalid().into());
        }
    }
    Ok(Some(value))
}

async fn request_provider(
    registration: &crate::core::appservice::Registration,
    url: &Url,
    user: Option<&UserId>,
) -> AppResult<Option<Value>> {
    let origin = registration
        .url
        .as_deref()
        .ok_or_else(|| MatrixError::unknown("Preview provider has no URL"))?;
    crate::appservice::validate_appservice_url(origin)?;
    let mut request = preview_url_request(
        origin,
        PreviewUrlReqArgs {
            url: url.as_str().into(),
            user_id: user.map(ToOwned::to_owned),
        },
    )?;
    request.headers_mut().insert(
        reqwest::header::AUTHORIZATION,
        format!("Bearer {}", registration.hs_token)
            .parse()
            .map_err(|_| MatrixError::unknown("Invalid preview provider credential"))?,
    );
    let response = provider_client()
        .execute(request.into_inner())
        .await
        .map_err(|_| MatrixError::unknown("Preview provider request failed"))?;
    if response.status() != reqwest::StatusCode::OK {
        return Err(MatrixError::unknown("Preview provider declined the request").into());
    }
    let body = utils::read_response_limited(
        response,
        config::get().url_preview.max_spider_size.min(MAX_RESPONSE),
    )
    .await?;
    let value = serde_json::from_slice(&body)
        .map_err(|_| MatrixError::unknown("Invalid preview provider JSON"))?;
    validate_preview(value)
}

pub(super) async fn preview(url: &Url, user: &UserId) -> AppResult<Option<Value>> {
    let registrations = crate::appservice::all().await?;
    let mut providers = Vec::new();
    for provider in &config::get().url_preview.appservice_providers {
        if let Some(info) = registrations.get(&provider.id) {
            // Preview patterns affect only explicitly enabled preview providers;
            // adding an unused namespace must not disable ordinary AS behavior.
            let namespace = match crate::appservice::NamespaceRegex::try_from(
                info.registration.namespaces.preview_urls.clone(),
            ) {
                Ok(namespace) => namespace,
                Err(_) => {
                    warn!(appservice_id = %provider.id, "Ignoring invalid application-service preview namespace");
                    continue;
                }
            };
            if namespace.is_match(url.as_str()) {
                providers.push((provider, info, namespace));
            }
        }
    }
    if providers.is_empty() {
        return Ok(None);
    }
    // Share the existing admission bound with local previews. Drop this lease
    // before falling back locally, so fallback never requires nested permits.
    // Results are not cached, so same-URL requests need no serialization queue.
    let _lease = PREVIEW_REQUESTS.try_enter(&format!("delegated:{user}:{url}"))?;
    check_destination(url).await?;
    for (provider, info, namespace) in providers {
        let exclusive = namespace.is_exclusive_match(url.as_str());
        let result = tokio::time::timeout(
            PROVIDER_TIMEOUT,
            request_provider(
                &info.registration,
                url,
                provider.send_user_id.then_some(user),
            ),
        )
        .await;
        match result {
            Ok(Ok(Some(value))) => return Ok(Some(value)),
            Ok(Ok(None)) if exclusive => return Ok(Some(json!({}))),
            Ok(Ok(None)) => {}
            _ if exclusive => {
                let mut error = MatrixError::unknown("Exclusive preview provider failed");
                error.status_code = Some(salvo::http::StatusCode::INTERNAL_SERVER_ERROR);
                return Err(error.into());
            }
            _ => {}
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests;
