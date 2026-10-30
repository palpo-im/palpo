use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use palpo_core::presence::PresenceState;
use salvo::oapi::extract::{JsonBody, QueryParam};
use salvo::prelude::*;
use subtle::ConstantTimeEq;

use crate::core::UnixMillis;
use crate::core::client::account::{LoginType, RegistrationKind};
use crate::core::client::register::*;
use crate::core::client::uiaa::{AuthData, AuthFlow, AuthType, UiaaInfo};
use crate::core::events::GlobalAccountDataEventType;
use crate::core::events::push_rules::PushRulesEventContent;
use crate::core::identifiers::*;
use crate::core::push::Ruleset;
use crate::core::serde::JsonValue;
use crate::data::schema::*;
use crate::data::user::NewDbPresence;
use crate::data::{connect, diesel_exists};
use crate::exts::*;
use crate::{
    AppError, AuthArgs, DEVICE_ID_LENGTH, EmptyResult, JsonResult, MatrixError,
    RANDOM_USER_ID_LENGTH, TOKEN_LENGTH, config, data, empty_ok, hoops, membership, room, utils,
};

pub fn public_router() -> Router {
    Router::with_path("register")
        .push(Router::with_path("available").get(available))
        .push(
            Router::with_path("email/requestToken").post(crate::registration_email::request_token),
        )
        .push(Router::with_path("email/submitToken").post(crate::registration_email::submit_token))
        .post(register)
        .push(Router::with_path("m.login.registration_token/validity").get(validate_token))
}

pub fn authed_router() -> Router {
    Router::with_path("register")
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
    _depot: &mut Depot,
    _res: &mut Response,
) -> JsonResult<RegisterResBody> {
    hoops::check_registration_rate(req)?;
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

    if conf.enabled_delegated_auth().is_some()
        && body.login_type != Some(LoginType::ApplicationService)
    {
        return Err(MatrixError::forbidden(
            "Local registration is disabled while delegated authentication is enabled. Use SSO/OIDC registration.",
            None,
        )
        .into());
    }

    let is_guest = body.kind == RegistrationKind::Guest;
    let email_required =
        conf.registration_email.is_some() && body.login_type != Some(LoginType::ApplicationService);
    if email_required && is_guest {
        return Err(
            MatrixError::forbidden("This server requires a verified email account.", None).into(),
        );
    }
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
            if data::user::user_exists(&proposed_user_id).await? {
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
            if !data::user::user_exists(&proposed_user_id).await? {
                break proposed_user_id;
            }
        },
    };

    let mut appservice = None;
    if body.login_type == Some(LoginType::ApplicationService) {
        let token = aa.require_access_token()?;
        // Constant-time comparison so we don't leak `as_token` bytes via
        // response timing. The same pattern is used in `hoops/auth.rs`.
        let matched = crate::appservices()
            .await
            .iter()
            .find(|appservice| {
                appservice
                    .as_token
                    .as_bytes()
                    .ct_eq(token.as_bytes())
                    .into()
            })
            .cloned();
        let Some(matched) = matched else {
            return Err(MatrixError::missing_token("missing appservice token").into());
        };
        let appservice_info: crate::appservice::RegistrationInfo = matched.clone().try_into()?;
        if !appservice_info.is_user_match(&user_id) {
            return Err(MatrixError::exclusive("user is not in appservice namespace").into());
        }
        appservice = Some(matched);
    } else if crate::appservice::is_exclusive_user_id(&user_id).await? {
        return Err(MatrixError::exclusive("user id reserved by appservice").into());
    }

    // UIAA
    let mut uiaa_info = UiaaInfo {
        flows: vec![AuthFlow {
            stages: {
                let mut stages = Vec::new();
                if email_required {
                    stages.push(AuthType::EmailIdentity);
                }
                if conf.registration_token.is_some() {
                    stages.push(AuthType::RegistrationToken);
                }
                if stages.is_empty() {
                    stages.push(AuthType::Dummy);
                }
                stages
            },
        }],
        completed: Vec::new(),
        params: Default::default(),
        session: None,
        auth_error: None,
    };

    let mut completed_session = None;
    if body.login_type != Some(LoginType::ApplicationService) && !is_guest {
        let uiaa_user_id =
            UserId::parse_with_server_name("", &conf.server_name).expect("we know this is valid");
        let uiaa_device_id = body.device_id.clone().unwrap_or_else(|| "".into());
        let fingerprint = registration_attempt_fingerprint(&body, &user_id)?;
        let now = crate::registration_email::now();
        match body.auth.as_ref() {
            // Dummy-only registration needs no prior challenge: Matrix permits
            // omitting its session. Stronger configured flows still require the
            // issued session below, and a supplied session is always validated.
            Some(auth @ AuthData::Dummy(_))
                if auth.session().is_none()
                    && uiaa_info.flows.len() == 1
                    && uiaa_info.flows[0].stages == [AuthType::Dummy] =>
            {
                if let Some(challenge) = data::user::uiaa::registration_session(
                    &uiaa_user_id,
                    &uiaa_device_id,
                    &fingerprint,
                    now,
                )
                .await?
                .filter(dummy_only_registration)
                {
                    // A first Dummy request needs no session. Once challenged,
                    // this attempt must use the nonce the server already issued.
                    return Err(challenge.into());
                }
                let (authed, uiaa) =
                    crate::uiaa::try_auth(&uiaa_user_id, &uiaa_device_id, auth, &uiaa_info).await?;
                if !authed {
                    return Err(AppError::Uiaa(uiaa));
                }
                completed_session = uiaa.session;
            }
            Some(auth) if auth.session().is_some() => {
                // Only claim after checking that this server issued the UIAA session.
                let session = auth.session().expect("guard checked the session");
                let stored =
                    crate::uiaa::get_session(&uiaa_user_id, &uiaa_device_id, session).await?;
                validate_registration_session(
                    &uiaa_user_id,
                    &uiaa_device_id,
                    session,
                    &fingerprint,
                    now,
                    &stored,
                    &uiaa_info,
                )
                .await?;
                if let crate::core::client::uiaa::AuthData::EmailIdentity(email) = auth
                    && (!email_required
                        || email.thirdparty_id_creds.id_server.is_some()
                        || !data::user::registration_email::claim(
                            email.thirdparty_id_creds.sid.as_str(),
                            &crate::registration_email::secret_hash(
                                email.thirdparty_id_creds.client_secret.as_str(),
                            )?,
                            session,
                            &user_id,
                            crate::registration_email::now(),
                        )
                        .await?)
                {
                    return Err(MatrixError::forbidden(
                        "Verify your email before registering.",
                        None,
                    )
                    .into());
                }
                let (authed, uiaa) =
                    crate::uiaa::try_auth(&uiaa_user_id, &uiaa_device_id, auth, &uiaa_info).await?;
                if !authed {
                    return Err(AppError::Uiaa(uiaa));
                }
                completed_session = uiaa.session;
            }
            _ => {
                // Only Dummy-only challenges may be reused without a session.
                // Never disclose a partially completed token/email session.
                if dummy_only_registration(&uiaa_info)
                    && let Some(challenge) = data::user::uiaa::registration_session(
                        &uiaa_user_id,
                        &uiaa_device_id,
                        &fingerprint,
                        now,
                    )
                    .await?
                    .filter(dummy_only_registration)
                {
                    return Err(challenge.into());
                }
                uiaa_info.session = Some(utils::random_string(crate::SESSION_ID_LENGTH));
                let expires_at = now.saturating_add(conf.session_ttl.min(3_600_000) as i64);
                data::user::uiaa::create_registration_session(
                    &uiaa_user_id,
                    &uiaa_device_id,
                    &uiaa_info,
                    &fingerprint,
                    expires_at,
                )
                .await?;
                return Err(uiaa_info.into());
            }
        }
    }

    // Create user
    let db_user = if is_guest {
        crate::user::create_guest_user(user_id.clone()).await?
    } else {
        let email_session =
            if email_required {
                Some(completed_session.as_deref().ok_or_else(|| {
                    MatrixError::forbidden("Email verification is required.", None)
                })?)
            } else {
                None
            };
        let new_user = data::user::NewDbUser {
            id: user_id.clone(),
            ty: None,
            is_admin: false,
            is_guest: false,
            is_local: true,
            localpart: user_id.localpart().to_owned(),
            server_name: user_id.server_name().to_owned(),
            appservice_id: None,
            created_at: UnixMillis::now(),
        };
        let hash = body
            .password
            .as_deref()
            .map(crate::utils::hash_password)
            .transpose()?;
        data::user::registration_email::create_registered_user(
            &new_user,
            hash.as_deref(),
            email_session,
            crate::registration_email::now(),
        )
        .await?
    };

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
            updated_at: UnixMillis::now(),
        },
        true,
    )
    .await?;

    // Initial account data
    crate::data::user::set_data(
        &user_id,
        None,
        &GlobalAccountDataEventType::PushRules.to_string(),
        serde_json::to_value(PushRulesEventContent::new(Ruleset::server_default(
            &user_id,
        )))
        .expect("to json always works"),
    )
    .await?;

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
    )
    .await?;

    // If this is the first real user, grant them admin privileges
    // Note: the server user, @palpo:servername, is generated first
    if !is_guest {
        // TODO: admin
        // if let Ok(admin_room) = crate::room::get_admin_room() {
        //     if crate::room::user::join_count(&admin_room)? == 1 {
        //         crate::admin::make_admin(&user_id).await?;
        //         warn!("Granting {} admin privileges as the first user", user_id);
        //     } else if body.login_type != Some(LoginType::ApplicationService) {
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

            if !room::is_server_joined(&conf.server_name, &room_id).await? {
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

fn dummy_only_registration(info: &UiaaInfo) -> bool {
    info.flows.len() == 1 && info.flows[0].stages == [AuthType::Dummy]
}

/// Bind client-proposed identity/options, without retaining credentials or a fast
/// password verifier. Anonymous requests use None, not a freshly generated ID.
fn registration_attempt_fingerprint(
    body: &RegisterReqBody,
    proposed: &UserId,
) -> crate::AppResult<String> {
    let identity = serde_json::json!({
        "username": body.username.as_ref().map(|_| proposed.as_str()),
        "device_id": &body.device_id,
        "initial_device_display_name": &body.initial_device_display_name,
        "kind": &body.kind,
        "inhibit_login": body.inhibit_login,
        "type": &body.login_type,
        "refresh_token": body.refresh_token,
    });
    Ok(utils::hash_data_sha2_256(&serde_json::to_vec(&identity)?)?.to_hex_uppercase())
}

async fn validate_registration_session(
    user_id: &UserId,
    device_id: &DeviceId,
    session: &str,
    fingerprint: &str,
    now: i64,
    stored: &UiaaInfo,
    current: &UiaaInfo,
) -> crate::AppResult<()> {
    let request = data::user::uiaa::get_uiaa_request(user_id, device_id, session).await?;
    let binding = request
        .as_ref()
        .and_then(|value| value.as_object())
        .and_then(|body| body.get("palpo.registration"))
        .and_then(|value| value.as_object());
    let expires_at = binding
        .and_then(|binding| binding.get("expires_at"))
        .and_then(|value| value.as_integer());
    if expires_at.is_some_and(|expires| expires <= now) {
        crate::uiaa::update_session(user_id, device_id, session, None).await?;
        return Err(MatrixError::unauthorized("Registration session expired. Start again.").into());
    }
    let matching = binding
        .and_then(|binding| binding.get("fingerprint"))
        .and_then(|value| value.as_str())
        == Some(fingerprint);
    let matching_flows = stored
        .flows
        .iter()
        .map(|flow| &flow.stages)
        .eq(current.flows.iter().map(|flow| &flow.stages));
    if !matching || expires_at.is_none() || !matching_flows {
        return Err(
            MatrixError::unauthorized("Registration session does not match this request.").into(),
        );
    }
    Ok(())
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
async fn available(
    username: QueryParam<String, true>,
    req: &mut Request,
) -> JsonResult<AvailableResBody> {
    hoops::check_registration_available_rate(req)?;
    if config::get().enabled_delegated_auth().is_some() {
        return Err(MatrixError::forbidden(
            "Local registration is disabled while delegated authentication is enabled.",
            None,
        )
        .into());
    }

    let username = username.into_inner().to_lowercase();
    // Validate user id
    let server_name = &config::get().server_name;
    let user_id = UserId::parse_with_server_name(username, server_name)
        .ok()
        .filter(|user_id| !user_id.is_historical() && user_id.server_name() == server_name)
        .ok_or(MatrixError::invalid_username("Username is invalid."))?;

    // Check if username is creative enough
    let query = users::table.find(&user_id);
    if diesel_exists!(query, &mut connect().await?)? {
        return Err(MatrixError::user_in_use("Desired user ID is already taken.").into());
    }

    // Check if the username is reserved by an appservice exclusive namespace
    if crate::appservice::is_exclusive_user_id(&user_id).await? {
        return Err(
            MatrixError::exclusive("Username is reserved by an application service.").into(),
        );
    }

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
async fn validate_token(
    token: QueryParam<String, true>,
    req: &mut Request,
) -> JsonResult<ValidateTokenResBody> {
    hoops::check_registration_token_rate(req)?;
    let token = token.into_inner();
    let valid = config::get()
        .registration_token
        .as_deref()
        .is_some_and(|expected| {
            let supplied = token.trim().as_bytes();
            supplied.len() == expected.len() && supplied.ct_eq(expected.as_bytes()).into()
        });
    Ok(Json(ValidateTokenResBody { valid }))
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

#[cfg(test)]
mod dummy_registration_tests {
    use salvo::test::{ResponseExt, TestClient};
    use serde_json::{Value, json};

    use super::*;

    // Run each policy test in its own process and empty dedicated database:
    // the production configuration is immutable for the lifetime of a server.
    fn service(policy: &str) -> Service {
        crate::test_database::init();
        let mut conf = json!({
            "server_name": "dummy-registration.example", "db": {"url": "unused-test-config"},
            "allow_registration": policy != "closed", "auto_join_rooms": []
        });
        if policy == "token" {
            conf["allow_registration"] = json!(false);
            conf["registration_token"] = json!("required-registration-token");
        }
        if policy == "email" {
            conf["registration_email"] = json!({
                "agentmail_inbox": "registration@example.test",
                "agentmail_api_key_file": std::env::temp_dir().join("palpo-dummy-registration-unread-api-key"),
                "otp_secret_file": std::env::temp_dir().join("palpo-dummy-registration-unread-otp-key")
            });
        }
        crate::config::CONFIG
            .set(serde_json::from_value(conf).unwrap())
            .expect("run each registration policy test in its own process");
        Service::new(
            Router::new().push(public_router()).push(
                Router::new()
                    .hoop(hoops::auth_by_access_token)
                    .push(crate::routing::client::account::authed_router()),
            ),
        )
    }

    async fn register_request(service: &Service, body: Value) -> (StatusCode, Value) {
        let mut response = TestClient::post("http://localhost/register")
            .json(&body)
            .send(service)
            .await;
        (
            response.status_code.unwrap(),
            response.take_json::<Value>().await.unwrap(),
        )
    }

    async fn assert_no_account(localpart: &str) {
        let user = UserId::parse(format!("@{localpart}:dummy-registration.example")).unwrap();
        assert!(!data::user::user_exists(&user).await.unwrap());
    }

    async fn session_rows(session: &str) -> i64 {
        user_uiaa_datas::table
            .filter(user_uiaa_datas::session.eq(session))
            .count()
            .get_result(&mut connect().await.unwrap())
            .await
            .unwrap()
    }

    async fn expire_session(session: &str) {
        diesel::sql_query("UPDATE user_uiaa_datas SET request_body = jsonb_set(request_body, '{palpo.registration,expires_at}', '0'::jsonb) WHERE session = $1")
            .bind::<diesel::sql_types::Text,_>(session)
            .execute(&mut connect().await.unwrap()).await.unwrap();
    }

    async fn assert_registration_continuation(service: &Service) {
        let base = json!({"username":"bound-attempt", "password":"never-persist-this-password", "device_id":"BIND"});
        let (status, challenge) = register_request(service, base.clone()).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{challenge}");
        let session = challenge["session"].as_str().unwrap();
        assert_eq!(session_rows(session).await, 1);
        let stored: Value = user_uiaa_datas::table
            .filter(user_uiaa_datas::session.eq(session))
            .select(user_uiaa_datas::request_body)
            .first::<Option<Value>>(&mut connect().await.unwrap())
            .await
            .unwrap()
            .unwrap();
        assert!(!stored.to_string().contains("never-persist-this-password"));
        assert!(stored.get("password").is_none() && stored.get("auth").is_none());
        let mut omitted = base.clone();
        omitted["auth"] = json!({"type":"m.login.dummy"});
        let (status, retry) = register_request(service, omitted).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{retry}");
        assert_eq!(retry["session"], challenge["session"]);
        assert_no_account("bound-attempt").await;
        let (status, fresh) = register_request(service, json!({"username":"fresh-while-bound", "device_id":"BIND", "auth":{"type":"m.login.dummy"}})).await;
        assert_eq!(status, StatusCode::OK, "{fresh}");
        let (status, fresh) = register_request(service, json!({"username":"fresh-other-device", "device_id":"OTHER", "auth":{"type":"m.login.dummy"}})).await;
        assert_eq!(status, StatusCode::OK, "{fresh}");

        let mut wrong = base.clone();
        wrong["username"] = json!("wrong-attempt");
        wrong["auth"] = json!({"type":"m.login.dummy", "session":session});
        let (status, error) = register_request(service, wrong).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{error}");
        assert_no_account("wrong-attempt").await;
        assert_eq!(
            session_rows(session).await,
            1,
            "cross-attempt failure consumed the original challenge"
        );
        let mut changed = base.clone();
        changed["initial_device_display_name"] = json!("changed attempt");
        changed["auth"] = json!({"type":"m.login.dummy", "session":session});
        let (status, error) = register_request(service, changed).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{error}");
        assert_no_account("bound-attempt").await;
        let mut correct = base;
        correct["auth"] = json!({"type":"m.login.dummy", "session":session});
        let (status, account) = register_request(service, correct).await;
        assert_eq!(status, StatusCode::OK, "{account}");
        assert_eq!(
            account["user_id"],
            "@bound-attempt:dummy-registration.example"
        );
        assert_eq!(
            session_rows(session).await,
            0,
            "completed challenge and metadata survived"
        );
    }

    async fn assert_anonymous_registration_continuation(service: &Service) {
        let (status, first) =
            register_request(service, json!({"auth":{"type":"m.login.dummy"}})).await;
        assert_eq!(status, StatusCode::OK, "{first}");
        let base = json!({"password":"anonymous-password"});
        let (status, challenge) = register_request(service, base.clone()).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{challenge}");
        let session = challenge["session"].as_str().unwrap();
        let (status, fresh) = register_request(
            service,
            json!({"device_id":"ANON-OTHER", "auth":{"type":"m.login.dummy"}}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{fresh}");
        let mut omitted = base.clone();
        omitted["auth"] = json!({"type":"m.login.dummy"});
        let (status, retry) = register_request(service, omitted).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{retry}");
        assert_eq!(retry["session"], challenge["session"]);
        let mut correct = base;
        correct["auth"] = json!({"type":"m.login.dummy", "session":session});
        let (status, account) = register_request(service, correct).await;
        assert_eq!(status, StatusCode::OK, "{account}");
        assert!(
            account["user_id"]
                .as_str()
                .unwrap()
                .ends_with(":dummy-registration.example")
        );
        assert_eq!(session_rows(session).await, 0);
    }

    async fn assert_registration_session_expiry(service: &Service) {
        let (status, challenge) =
            register_request(service, json!({"username":"expired-binding"})).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let session = challenge["session"].as_str().unwrap();
        expire_session(session).await;
        let (status, error) = register_request(service, json!({"username":"expired-binding", "auth":{"type":"m.login.dummy", "session":session}})).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{error}");
        assert_no_account("expired-binding").await;
        assert_eq!(session_rows(session).await, 0);
        let (status, account) = register_request(
            service,
            json!({"username":"expired-binding", "auth":{"type":"m.login.dummy"}}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{account}");
        let (_, challenge) = register_request(service, json!({"username":"expired-lookup"})).await;
        let session = challenge["session"].as_str().unwrap();
        expire_session(session).await;
        let (status, account) = register_request(
            service,
            json!({"username":"expired-lookup", "auth":{"type":"m.login.dummy"}}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{account}");
        assert_eq!(
            session_rows(session).await,
            0,
            "expired binding was not pruned by the sessionless lookup"
        );

        let (_, challenge) = register_request(service, json!({"username":"changed-policy"})).await;
        let session = challenge["session"].as_str().unwrap();
        let mut info: Value = user_uiaa_datas::table
            .filter(user_uiaa_datas::session.eq(session))
            .select(user_uiaa_datas::uiaa_info)
            .first(&mut connect().await.unwrap())
            .await
            .unwrap();
        info["flows"] = json!([{"stages":["m.login.registration_token"]}]);
        diesel::update(user_uiaa_datas::table.filter(user_uiaa_datas::session.eq(session)))
            .set(user_uiaa_datas::uiaa_info.eq(info))
            .execute(&mut connect().await.unwrap())
            .await
            .unwrap();
        let (status, error) = register_request(service, json!({"username":"changed-policy", "auth":{"type":"m.login.dummy", "session":session}})).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{error}");
        assert_no_account("changed-policy").await;
    }

    #[test]
    fn registration_attempt_binding_contains_no_credential_verifier() {
        let first: RegisterReqBody = serde_json::from_value(
            json!({"username":"BIND", "password":"one", "auth":{"type":"m.login.dummy"}}),
        )
        .unwrap();
        let second: RegisterReqBody = serde_json::from_value(json!({"username":"bind", "password":"two", "auth":{"type":"m.login.registration_token", "token":"private-token"}})).unwrap();
        let user = UserId::parse("@bind:dummy-registration.example").unwrap();
        assert_eq!(
            registration_attempt_fingerprint(&first, &user).unwrap(),
            registration_attempt_fingerprint(&second, &user).unwrap()
        );
        let anonymous: RegisterReqBody =
            serde_json::from_value(json!({"password":"anonymous"})).unwrap();
        let generated = UserId::parse("@generated:dummy-registration.example").unwrap();
        assert_eq!(
            registration_attempt_fingerprint(&anonymous, &user).unwrap(),
            registration_attempt_fingerprint(&anonymous, &generated).unwrap()
        );
    }

    #[tokio::test]
    #[ignore = "run alone with an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn http_dummy_registration_open_policy() {
        let service = service("open");
        for (name, body) in [
            (
                "minimal",
                json!({"username":"minimal", "auth":{"type":"m.login.dummy"}}),
            ),
            (
                "blueprint",
                json!({"username":"blueprint", "password":"complement_meets_min_password_req",
                    "device_id":"BLUEPRINT", "auth":{"type":"m.login.dummy"}}),
            ),
        ] {
            let (status, result) = register_request(&service, body).await;
            assert_eq!(status, StatusCode::OK, "{result}");
            let user = format!("@{name}:dummy-registration.example");
            assert_eq!(result["user_id"], user);
            assert!(
                data::user::user_exists(UserId::parse(&user).unwrap().as_ref())
                    .await
                    .unwrap()
            );
            let mut response = TestClient::get("http://localhost/account/whoami")
                .add_header(
                    "authorization",
                    format!("Bearer {}", result["access_token"].as_str().unwrap()),
                    true,
                )
                .send(&service)
                .await;
            assert_eq!(response.status_code, Some(StatusCode::OK));
            let identity: Value = response.take_json().await.unwrap();
            assert_eq!(identity["user_id"], user);
        }
        let (status, challenge) = register_request(&service, json!({"username":"challenge"})).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(challenge["session"].is_string());
        let (status, error) = register_request(
            &service,
            json!({"username":"unknownsession",
            "auth":{"type":"m.login.dummy", "session":"not-issued"}}),
        )
        .await;
        assert!(!status.is_success(), "{error}");
        assert_no_account("unknownsession").await;
        assert_no_account("challenge").await;
        assert_registration_continuation(&service).await;
        assert_anonymous_registration_continuation(&service).await;
        assert_registration_session_expiry(&service).await;
    }

    async fn assert_policy_requires_stage(policy: &str, stage: &str) {
        let service = service(policy);
        let (status, challenge) = register_request(
            &service,
            json!({"username":"blocked",
            "auth":{"type":"m.login.dummy"}}),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{challenge}");
        assert_eq!(challenge["flows"][0]["stages"], json!([stage]));
        // Dummy also cannot complete a real, issued stronger-flow session.
        let (status, error) = register_request(
            &service,
            json!({"username":"blocked",
            "auth":{"type":"m.login.dummy", "session":challenge["session"]}}),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{error}");
        assert_no_account("blocked").await;
        let (status, fresh) = register_request(
            &service,
            json!({"username":"blocked", "auth":{"type":"m.login.dummy"}}),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{fresh}");
        assert_ne!(
            fresh["session"], challenge["session"],
            "strong-flow session was exposed to a caller without its session"
        );
        assert!(
            fresh
                .get("completed")
                .and_then(Value::as_array)
                .is_none_or(|stages| stages.is_empty())
        );
        assert_no_account("blocked").await;
    }

    #[tokio::test]
    #[ignore = "run alone with an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn http_dummy_registration_preserves_token_policy() {
        assert_policy_requires_stage("token", "m.login.registration_token").await;
    }

    #[tokio::test]
    #[ignore = "run alone with an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn http_dummy_registration_preserves_email_policy() {
        assert_policy_requires_stage("email", "m.login.email.identity").await;
    }

    #[tokio::test]
    #[ignore = "run alone with an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn http_dummy_registration_preserves_closed_policy() {
        let service = service("closed");
        let (status, error) = register_request(
            &service,
            json!({"username":"blocked",
            "auth":{"type":"m.login.dummy"}}),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{error}");
        assert_no_account("blocked").await;
    }
}
