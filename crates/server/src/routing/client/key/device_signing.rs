use salvo::prelude::*;

use crate::core::client::key::UploadSigningKeysReqBody;
use crate::core::client::uiaa::{AuthFlow, AuthType, UiaaInfo};
use crate::core::encryption::CrossSigningKey;
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
    let mut challenged_body = if let Ok(body) = &body {
        if signing_key_payload_missing(body) {
            if let Some(session) = body.auth.as_ref().and_then(|auth| auth.session()) {
                load_challenged_signing_key_payload(sender_id, authed.device_id(), session).await?
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };
    // When delegated auth (OIDC) is enabled, UIA is not used — the OIDC
    // token already proves the user's identity.  Pasion calls
    // `allow_cross_signing_reset` to set a time-limited bypass before
    // Element uploads new keys, so we also honour that flag.
    let uia_required = if config::get().enabled_delegated_auth().is_some() {
        false
    } else if let Ok(body) = &body {
        let exist_master_key = crate::user::key::get_master_key(sender_id).await?;
        let exist_self_signing_key = crate::user::key::get_self_signing_key(sender_id).await?;
        let exist_user_signing_key = crate::user::key::get_user_signing_key(sender_id).await?;
        if exist_master_key.is_none()
            && exist_self_signing_key.is_none()
            && exist_user_signing_key.is_none()
        {
            false
        } else if let Some(expires_ts) =
            data::user::key::get_cross_signing_replacement_allowed(sender_id).await?
        {
            let now_ms = crate::core::UnixMillis::now().get() as i64;
            // Bypass still valid — skip UIA
            now_ms >= expires_ts
        } else {
            let body = signing_key_payload_for_uia(body, challenged_body.as_ref());
            signing_key_payload_changed(
                body,
                exist_master_key.as_ref(),
                exist_self_signing_key.as_ref(),
                exist_user_signing_key.as_ref(),
            )
        }
    } else {
        true
    };

    if body.is_err()
        || body
            .as_ref()
            .is_ok_and(|body| uia_required && body.auth.is_none())
    {
        if let Ok(json) = serde_json::from_slice::<CanonicalJsonValue>(payload) {
            uiaa_info.session = Some(utils::random_string(SESSION_ID_LENGTH));
            crate::uiaa::create_session(sender_id, authed.device_id(), &uiaa_info, json).await?;
            return Err(uiaa_info.into());
        } else {
            return Err(MatrixError::not_json("no json body was sent when required").into());
        }
    };
    let mut body = body.expect("body should be ok");
    if uia_required {
        let Some(auth) = &body.auth else {
            return Err(MatrixError::not_json("auth is none should not happend").into());
        };

        if challenged_body.is_none()
            && let Some(session) = auth.session()
        {
            challenged_body =
                load_challenged_signing_key_payload(sender_id, authed.device_id(), session).await?;
        }

        let (authenticated, uiaa) =
            crate::uiaa::try_auth(sender_id, authed.device_id(), auth, &uiaa_info).await?;
        if !authenticated {
            return Err(uiaa.into());
        }
        restore_signing_key_payload(&mut body, challenged_body)?;
    }

    if !signing_key_payload_missing(&body) {
        if body.master_key.is_none()
            && (body.self_signing_key.is_some() || body.user_signing_key.is_some())
            && crate::user::key::get_master_key(sender_id).await?.is_none()
        {
            return Err(MatrixError::invalid_param("Missing master signing key.").into());
        }

        crate::user::add_cross_signing_key_updates(
            sender_id,
            body.master_key.as_ref(),
            body.self_signing_key.as_ref(),
            body.user_signing_key.as_ref(),
            true, // notify so that other users see the new keys
        )
        .await?;
    }
    empty_ok()
}

fn signing_key_payload_missing(body: &UploadSigningKeysReqBody) -> bool {
    body.master_key.is_none() && body.self_signing_key.is_none() && body.user_signing_key.is_none()
}

fn signing_key_payload_for_uia<'a>(
    body: &'a UploadSigningKeysReqBody,
    challenged_body: Option<&'a UploadSigningKeysReqBody>,
) -> &'a UploadSigningKeysReqBody {
    if signing_key_payload_missing(body) {
        challenged_body.unwrap_or(body)
    } else {
        body
    }
}

async fn load_challenged_signing_key_payload(
    sender_id: &crate::core::identifiers::UserId,
    device_id: &crate::core::identifiers::DeviceId,
    session: &str,
) -> crate::AppResult<Option<UploadSigningKeysReqBody>> {
    crate::uiaa::get_uiaa_request(sender_id, device_id, session)
        .await
        .map(|request| {
            serde_json::from_value::<UploadSigningKeysReqBody>(serde_json::to_value(request)?)
        })
        .transpose()
        .map_err(Into::into)
}

fn signing_key_changed(
    existing_key: Option<&CrossSigningKey>,
    provided_key: Option<&CrossSigningKey>,
) -> bool {
    provided_key.is_some_and(|provided_key| existing_key != Some(provided_key))
}

fn signing_key_payload_changed(
    body: &UploadSigningKeysReqBody,
    existing_master_key: Option<&CrossSigningKey>,
    existing_self_signing_key: Option<&CrossSigningKey>,
    existing_user_signing_key: Option<&CrossSigningKey>,
) -> bool {
    signing_key_changed(existing_master_key, body.master_key.as_ref())
        || signing_key_changed(existing_self_signing_key, body.self_signing_key.as_ref())
        || signing_key_changed(existing_user_signing_key, body.user_signing_key.as_ref())
}

fn restore_signing_key_payload(
    body: &mut UploadSigningKeysReqBody,
    challenged_body: Option<UploadSigningKeysReqBody>,
) -> Result<(), MatrixError> {
    if let Some(challenged_body) = challenged_body {
        *body = challenged_body;
    }

    if signing_key_payload_missing(body) {
        return Err(MatrixError::bad_json("missing signing key payload"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_upload_body() -> UploadSigningKeysReqBody {
        UploadSigningKeysReqBody {
            auth: None,
            master_key: None,
            self_signing_key: None,
            user_signing_key: None,
        }
    }

    fn upload_body_with_master_key(user_id: &str) -> UploadSigningKeysReqBody {
        serde_json::from_value(serde_json::json!({
            "master_key": {
                "user_id": user_id,
                "usage": ["master"],
                "keys": {
                    "ed25519:abc": "abc"
                }
            }
        }))
        .unwrap()
    }

    fn upload_body_with_self_signing_key(user_id: &str, key_id: &str) -> UploadSigningKeysReqBody {
        serde_json::from_value(serde_json::json!({
            "self_signing_key": {
                "user_id": user_id,
                "usage": ["self_signing"],
                "keys": {
                    key_id: "abc"
                }
            }
        }))
        .unwrap()
    }

    #[test]
    fn signing_key_payload_changed_ignores_omitted_existing_keys() {
        let master_body = upload_body_with_master_key("@alice:example.com");
        let self_body = upload_body_with_self_signing_key("@alice:example.com", "ed25519:self");
        let existing_master_key = master_body.master_key.as_ref().unwrap();
        let existing_self_signing_key = self_body.self_signing_key.as_ref().unwrap();

        assert!(!signing_key_payload_changed(
            &self_body,
            Some(existing_master_key),
            Some(existing_self_signing_key),
            None,
        ));
    }

    #[test]
    fn signing_key_payload_changed_detects_new_provided_key() {
        let self_body = upload_body_with_self_signing_key("@alice:example.com", "ed25519:self");

        assert!(signing_key_payload_changed(&self_body, None, None, None));
    }

    #[test]
    fn signing_key_payload_changed_detects_replacement_key() {
        let existing_self_body =
            upload_body_with_self_signing_key("@alice:example.com", "ed25519:self");
        let new_self_body = upload_body_with_self_signing_key("@alice:example.com", "ed25519:new");

        assert!(signing_key_payload_changed(
            &new_self_body,
            None,
            existing_self_body.self_signing_key.as_ref(),
            None,
        ));
    }

    #[test]
    fn signing_key_payload_for_uia_uses_challenged_payload_for_auth_only_body() {
        let body = empty_upload_body();
        let challenged_body = upload_body_with_master_key("@alice:example.com");

        assert!(
            signing_key_payload_for_uia(&body, Some(&challenged_body))
                .master_key
                .is_some()
        );
    }

    #[test]
    fn signing_key_payload_for_uia_keeps_current_payload_when_keys_are_present() {
        let body = upload_body_with_self_signing_key("@alice:example.com", "ed25519:self");
        let challenged_body = upload_body_with_master_key("@alice:example.com");

        assert!(
            signing_key_payload_for_uia(&body, Some(&challenged_body))
                .self_signing_key
                .is_some()
        );
    }

    #[test]
    fn restore_signing_key_payload_restores_challenged_payload() {
        let mut body = empty_upload_body();

        restore_signing_key_payload(
            &mut body,
            Some(upload_body_with_master_key("@alice:example.com")),
        )
        .unwrap();

        assert!(body.master_key.is_some());
    }

    #[test]
    fn restore_signing_key_payload_prefers_challenged_payload() {
        let mut body = upload_body_with_master_key("@bob:example.com");

        restore_signing_key_payload(
            &mut body,
            Some(upload_body_with_master_key("@alice:example.com")),
        )
        .unwrap();

        assert_eq!(
            body.master_key.as_ref().unwrap().user_id.as_str(),
            "@alice:example.com"
        );
    }

    #[test]
    fn restore_signing_key_payload_rejects_missing_payload() {
        let mut body = empty_upload_body();

        assert!(restore_signing_key_payload(&mut body, None).is_err());
        assert!(restore_signing_key_payload(&mut body, Some(empty_upload_body())).is_err());
    }
}
