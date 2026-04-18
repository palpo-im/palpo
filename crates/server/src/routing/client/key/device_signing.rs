use salvo::prelude::*;

use crate::core::client::key::UploadSigningKeysReqBody;
use crate::core::client::uiaa::{AuthFlow, AuthType, UiaaInfo};
use crate::core::serde::CanonicalJsonValue;
use crate::{
    AuthArgs, DepotExt, EmptyResult, MatrixError, SESSION_ID_LENGTH, config, data, empty_ok, utils,
};

/// #POST /_matrix/client/r0/keys/device_signing/upload
/// Uploads end-to-end key information for the sender user.
///
/// - Requires UIAA to verify password
#[endpoint]
pub(super) async fn upload(_aa: AuthArgs, req: &mut Request, depot: &mut Depot) -> EmptyResult {
    let authed = depot.authed_info()?;
    let sender_id = authed.user_id();

    let payload = req.payload().await?;
    // UIAA
    let mut uiaa_info = UiaaInfo {
        flows: vec![AuthFlow {
            stages: vec![AuthType::Password],
        }],
        completed: Vec::new(),
        params: Default::default(),
        session: None,
        auth_error: None,
    };
    let body = serde_json::from_slice::<UploadSigningKeysReqBody>(payload);
    let none_auth = body.as_ref().map(|b| b.auth.is_none()).unwrap_or(true);
    // When delegated auth (OIDC) is enabled, UIA is not used — the OIDC
    // token already proves the user's identity.  Pasion calls
    // `allow_cross_signing_reset` to set a time-limited bypass before
    // Element uploads new keys, so we also honour that flag.
    let uia_required = if config::get().enabled_delegated_auth().is_some() {
        false
    } else if none_auth {
        let exist_master_key = crate::user::key::get_master_key(sender_id)?;
        let exist_self_signing_key = crate::user::key::get_self_signing_key(sender_id)?;
        let exist_user_signing_key = crate::user::key::get_user_signing_key(sender_id)?;
        if exist_master_key.is_none()
            && exist_self_signing_key.is_none()
            && exist_user_signing_key.is_none()
        {
            false
        } else if let Some(expires_ts) =
            data::user::key::get_cross_signing_replacement_allowed(sender_id)?
        {
            let now_ms = crate::core::UnixMillis::now().get() as i64;
            // Bypass still valid — skip UIA
            now_ms >= expires_ts
        } else {
            exist_master_key.as_ref()
                != body.as_ref().map(|b| b.master_key.as_ref()).unwrap_or(None)
                || exist_self_signing_key.as_ref()
                    != body
                        .as_ref()
                        .map(|b| b.self_signing_key.as_ref())
                        .unwrap_or(None)
                || exist_user_signing_key.as_ref()
                    != body
                        .as_ref()
                        .map(|b| b.user_signing_key.as_ref())
                        .unwrap_or(None)
        }
    } else {
        false
    };

    if body.is_err() || uia_required {
        if let Ok(json) = serde_json::from_slice::<CanonicalJsonValue>(payload) {
            uiaa_info.session = Some(utils::random_string(SESSION_ID_LENGTH));
            crate::uiaa::create_session(sender_id, authed.device_id(), &uiaa_info, json)?;
            return Err(uiaa_info.into());
        } else {
            return Err(MatrixError::not_json("no json body was sent when required").into());
        }
    };
    let body = body.expect("body should be ok");
    if uia_required {
        let Some(auth) = &body.auth else {
            return Err(MatrixError::not_json("auth is none should not happend").into());
        };

        crate::uiaa::try_auth(sender_id, authed.device_id(), auth, &uiaa_info)?;
    }

    if let Some(master_key) = &body.master_key {
        crate::user::add_cross_signing_keys(
            sender_id,
            master_key,
            &body.self_signing_key,
            &body.user_signing_key,
            true, // notify so that other users see the new keys
        )?;
    }
    empty_ok()
}
