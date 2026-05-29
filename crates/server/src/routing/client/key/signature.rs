use std::collections::BTreeMap;

use salvo::oapi::extract::*;
use salvo::prelude::*;

use crate::core::client::key::{UploadSignaturesReqBody, UploadSignaturesResBody};
use crate::{AuthArgs, DepotExt, JsonResult, MatrixError, json_ok};

/// #POST /_matrix/client/r0/keys/signatures/upload
/// Uploads end-to-end key signatures from the sender user.
#[endpoint]
pub(super) async fn upload(
    _aa: AuthArgs,
    body: JsonBody<UploadSignaturesReqBody>,
    depot: &mut Depot,
) -> JsonResult<UploadSignaturesResBody> {
    let authed = depot.authed_info()?;
    let body = body.into_inner();

    for (user_id, keys) in &body.0 {
        for (key_id, key) in keys {
            let key = serde_json::to_value(key)
                .map_err(|_| MatrixError::invalid_param("Invalid key JSON"))?;

            for signature in key
                .get("signatures")
                .ok_or(MatrixError::invalid_param("Missing signatures field."))?
                .get(authed.user_id().to_string())
                .ok_or(MatrixError::invalid_param(
                    "Invalid user in signatures field.",
                ))?
                .as_object()
                .ok_or(MatrixError::invalid_param("Invalid signature."))?
                .clone()
                .into_iter()
            {
                // Signature validation?
                let signature = (
                    signature.0,
                    signature
                        .1
                        .as_str()
                        .ok_or(MatrixError::invalid_param("Invalid signature value."))?
                        .to_owned(),
                );
                crate::user::sign_key(user_id, key_id, signature, authed.user_id()).await?;
            }
        }
    }

    json_ok(UploadSignaturesResBody {
        failures: BTreeMap::new(), // TODO: integrate
    })
}
