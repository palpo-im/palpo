use std::collections::BTreeSet;

use salvo::oapi::extract::JsonBody;
use salvo::prelude::*;
use serde_json::Value;

use crate::core::client::profile::query::{QueryProfilesReqBody, QueryProfilesResBody};
use crate::{AuthArgs, JsonResult, MatrixError, config, data, json_ok};

pub(super) fn router() -> Router {
    Router::with_path("org.matrix.msc4536/profile/query").post(query_profiles)
}

/// Query known local profiles without scheduling federation requests (MSC4536).
#[endpoint(request_body = QueryProfilesReqBody)]
async fn query_profiles(_aa: AuthArgs, body: JsonBody<Value>) -> JsonResult<QueryProfilesResBody> {
    if !config::get().enable_batch_profile_queries {
        return Err(MatrixError::not_found("Batch profile queries are disabled").into());
    }
    let query = parse_query(body.into_inner())?;
    let user_ids: Vec<_> = query
        .users
        .into_iter()
        .filter(|id| id.server_name() == config::server_name())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let fields = query.fields.map(|fields| {
        fields
            .into_iter()
            .map(|f| f.to_string())
            .collect::<BTreeSet<_>>()
    });
    let mut profiles = QueryProfilesResBody::new();
    for profile in data::user::get_local_global_profiles(&user_ids).await? {
        let mut values = profile.fields.as_object().cloned().unwrap_or_default();
        // Reserved columns are authoritative, including when they are unset.
        for field in ["displayname", "avatar_url", "xyz.amorgan.blurhash"] {
            values.remove(field);
        }
        if let Some(value) = profile.display_name {
            values.insert("displayname".into(), Value::String(value));
        }
        if let Some(value) = profile.avatar_url {
            values.insert("avatar_url".into(), Value::String(value.to_string()));
        }
        if let Some(value) = profile.blurhash {
            values.insert("xyz.amorgan.blurhash".into(), Value::String(value));
        }
        if values.is_empty() {
            continue;
        }
        if let Some(fields) = &fields {
            values.retain(|field, _| fields.contains(field));
        }
        profiles.insert(profile.user_id, values);
    }
    // Current profile writes are bounded to 64 KiB per user. Also bound legacy data.
    if serde_json::to_vec(&profiles)?.len() > 8 * 1024 * 1024 {
        return Err(MatrixError::too_large("Batch profile response exceeds 8 MiB").into());
    }
    json_ok(profiles)
}

fn parse_query(body: Value) -> Result<QueryProfilesReqBody, MatrixError> {
    if !body.is_object() {
        return Err(MatrixError::bad_json("Expected an object"));
    }
    if body.get("users").is_none() {
        return Err(MatrixError::missing_param("Missing users"));
    }
    if body.get("fields").is_some_and(Value::is_null) {
        return Err(MatrixError::bad_json("fields must be a list"));
    }
    let query: QueryProfilesReqBody = serde_json::from_value(body)
        .map_err(|_| MatrixError::bad_json("Invalid batch profile request"))?;
    if query.users.is_empty() || query.users.len() > 100 {
        return Err(MatrixError::bad_json("users must contain 1 to 100 IDs"));
    }
    if let Some(fields) = &query.fields
        && (fields.is_empty()
            || fields.len() > 30
            || fields
                .iter()
                .any(|f| f.as_str().is_empty() || f.as_str().len() > 255))
    {
        return Err(MatrixError::bad_json(
            "fields must contain 1 to 30 names of 1 to 255 UTF-8 bytes",
        ));
    }
    Ok(query)
}

#[cfg(test)]
#[path = "profile_query_tests.rs"]
mod tests;
