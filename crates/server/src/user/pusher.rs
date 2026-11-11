use palpo_core::push::PusherIds;
use url::Url;

use crate::core::UnixMillis;
use crate::core::client::push::{PusherAction, PusherPostData};
use crate::core::events::TimelineEventType;
use crate::core::identifiers::*;
use crate::core::push::push_gateway::{
    Device, Notification, NotificationCounts, NotificationPriority, SendEventNotificationReqBody,
    SendEventNotificationResBody,
};
use crate::core::push::{
    Action, HighlightTweakValue, PushFormat, Pusher, PusherKind, Ruleset, Tweak,
};
use crate::data::user::pusher::{DbPusher, NewDbPusher};
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
                device_id: authed.require_device_id()?.to_owned(),
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
    pusher: &DbPusher,
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
    registration: &DbPusher,
    tweaks: Vec<Tweak>,
    event: &PduEvent,
) -> AppResult<()> {
    let pusher: Pusher = registration.clone().try_into()?;
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

            if !event_id_only {
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
            }

            let response = send_gateway_notification(push_client, url, notification).await?;
            remove_rejected_pusher(registration, &response).await?;

            Ok(())
        }
        // TODO: Handle email
        PusherKind::Email(_) => Ok(()),
        _ => Ok(()),
    }
}

async fn send_gateway_notification(
    client: reqwest::Client,
    url: Url,
    notification: Notification,
) -> AppResult<SendEventNotificationResBody> {
    let request = sending::post(url)
        .stuff(SendEventNotificationReqBody::new(notification))?
        .into_inner();
    // Receiving HTTP is not an acknowledgement. In particular, a gateway may
    // return a well-formed JSON error with 503; preserve the durable request.
    let response = client.execute(request).await?;
    if !response.status().is_success() {
        return Err(AppError::public(format!(
            "Push gateway refused notification with HTTP {}",
            response.status()
        )));
    }
    Ok(response.json().await?)
}

async fn remove_rejected_pusher(
    pusher: &DbPusher,
    response: &SendEventNotificationResBody,
) -> AppResult<()> {
    // Only this device was sent to this gateway. Never let a gateway response
    // remove another application's keys or another user's registration.
    if response.rejected.iter().any(|key| key == &pusher.pushkey) {
        data::user::pusher::delete_rejected_pusher(pusher).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use diesel::prelude::*;
    use horme::RunQueryDsl;

    use super::*;
    use crate::data::room::NewDbRoomUser;
    use crate::data::schema::*;

    fn gateway(body: &'static str, status: &'static str) -> (Url, std::thread::JoinHandle<()>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = Url::parse(&format!("http://{}/notify", listener.local_addr().unwrap())).unwrap();
        let peer = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buf = [0; 1024];
            loop {
                let n = stream.read(&mut buf).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buf[..n]);
                if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
                    let length: usize = head
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length: "))
                        .unwrap()
                        .trim()
                        .parse()
                        .unwrap();
                    if bytes.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        });
        (url, peer)
    }

    #[tokio::test]
    async fn gateway_requires_success_status_and_standard_response() {
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .unwrap();
        for (body, status, success) in [
            (r#"{"rejected":[]}"#, "200 OK", true),
            (r#"{"rejected":["expired"]}"#, "200 OK", true),
            (r#"{"rejected":[]}"#, "503 Service Unavailable", false),
            (r#"{"rejected":[]}"#, "302 Found", false),
            ("invalid-json", "200 OK", false),
        ] {
            let (url, peer) = gateway(body, status);
            let result =
                send_gateway_notification(client.clone(), url, Notification::new(vec![])).await;
            assert_eq!(result.is_ok(), success, "{status} {body}");
            if body.contains("expired") {
                assert_eq!(result.unwrap().rejected, ["expired"]);
            }
            peer.join().unwrap();
        }
        // Transport failure must also propagate so the durable caller retries.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = Url::parse(&format!("http://{}/notify", listener.local_addr().unwrap())).unwrap();
        drop(listener);
        assert!(
            send_gateway_notification(client, url, Notification::new(vec![]))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn review_rejected_gateway_keys_only_remove_matching_pusher() {
        crate::test_database::init();
        let alice = UserId::parse("@alice:example.com").unwrap();
        for row in [
            http_pusher(alice.as_str(), "app", "expired"),
            http_pusher(alice.as_str(), "other-app", "expired"),
            http_pusher("@bob:example.com", "app", "expired"),
            http_pusher(alice.as_str(), "app", "other-key"),
        ] {
            data::user::pusher::set_pusher(&row, true).await.unwrap();
        }
        let pusher = data::user::pusher::get_pushers(&alice)
            .await
            .unwrap()
            .into_iter()
            .find(|row| row.app_id == "app" && row.pushkey == "expired")
            .unwrap();
        remove_rejected_pusher(
            &pusher,
            &SendEventNotificationResBody::new(vec!["other-key".into()]),
        )
        .await
        .unwrap();
        assert_eq!(pusher_owners("app", "expired").await.len(), 2);
        // A response from the previous gateway must not delete the replacement,
        // even though its user/app/key tuple is unchanged.
        let mut replacement = http_pusher(alice.as_str(), "app", "expired");
        replacement.data = serde_json::json!({"url":"https://new-gateway.example/notify"});
        data::user::pusher::set_pusher(&replacement, true)
            .await
            .unwrap();
        remove_rejected_pusher(
            &pusher,
            &SendEventNotificationResBody::new(vec!["expired".into()]),
        )
        .await
        .unwrap();
        assert_eq!(pusher_owners("app", "expired").await.len(), 2);
        let pusher = data::user::pusher::get_pushers(&alice)
            .await
            .unwrap()
            .into_iter()
            .find(|row| row.app_id == "app" && row.pushkey == "expired")
            .unwrap();
        // Also fence updates that preserve the database row id.
        diesel::update(user_pushers::table.find(pusher.id))
            .set(
                user_pushers::data
                    .eq(serde_json::json!({"url":"https://updated-in-place.example/notify"})),
            )
            .execute(&mut crate::data::db())
            .await
            .unwrap();
        remove_rejected_pusher(
            &pusher,
            &SendEventNotificationResBody::new(vec!["expired".into()]),
        )
        .await
        .unwrap();
        assert_eq!(pusher_owners("app", "expired").await.len(), 2);
        let current = data::user::pusher::get_pushers(&alice)
            .await
            .unwrap()
            .into_iter()
            .find(|row| row.id == pusher.id)
            .unwrap();
        remove_rejected_pusher(
            &current,
            &SendEventNotificationResBody::new(vec!["expired".into()]),
        )
        .await
        .unwrap();
        assert_eq!(pusher_owners("app", "expired").await, ["@bob:example.com"]);
        assert_eq!(
            pusher_owners("other-app", "expired").await,
            [alice.as_str()]
        );
        assert_eq!(pusher_owners("app", "other-key").await, [alice.as_str()]);
    }

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
            .load::<String>(&mut crate::data::db())
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
                .execute(&mut crate::data::db())
                .await
                .unwrap();
        }

        // No statistics row has been written for this room, which is the state of rooms
        // whose membership has not changed since statistics were introduced.
        assert_eq!(room::joined_member_count(&room_id).await.unwrap(), 2);
    }
}
