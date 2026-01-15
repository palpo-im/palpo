mod password;
mod threepid;

use salvo::oapi::extract::*;
use salvo::prelude::*;

use crate::core::client::account::{
    DeactivateReqBody, DeactivateResBody, ThirdPartyIdRemovalStatus, WhoamiResBody,
};
use crate::core::client::uiaa::{AuthFlow, AuthType, UiaaInfo};
use crate::exts::*;
use crate::{
    AuthArgs, EmptyResult, JsonResult, MatrixError, SESSION_ID_LENGTH, data, hoops, json_ok, utils,
};

pub fn public_router() -> Router {
    Router::with_path("account")
        // .push(
        //     Router::with_path("3pid")
        //         .get(get_3pid)
        //         .push(Router::with_path("add").post(add_3pid))
        //         .push(Router::with_path("bind").post(bind_3pid))
        //         .push(Router::with_path("unbind").post(unbind_3pid))
        //         .push(Router::with_path("delete").post(delete_3pid))
        //         .push(Router::with_path("msisdn/requestToken").post(msisdn_request_token))
        //         .push(Router::with_path("email/requestTZoken").post(email_request_token)),
        // )
        .push(Router::with_path("email/requestToken").post(token_via_email))
        .push(Router::with_path("msisdn/requestToken").post(token_via_msisdn))
}
pub fn authed_router() -> Router {
    Router::with_path("account")
        // .push(
        //     Router::with_path("3pid")
        //         .get(get_3pid)
        //         .push(Router::with_path("add").post(add_3pid))
        //         .push(Router::with_path("bind").post(bind_3pid))
        //         .push(Router::with_path("unbind").post(unbind_3pid))
        //         .push(Router::with_path("delete").post(delete_3pid))
        //         .push(Router::with_path("msisdn/requestToken").post(msisdn_request_token))
        //         .push(Router::with_path("email/requestTZoken").post(email_request_token)),
        // )
        .push(
            Router::with_path("whoami")
                .hoop(hoops::limit_rate)
                .get(whoami),
        )
        .push(
            Router::with_path("deactivate")
                .hoop(hoops::limit_rate)
                .post(deactivate),
        )
        .push(password::authed_router())
        .push(threepid::authed_router())
}

/// #POST /_matrix/client/v3/account/3pid/email/requestToken
/// "This API should be used to request validation tokens when adding an email address to an
/// account"
///
/// - 403 signals that The homeserver does not allow the third party identifier as a contact option.
#[endpoint]
async fn token_via_email(_aa: AuthArgs) -> EmptyResult {
    Err(MatrixError::threepid_denied("Third party identifier is not allowed").into())
}

/// #POST /_matrix/client/v3/account/3pid/msisdn/requestToken
/// "This API should be used to request validation tokens when adding an phone number to an account"
///
/// - 403 signals that The homeserver does not allow the third party identifier as a contact option.
#[endpoint]
async fn token_via_msisdn(_aa: AuthArgs) -> EmptyResult {
    Err(MatrixError::threepid_denied("Third party identifier is not allowed").into())
}

/// #GET _matrix/client/r0/account/whoami
///
/// Get user_id of the sender user.
///
/// Note: Also works for Application Services
#[endpoint]
async fn whoami(_aa: AuthArgs, depot: &mut Depot) -> JsonResult<WhoamiResBody> {
    let authed = depot.take_authed_info()?;

    json_ok(WhoamiResBody {
        user_id: authed.user_id().to_owned(),
        device_id: Some(authed.device_id().to_owned()),
        is_guest: false,
    })
}

/// #POST /_matrix/client/r0/account/deactivate
/// Deactivate sender user account.
///
/// - Leaves all rooms and rejects all invitations
/// - Invalidates all access tokens
/// - Deletes all device metadata (device id, device display name, last seen ip, last seen ts)
/// - Forgets all to-device events
/// - Triggers device list updates
/// - Removes ability to log in again
#[endpoint]
async fn deactivate(
    _aa: AuthArgs,
    body: JsonBody<DeactivateReqBody>,
    depot: &mut Depot,
    res: &mut Response,
) -> JsonResult<DeactivateResBody> {
    let authed = depot.authed_info()?;

    let mut uiaa_info = UiaaInfo {
        flows: vec![AuthFlow {
            stages: vec![AuthType::Password],
        }],
        completed: Vec::new(),
        params: Default::default(),
        session: None,
        auth_error: None,
    };

    let Some(auth) = &body.auth else {
        uiaa_info.session = Some(utils::random_string(SESSION_ID_LENGTH));
        return Err(uiaa_info.into());
    };
    if crate::uiaa::try_auth(authed.user_id(), authed.device_id(), auth, &uiaa_info).is_err() {
        res.status_code(StatusCode::UNAUTHORIZED);
        return Err(MatrixError::forbidden("Authentication failed.", None).into());
    }

    // Remove devices and mark account as deactivated
    data::user::deactivate(authed.user_id())?;

    // info!("User {} deactivated their account.", authed.user_id());
    // crate::admin::send_message(RoomMessageEventContent::notice_plain(format!(
    //     "User {authed.user_id()} deactivated their account."
    // )));

    json_ok(DeactivateResBody {
        id_server_unbind_result: ThirdPartyIdRemovalStatus::NoSupport,
    })
}

// msc3391
#[handler]
pub(super) fn delete_account_data_msc3391(
    _req: &mut Request,
    _res: &mut Response,
) -> JsonResult<()> {
    json_ok(())
}
