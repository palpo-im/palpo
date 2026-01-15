use diesel::prelude::*;
use palpo_core::push::PusherIds;
use url::Url;

use crate::core::UnixMillis;
use crate::core::client::push::{PusherAction, PusherPostData};
use crate::core::events::TimelineEventType;
use crate::core::identifiers::*;
use crate::core::push::push_gateway::{
    Device, Notification, NotificationCounts, NotificationPriority, SendEventNotificationReqBody,
};
use crate::core::push::{Action, PushFormat, Pusher, PusherKind, Ruleset, Tweak};
use crate::data::connect;
use crate::data::schema::*;
use crate::data::user::pusher::NewDbPusher;
use crate::event::PduEvent;
use crate::{AppError, AppResult, AuthedInfo, data, room};

pub fn set_pusher(authed: &AuthedInfo, pusher: PusherAction) -> AppResult<()> {
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
            if !append {
                diesel::delete(
                    user_pushers::table
                        .filter(user_pushers::user_id.eq(authed.user_id()))
                        .filter(user_pushers::pushkey.eq(&pushkey))
                        .filter(user_pushers::app_id.eq(&app_id)),
                )
                .execute(&mut connect()?)?;
            }
            diesel::insert_into(user_pushers::table)
                .values(&NewDbPusher {
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
                    enabled: true, // TODO
                    created_at: UnixMillis::now(),
                })
                .execute(&mut connect()?)?;
        }
        PusherAction::Delete(ids) => {
            diesel::delete(
                user_pushers::table
                    .filter(user_pushers::user_id.eq(authed.user_id()))
                    .filter(user_pushers::pushkey.eq(ids.pushkey))
                    .filter(user_pushers::app_id.eq(ids.app_id)),
            )
            .execute(&mut connect()?)?;
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

    for action in data::user::pusher::get_actions(
        user,
        &ruleset,
        &power_levels,
        &pdu.to_sync_room_event(),
        &pdu.room_id,
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
                || tweaks
                    .iter()
                    .any(|t| matches!(t, Tweak::Highlight(true) | Tweak::Sound(_)))
            {
                notification.prio = NotificationPriority::High
            }

            if event_id_only {
                crate::sending::post(Url::parse(&http.url)?)
                    .stuff(SendEventNotificationReqBody::new(notification))?
                    .send::<()>()
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
                    data::user::display_name(&event.sender).ok().flatten();
                notification.room_name = room::get_name(&event.room_id).ok();

                crate::sending::post(Url::parse(&http.url)?)
                    .stuff(SendEventNotificationReqBody::new(notification))?
                    .send::<()>()
                    .await?;
            }

            Ok(())
        }
        // TODO: Handle email
        PusherKind::Email(_) => Ok(()),
        _ => Ok(()),
    }
}
