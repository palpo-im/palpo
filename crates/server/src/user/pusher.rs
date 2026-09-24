use palpo_core::push::PusherIds;
use url::Url;

use crate::core::UnixMillis;
use crate::core::client::push::{PusherAction, PusherPostData};
use crate::core::events::TimelineEventType;
use crate::core::identifiers::*;
use crate::core::push::push_gateway::{
    Device, Notification, NotificationCounts, NotificationPriority, SendEventNotificationReqBody,
};
use crate::core::push::{
    Action, HighlightTweakValue, PushFormat, Pusher, PusherKind, Ruleset, Tweak,
};
use crate::data::user::pusher::NewDbPusher;
use crate::event::PduEvent;
use crate::utils::url_guard;
use crate::{AppError, AppResult, AuthedInfo, data, room, sending};

pub async fn set_pusher(authed: &AuthedInfo, pusher: PusherAction) -> AppResult<()> {
    match pusher {
        PusherAction::Post(data) => {
            let PusherPostData {
                pusher:
                    Pusher {
                        ids: PusherIds { app_id, pushkey },
                        kind,
                        app_display_name,
                        device_display_name,
                        lang,
                        profile_tag,
                        ..
                    },
                append,
            } = data;
            let new_pusher = NewDbPusher {
                user_id: authed.user_id().to_owned(),
                profile_tag,
                kind: kind.name().to_owned(),
                app_id,
                app_display_name,
                device_id: authed.device_id().to_owned(),
                device_display_name,
                access_token_id: authed.access_token_id().to_owned(),
                pushkey,
                lang,
                data: kind.json_data()?,
                enabled: true,
                created_at: UnixMillis::now(),
            };
            data::user::pusher::set_pusher(&new_pusher, append).await?;
        }
        PusherAction::Delete(ids) => {
            data::user::pusher::delete_pusher(authed.user_id(), &ids.app_id, &ids.pushkey).await?;
        }
    }
    Ok(())
}

// #[tracing::instrument(skip(destination, request))]
// pub async fn send_request<T: OutgoingRequest>(destination: &str, request: T) ->
// AppResult<T::IncomingResponse> where
//     T: Debug,
// {
//     let destination = destination.replace("/_matrix/push/v1/notify", "");

//     let http_request = request
//         .try_into_http_request::<BytesMut>(&destination, SendDbAccessToken::IfRequired(""),
// &[MatrixVersion::V1_0])         .map_err(|e| {
//             warn!("Failed to find destination {}: {}", destination, e);
//             AppError::public("Invalid destination")
//         })?
//         .map(|body| body.freeze());

//     let reqwest_request = reqwest::Request::try_from(http_request).expect("all http requests are
// valid reqwest requests");

//     // TODO: we could keep this very short and let expo backoff do it's thing...
//     //*reqwest_request.timeout_mut() = Some(Duration::from_secs(5));

//     let url = reqwest_request.url().clone();
//     let response = crate::default_client().execute(reqwest_request).await;

//     match response {
//         Ok(mut response) => {
//             // reqwest::Response -> http::Response conversion
//             let status = response.status();
//             let mut http_response_builder =
// http::Response::builder().status(status).version(response.version());             mem::swap(
//                 response.headers_mut(),
//                 http_response_builder.headers_mut().expect("http::response::Builder is usable"),
//             );

//             let body = response.bytes().await.unwrap_or_else(|e| {
//                 warn!("server error {}", e);
//                 Vec::new().into()
//             }); // TODO: handle timeout

//             if status != 200 {
//                 info!(
//                     "Push gateway returned bad response {} {}\n{}\n{:?}",
//                     destination,
//                     status,
//                     url,
//                     crate::utils::string_from_bytes(&body)
//                 );
//             }

//             let response =
// T::IncomingResponse::try_from_http_response(http_response_builder.body(body).expect("reqwest body
// is valid http body"));             response.map_err(|_| {
//                 info!("Push gateway returned invalid response bytes {}\n{}", destination, url);
//                 AppError::public("Push gateway returned bad response.")
//             })
//         }
//         Err(e) => {
//             warn!("Could not send request to pusher {}: {}", destination, e);
//             Err(e.into())
//         }
//     }
// }

#[tracing::instrument(skip(user, unread, pusher, ruleset, pdu))]
pub async fn send_push_notice(
    user: &UserId,
    unread: u64,
    pusher: &Pusher,
    ruleset: Ruleset,
    pdu: &PduEvent,
) -> AppResult<()> {
    let mut notify = None;
    let mut tweaks = Vec::new();
    let power_levels = room::get_power_levels(&pdu.room_id).await?;
    let member_count = room::joined_member_count(&pdu.room_id).await?;

    for action in data::user::pusher::get_actions(
        user,
        &ruleset,
        &power_levels,
        &pdu.to_sync_room_event_without_transaction_id(),
        &pdu.room_id,
        member_count,
    )
    .await?
    {
        let n = match action {
            Action::Notify => true,
            Action::SetTweak(tweak) => {
                tweaks.push(tweak.clone());
                continue;
            }
            _ => false,
        };
        if notify.is_some() {
            return Err(AppError::internal(
                r#"Malformed pushrule contains more than one of these actions: ["dont_notify", "notify", "coalesce"]"#,
            ));
        }
        notify = Some(n);
    }

    if notify == Some(true) {
        send_notice(unread, pusher, tweaks, pdu).await?;
    }
    // Else the event triggered no actions

    Ok(())
}

#[tracing::instrument(skip_all)]
async fn send_notice(
    unread: u64,
    pusher: &Pusher,
    tweaks: Vec<Tweak>,
    event: &PduEvent,
) -> AppResult<()> {
    // TODO: email
    match &pusher.kind {
        PusherKind::Http(http) => {
            // Two problems with this
            // 1. if "event_id_only" is the only format kind it seems we should never add more info
            // 2. can pusher/devices have conflicting formats
            let event_id_only = http.format == Some(PushFormat::EventIdOnly);

            let mut device = Device::new(pusher.ids.app_id.clone(), pusher.ids.pushkey.clone());
            device.data.default_payload = http.default_payload.clone();
            device.data.format = http.format.clone();

            // Tweaks are only added if the format is NOT event_id_only
            if !event_id_only {
                device.tweaks = tweaks.clone();
            }

            let d = vec![device];
            let mut notification = Notification::new(d);

            notification.prio = NotificationPriority::Low;
            notification.event_id = Some((*event.event_id).to_owned());
            notification.room_id = Some((*event.room_id).to_owned());
            // TODO: missed calls
            notification.counts = NotificationCounts::new(unread, 0);

            if event.event_ty == TimelineEventType::RoomEncrypted
                || tweaks.iter().any(|t| {
                    matches!(
                        t,
                        Tweak::Highlight(HighlightTweakValue::Yes) | Tweak::Sound(_)
                    )
                })
            {
                notification.prio = NotificationPriority::High
            }

            // Refuse user-supplied URLs that point at internal addresses or
            // use schemes other than http(s). This is the boundary check;
            // DNS-name hosts that resolve to denylisted IPs are also caught
            // at connect time by `push_gateway_client`'s safe DNS resolver.
            let url = Url::parse(&http.url)?;
            url_guard::ensure_safe_outbound_url(&url)?;
            let push_client = sending::push_gateway_client();

            if event_id_only {
                crate::sending::post(url)
                    .stuff(SendEventNotificationReqBody::new(notification))?
                    .send_by_client::<()>(push_client)
                    .await?;
            } else {
                notification.sender = Some(event.sender.clone());
                notification.event_type = Some(event.event_ty.clone());
                notification.content = serde_json::value::to_raw_value(&event.content).ok();
                if event.event_ty == TimelineEventType::RoomMember {
                    notification.user_is_target =
                        event.state_key.as_deref() == Some(event.sender.as_str());
                }
                notification.sender_display_name =
                    data::user::display_name(&event.sender).await.ok().flatten();
                notification.room_name = room::get_name(&event.room_id).await.ok();

                crate::sending::post(url)
                    .stuff(SendEventNotificationReqBody::new(notification))?
                    .send_by_client::<()>(push_client)
                    .await?;
            }

            Ok(())
        }
        // TODO: Handle email
        PusherKind::Email(_) => Ok(()),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;

    use super::*;
    use crate::data::connect;
    use crate::data::room::NewDbRoomUser;
    use crate::data::schema::*;

    fn http_pusher(user_id: &str, app_id: &str, pushkey: &str) -> NewDbPusher {
        NewDbPusher {
            user_id: UserId::parse(user_id).unwrap(),
            kind: "http".to_owned(),
            app_id: app_id.to_owned(),
            app_display_name: "App".to_owned(),
            device_id: "DEVICE".into(),
            device_display_name: "Device".to_owned(),
            access_token_id: None,
            profile_tag: None,
            pushkey: pushkey.to_owned(),
            lang: "en".to_owned(),
            data: serde_json::json!({ "url": "https://push.example/_matrix/push/v1/notify" }),
            enabled: true,
            created_at: UnixMillis::now(),
        }
    }

    async fn pusher_owners(app_id: &str, pushkey: &str) -> Vec<String> {
        user_pushers::table
            .filter(user_pushers::app_id.eq(app_id))
            .filter(user_pushers::pushkey.eq(pushkey))
            .order_by(user_pushers::user_id)
            .select(user_pushers::user_id)
            .load::<String>(&mut connect().await.unwrap())
            .await
            .unwrap()
    }

    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn database_set_pusher_follows_append_semantics() {
        crate::test_database::init();
        let (app_id, pushkey) = ("m.example.app", "shared-pushkey");

        // Setting the same pusher again replaces it instead of adding a duplicate, whether
        // or not `append` is set.
        data::user::pusher::set_pusher(&http_pusher("@alice:example.com", app_id, pushkey), false)
            .await
            .unwrap();
        data::user::pusher::set_pusher(&http_pusher("@alice:example.com", app_id, pushkey), true)
            .await
            .unwrap();
        assert_eq!(pusher_owners(app_id, pushkey).await, ["@alice:example.com"]);

        // `append` keeps other users' pushers for the same key.
        data::user::pusher::set_pusher(&http_pusher("@bob:example.com", app_id, pushkey), true)
            .await
            .unwrap();
        assert_eq!(
            pusher_owners(app_id, pushkey).await,
            ["@alice:example.com", "@bob:example.com"]
        );

        // Without `append`, the key moves to the new user.
        data::user::pusher::set_pusher(&http_pusher("@carol:example.com", app_id, pushkey), false)
            .await
            .unwrap();
        assert_eq!(pusher_owners(app_id, pushkey).await, ["@carol:example.com"]);
    }

    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn database_joined_member_count_without_statistics_counts_members() {
        crate::test_database::init();
        let room_id = RoomId::parse("!push-member-count:example.com").unwrap();

        for (index, (user_id, membership)) in [
            ("@alice:example.com", "join"),
            ("@bob:example.com", "join"),
            ("@carol:example.com", "leave"),
        ]
        .into_iter()
        .enumerate()
        {
            let user_id = UserId::parse(user_id).unwrap();
            diesel::insert_into(room_users::table)
                .values(&NewDbRoomUser {
                    event_id: EventId::parse(format!("$member-count-{index}")).unwrap(),
                    event_sn: index as i64,
                    room_id: room_id.clone(),
                    room_server_id: Some(room_id.server_name().unwrap().to_owned()),
                    user_server_id: user_id.server_name().to_owned(),
                    sender_id: user_id.clone(),
                    user_id,
                    membership: membership.to_owned(),
                    forgotten: false,
                    display_name: None,
                    avatar_url: None,
                    state_data: None,
                    created_at: UnixMillis::now(),
                })
                .execute(&mut connect().await.unwrap())
                .await
                .unwrap();
        }

        // No statistics row has been written for this room, which is the state of rooms
        // whose membership has not changed since statistics were introduced.
        assert_eq!(room::joined_member_count(&room_id).await.unwrap(), 2);
    }
}
