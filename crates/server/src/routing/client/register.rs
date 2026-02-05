use diesel::prelude::*;
use palpo_core::presence::PresenceState;
use salvo::oapi::extract::{JsonBody, QueryParam};
use salvo::prelude::*;

use crate::core::UnixMillis;
use crate::core::client::account::{LoginType, RegistrationKind};
use crate::core::client::register::*;
use crate::core::client::uiaa::{AuthFlow, AuthType, UiaaInfo};
use crate::core::events::GlobalAccountDataEventType;
use crate::core::events::push_rules::PushRulesEventContent;
use crate::core::identifiers::*;
use crate::core::push::Ruleset;
use crate::core::serde::JsonValue;
use crate::data::schema::*;
use crate::data::user::{NewDbPresence, NewDbProfile};
use crate::data::{connect, diesel_exists};
use crate::exts::*;
use crate::{
    AppError, AuthArgs, DEVICE_ID_LENGTH, EmptyResult, JsonResult, MatrixError,
    RANDOM_USER_ID_LENGTH, SESSION_ID_LENGTH, TOKEN_LENGTH, config, data, empty_ok, hoops,
    membership, room, utils,
};

pub fn public_router() -> Router {
    Router::with_path("register").push(
        Router::with_hoop(hoops::limit_rate)
            .push(Router::with_path("available").get(available))
            .post(register)
            .push(Router::with_path("m.login.registration_token/validity").get(validate_token)),
    )
}

pub fn authed_router() -> Router {
    Router::with_path("register")
        .push(Router::with_path("email/requestToken").post(token_via_email))
        .push(Router::with_path("msisdn/requestToken").post(token_via_msisdn))
}

/// `POST /_matrix/client/*/register`
///
/// Register an account on this homeserver.
/// `/v3/` ([spec])
///
/// [spec]: https://spec.matrix.org/latest/client-server-api/#post_matrixclientv3register
#[endpoint]
async fn register(
    aa: AuthArgs,
    body: JsonBody<RegisterReqBody>,
    req: &mut Request,
    depot: &mut Depot,
    _res: &mut Response,
) -> JsonResult<RegisterResBody> {
    let body = body.into_inner();
    // For complement test `TestRequestEncodingFails`.
    if body.is_default() {
        let payload = req.payload().await?;
        if let Err(e) = serde_json::from_slice::<JsonValue>(payload) {
            return Err(MatrixError::not_json(format!("invalid json data: {e}")).into());
        }
    }

    let conf = crate::config::get();
    if !conf.allow_registration && !aa.from_appservice && conf.registration_token.is_none() {
        return Err(MatrixError::forbidden("registration has been disabled", None).into());
    }

    let is_guest = body.kind == RegistrationKind::Guest;
    let user_id = match (&body.username, is_guest) {
        (Some(username), false) => {
            let proposed_user_id =
                UserId::parse_with_server_name(username.to_lowercase(), &conf.server_name)
                    .ok()
                    .filter(|user_id| {
                        !user_id.is_historical() && user_id.server_name() == conf.server_name
                    })
                    .ok_or(MatrixError::invalid_username("username is invalid"))?;
            proposed_user_id.validate_strict()?;
            if data::user::user_exists(&proposed_user_id)? {
                return Err(MatrixError::user_in_use("desired user id is already taken").into());
            }
            proposed_user_id
        }
        _ => loop {
            let proposed_user_id = UserId::parse_with_server_name(
                utils::random_string(RANDOM_USER_ID_LENGTH).to_lowercase(),
                &conf.server_name,
            )?;
            proposed_user_id.validate_strict()?;
            if !data::user::user_exists(&proposed_user_id)? {
                break proposed_user_id;
            }
        },
    };

    let mut appservice = None;
    if body.login_type == Some(LoginType::Appservice) {
        let token = aa.require_access_token()?;
        appservice = crate::appservices()
            .into_iter()
            .find(|appservice| appservice.as_token == token)
            .cloned();
        if appservice.is_none() {
            return Err(MatrixError::missing_token("missing appservice token").into());
        }
    } else if crate::appservice::is_exclusive_user_id(&user_id)? {
        return Err(MatrixError::exclusive("user id reserved by appservice").into());
    }

    // UIAA
    let mut uiaa_info = UiaaInfo {
        flows: vec![AuthFlow {
            stages: if conf.registration_token.is_some() {
                vec![AuthType::RegistrationToken]
            } else {
                vec![AuthType::Dummy]
            },
        }],
        completed: Vec::new(),
        params: Default::default(),
        session: None,
        auth_error: None,
    };

    if body.login_type != Some(LoginType::Appservice) && !is_guest {
        if let Some(auth) = &body.auth {
            let (authed, uiaa) = crate::uiaa::try_auth(
                &UserId::parse_with_server_name("", &conf.server_name)
                    .expect("we know this is valid"),
                &body.device_id.clone().unwrap_or_else(|| "".into()),
                auth,
                &uiaa_info,
            )?;
            if !authed {
                return Err(AppError::Uiaa(uiaa));
            }
        } else {
            uiaa_info.session = Some(utils::random_string(SESSION_ID_LENGTH));
            crate::uiaa::update_session(
                &UserId::parse_with_server_name("", &config::get().server_name)
                    .expect("we know this is valid"),
                &body.device_id.clone().unwrap_or_else(|| "".into()),
                uiaa_info.session.as_ref().expect("session is always set"),
                Some(&uiaa_info),
            )?;
            return Err(uiaa_info.into());
        }
    }

    let password = if is_guest {
        None
    } else {
        body.password.as_deref()
    };

    // Create user
    let db_user = crate::user::create_user(user_id.clone(), password)?;

    // Default to pretty display_name
    let display_name = user_id.localpart().to_owned();
    // // If enabled append lightning bolt to display name (default true)
    // if config::enable_lightning_bolt() {
    //     display_name.push_str(" ⚡️");
    // }
    diesel::insert_into(user_profiles::table)
        .values(NewDbProfile {
            user_id: user_id.clone(),
            room_id: None,
            display_name: Some(display_name.clone()),
            avatar_url: None,
            blurhash: None,
        })
        .execute(&mut connect()?)?;

    // Presence update
    crate::data::user::set_presence(
        NewDbPresence {
            user_id: user_id.clone(),
            stream_id: None,
            state: Some(PresenceState::Online.to_string()),
            status_msg: None,
            last_active_at: Some(UnixMillis::now()),
            last_federation_update_at: None,
            last_user_sync_at: None,
            currently_active: None,
            occur_sn: None,
        },
        true,
    )?;

    // Initial account data
    crate::data::user::set_data(
        &user_id,
        None,
        &GlobalAccountDataEventType::PushRules.to_string(),
        serde_json::to_value(PushRulesEventContent {
            global: Ruleset::server_default(&user_id),
        })
        .expect("to json always works"),
    )?;

    // Inhibit login does not work for guests
    if !is_guest && body.inhibit_login {
        return Ok(Json(RegisterResBody {
            access_token: None,
            user_id,
            device_id: None,
            refresh_token: None,
            expires_in: None,
        }));
    }

    // Generate new device id if the user didn't specify one
    let device_id = if is_guest {
        None
    } else {
        body.device_id.clone()
    }
    .unwrap_or_else(|| utils::random_string(DEVICE_ID_LENGTH).into());

    // Generate new token for the device
    let token = utils::random_string(TOKEN_LENGTH);

    // Create device for this account
    data::user::device::create_device(
        &user_id,
        &device_id,
        &token,
        body.initial_device_display_name.clone(),
        Some(req.remote_addr().to_string()),
    )?;

    // If this is the first real user, grant them admin privileges
    // Note: the server user, @palpo:servername, is generated first
    if !is_guest {
        // TODO: admin
        // if let Ok(admin_room) = crate::room::get_admin_room() {
        //     if crate::room::user::join_count(&admin_room)? == 1 {
        //         crate::admin::make_admin(&user_id).await?;
        //         warn!("Granting {} admin privileges as the first user", user_id);
        //     } else if body.login_type != Some(LoginType::Appservice) {
        //         info!("New user {} registered on this server.", user_id);
        //         let _ = crate::admin::send_message(RoomMessageEventContent::notice_plain(format!(
        //             "New user {user_id} registered on this server."
        //         )));
        //     }
        // }
    }

    let from_appservice = appservice.is_some();
    if !from_appservice
        && !conf.auto_join_rooms.is_empty()
        && (conf.allow_guests_auto_join_rooms || !is_guest)
    {
        for room in &conf.auto_join_rooms {
            let Ok(room_id) = room::alias::resolve(room).await else {
                error!(
                    "failed to resolve room alias to room ID when attempting to auto join \
					 {room}, skipping"
                );
                continue;
            };

            if !room::is_server_joined(&conf.server_name, &room_id)? {
                warn!("skipping room {room} to automatically join as we have never joined before.");
                continue;
            }

            if let Ok(room_server_name) = room.server_name() {
                match membership::join_room(
                    &db_user,
                    Some(&device_id),
                    &room_id,
                    Some("automatically joining this room upon registration".to_owned()),
                    &[conf.server_name.clone(), room_server_name.to_owned()],
                    None,
                    None,
                    Default::default(),
                )
                .await
                {
                    Err(e) => {
                        // don't return this error so we don't fail registrations
                        error!("Failed to automatically join room {room} for user {user_id}: {e}");
                    }
                    _ => {
                        info!("automatically joined room {room} for user {user_id}");
                    }
                }
            }
        }
    }

    Ok(Json(RegisterResBody {
        access_token: Some(token),
        user_id,
        device_id: Some(device_id),
        refresh_token: None,
        expires_in: None,
    }))
}

/// #GET /_matrix/client/r0/register/available
/// Checks if a username is valid and available on this server.
///
/// Conditions for returning true:
/// - The user id is not historical
/// - The server name of the user id matches this server
/// - No user or appservice on this server already claimed this username
///
/// Note: This will not reserve the username, so the username might become invalid when trying to
/// register
#[endpoint]
async fn available(username: QueryParam<String, true>) -> JsonResult<AvailableResBody> {
    let username = username.into_inner().to_lowercase();
    // Validate user id
    let server_name = &config::get().server_name;
    let user_id = UserId::parse_with_server_name(username, server_name)
        .ok()
        .filter(|user_id| !user_id.is_historical() && user_id.server_name() == server_name)
        .ok_or(MatrixError::invalid_username("Username is invalid."))?;

    // Check if username is creative enough
    let query = users::table.find(&user_id);
    if diesel_exists!(query, &mut connect()?)? {
        return Err(MatrixError::user_in_use("Desired user ID is already taken.").into());
    }

    // TODO add check for appservice namespaces

    // If no if check is true we have an username that's available to be used.
    Ok(Json(AvailableResBody::new(true)))
}

/// `GET /_matrix/client/*/register/m.login.registration_token/validity`
///
/// Checks to see if the given registration token is valid.
/// `/v1/` ([spec])
///
/// [spec]: https://spec.matrix.org/latest/client-server-api/#get_matrixclientv1registermloginregistration_tokenvalidity

// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: true,
//     authentication: None,
//     history: {
//         unstable =>
// "/_matrix/client/unstable/org.matrix.msc3231/register/org.matrix.msc3231.login.
// registration_token/validity",         1.2 =>
// "/_matrix/client/v1/register/m.login.registration_token/validity",     }
// };
#[endpoint]
async fn validate_token(_aa: AuthArgs, depot: &mut Depot) -> EmptyResult {
    let _authed = depot.authed_info()?;
    empty_ok()
}

// `POST /_matrix/client/*/register/email/requestToken`
/// Request a registration token with a 3rd party email.
///
/// `/v3/` ([spec])
///
/// [spec]: https://spec.matrix.org/latest/client-server-api/#post_matrixclientv3registeremailrequesttoken

#[endpoint]
async fn token_via_email(_aa: AuthArgs, depot: &mut Depot) -> EmptyResult {
    let _authed = depot.authed_info()?;
    empty_ok()
}

/// `POST /_matrix/client/*/register/msisdn/requestToken`
/// Request a registration token with a phone number.
///
/// `/v3/` ([spec])
///
/// [spec]: https://spec.matrix.org/latest/client-server-api/#post_matrixclientv3registermsisdnrequesttoken
// const METADATA: Metadata = metadata! {
//     method: POST,
//     rate_limited: false,
//     authentication: None,
//     history: {
//         1.0 => "/_matrix/client/r0/register/msisdn/requestToken",
//         1.1 => "/_matrix/client/v3/register/msisdn/requestToken",
//     }
// };
#[endpoint]
async fn token_via_msisdn(_aa: AuthArgs, depot: &mut Depot) -> EmptyResult {
    let _authed = depot.authed_info()?;
    empty_ok()
}
