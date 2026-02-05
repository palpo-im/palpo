use salvo::oapi::extract::*;
use salvo::prelude::*;
use serde::{Deserialize, Serialize};

use crate::core::UnixMillis;
use crate::{AppResult, EmptyResult, JsonResult, MatrixError, data, empty_ok, json_ok};

/// Response for a single registration token
#[derive(Debug, Serialize, ToSchema)]
pub struct RegistrationTokenResponse {
    pub token: String,
    pub uses_allowed: Option<i64>,
    pub pending: i64,
    pub completed: i64,
    pub expiry_time: Option<i64>,
}

impl From<data::user::RegistrationTokenInfo> for RegistrationTokenResponse {
    fn from(info: data::user::RegistrationTokenInfo) -> Self {
        Self {
            token: info.token,
            uses_allowed: info.uses_allowed,
            pending: info.pending,
            completed: info.completed,
            expiry_time: info.expiry_time,
        }
    }
}

/// Response for listing registration tokens
#[derive(Debug, Serialize, ToSchema)]
pub struct ListRegistrationTokensResponse {
    pub registration_tokens: Vec<RegistrationTokenResponse>,
}

/// Request body for creating a new registration token
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateRegistrationTokenReqBody {
    /// The token string. If not provided, a random token will be generated.
    #[serde(default)]
    pub token: Option<String>,
    /// Length of the token to generate (only used if token is not provided).
    /// Default is 16. Must be between 1 and 64.
    #[serde(default)]
    pub length: Option<usize>,
    /// Maximum number of uses. Null means unlimited.
    #[serde(default)]
    pub uses_allowed: Option<i64>,
    /// Expiry time in milliseconds since epoch. Null means no expiry.
    #[serde(default)]
    pub expiry_time: Option<i64>,
}

/// Request body for updating a registration token
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateRegistrationTokenReqBody {
    /// New maximum number of uses. Null means unlimited.
    /// Omit to leave unchanged.
    #[serde(default)]
    pub uses_allowed: Option<Option<i64>>,
    /// New expiry time in milliseconds since epoch. Null means no expiry.
    /// Omit to leave unchanged.
    #[serde(default)]
    pub expiry_time: Option<Option<i64>>,
}

/// Query parameters for listing registration tokens
#[derive(Debug, Deserialize, ToParameters)]
pub struct ListRegistrationTokensQuery {
    /// Filter by validity. If true, only valid tokens are returned.
    /// If false, only invalid tokens are returned.
    /// If omitted, all tokens are returned.
    #[serde(default)]
    pub valid: Option<bool>,
}

pub fn router() -> Router {
    Router::with_path("v1").push(
        Router::with_path("registration_tokens")
            .get(list_registration_tokens)
            .push(Router::with_path("new").post(create_registration_token))
            .push(
                Router::with_path("{token}")
                    .get(get_registration_token)
                    .put(update_registration_token)
                    .delete(delete_registration_token),
            ),
    )
}

/// List all registration tokens
///
/// GET /_synapse/admin/v1/registration_tokens
///
/// Optional query parameter:
/// - valid: Filter by validity (true/false)
#[endpoint(operation_id = "list_registration_tokens")]
pub fn list_registration_tokens(
    query: ListRegistrationTokensQuery,
) -> JsonResult<ListRegistrationTokensResponse> {
    let tokens = data::user::list_registration_tokens(query.valid)?;
    let registration_tokens = tokens.into_iter().map(Into::into).collect();
    json_ok(ListRegistrationTokensResponse { registration_tokens })
}

/// Create a new registration token
///
/// POST /_synapse/admin/v1/registration_tokens/new
///
/// Request body (all fields optional):
/// - token: The token string (auto-generated if not provided)
/// - length: Length of auto-generated token (default 16, max 64)
/// - uses_allowed: Maximum number of uses (null for unlimited)
/// - expiry_time: Expiry timestamp in milliseconds (null for no expiry)
#[endpoint(operation_id = "create_registration_token")]
pub fn create_registration_token(
    body: JsonBody<CreateRegistrationTokenReqBody>,
) -> AppResult<Json<RegistrationTokenResponse>> {
    let body = body.into_inner();

    // Determine the token string
    let token = if let Some(token) = body.token {
        // Validate provided token
        if token.is_empty() || token.len() > 64 {
            return Err(MatrixError::invalid_param(
                "token must not be empty and must not be longer than 64 characters",
            )
            .into());
        }
        if !data::user::is_valid_token_chars(&token) {
            return Err(MatrixError::invalid_param(
                "token must consist only of characters matched by [A-Za-z0-9._~-]",
            )
            .into());
        }
        token
    } else {
        // Generate a random token
        let length = body.length.unwrap_or(16);
        if length == 0 || length > 64 {
            return Err(MatrixError::invalid_param(
                "length must be greater than zero and not greater than 64",
            )
            .into());
        }
        data::user::generate_token(length)
    };

    // Validate uses_allowed
    if let Some(uses) = body.uses_allowed {
        if uses < 0 {
            return Err(
                MatrixError::invalid_param("uses_allowed must be a non-negative integer or null")
                    .into(),
            );
        }
    }

    // Validate expiry_time
    if let Some(expiry) = body.expiry_time {
        let now = UnixMillis::now().get() as i64;
        if expiry < now {
            return Err(MatrixError::invalid_param("expiry_time must not be in the past").into());
        }
    }

    // Create the token
    let created = data::user::create_registration_token(&token, body.uses_allowed, body.expiry_time)?;
    if !created {
        return Err(MatrixError::invalid_param(format!("Token already exists: {}", token)).into());
    }

    Ok(Json(RegistrationTokenResponse {
        token,
        uses_allowed: body.uses_allowed,
        pending: 0,
        completed: 0,
        expiry_time: body.expiry_time,
    }))
}

/// Get a specific registration token
///
/// GET /_synapse/admin/v1/registration_tokens/{token}
#[endpoint(operation_id = "get_registration_token")]
pub fn get_registration_token(token: PathParam<String>) -> JsonResult<RegistrationTokenResponse> {
    let token = token.into_inner();
    let token_info = data::user::get_registration_token(&token)?
        .ok_or_else(|| MatrixError::not_found(format!("No such registration token: {}", token)))?;
    json_ok(token_info.into())
}

/// Update a registration token
///
/// PUT /_synapse/admin/v1/registration_tokens/{token}
///
/// Request body (all fields optional):
/// - uses_allowed: New maximum number of uses (null for unlimited)
/// - expiry_time: New expiry timestamp in milliseconds (null for no expiry)
#[endpoint(operation_id = "update_registration_token")]
pub fn update_registration_token(
    token: PathParam<String>,
    body: JsonBody<UpdateRegistrationTokenReqBody>,
) -> JsonResult<RegistrationTokenResponse> {
    let token = token.into_inner();
    let body = body.into_inner();

    // Validate uses_allowed if provided
    if let Some(Some(uses)) = body.uses_allowed {
        if uses < 0 {
            return Err(
                MatrixError::invalid_param("uses_allowed must be a non-negative integer or null")
                    .into(),
            );
        }
    }

    // Validate expiry_time if provided
    if let Some(Some(expiry)) = body.expiry_time {
        let now = UnixMillis::now().get() as i64;
        if expiry < now {
            return Err(MatrixError::invalid_param("expiry_time must not be in the past").into());
        }
    }

    // If nothing to update, just get the token info
    let token_info = if body.uses_allowed.is_none() && body.expiry_time.is_none() {
        data::user::get_registration_token(&token)?
    } else {
        data::user::update_registration_token(&token, body.uses_allowed, body.expiry_time)?
    };

    let token_info = token_info
        .ok_or_else(|| MatrixError::not_found(format!("No such registration token: {}", token)))?;

    json_ok(token_info.into())
}

/// Delete a registration token
///
/// DELETE /_synapse/admin/v1/registration_tokens/{token}
#[endpoint(operation_id = "delete_registration_token")]
pub fn delete_registration_token(token: PathParam<String>) -> EmptyResult {
    let token = token.into_inner();
    let deleted = data::user::delete_registration_token(&token)?;
    if !deleted {
        return Err(
            MatrixError::not_found(format!("No such registration token: {}", token)).into(),
        );
    }
    empty_ok()
}
