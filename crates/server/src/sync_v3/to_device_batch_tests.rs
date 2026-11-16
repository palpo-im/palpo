//! Run alone against an empty disposable PostgreSQL database.
use serde_json::json;

use super::*;

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
fn to_device_batches_retry_and_ack_without_skipping_backlog() {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(check());
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn check() {
    crate::test_database::init();
    config::CONFIG
        .set(
            serde_json::from_value(json!({
                "server_name":"mailbox.example", "db":{"url":"unused"}, "auto_join_rooms":[],
                "presence":{"allow_local":false,"allow_incoming":false,"allow_outgoing":false}
            }))
            .unwrap(),
        )
        .unwrap();
    let user = UserId::parse("@reader:mailbox.example").unwrap();
    let device = OwnedDeviceId::from("D");
    data::user::create_user(&data::user::NewDbUser {
        id: user.clone(),
        ty: None,
        is_admin: false,
        is_guest: false,
        is_local: true,
        localpart: "reader".into(),
        server_name: user.server_name().to_owned(),
        appservice_id: None,
        created_at: UnixMillis::now(),
    })
    .await
    .unwrap();
    data::user::ensure_profile_exists(&user).await.unwrap();
    data::user::device::create_device(&user, &device, "mailbox-token", None, None)
        .await
        .unwrap();
    for n in 0..257 {
        data::user::device::add_to_device_event(
            &user,
            &user,
            &device,
            "org.example.test",
            json!({"n":n}),
        )
        .await
        .unwrap();
    }
    let mut args: SyncEventsReqArgs = serde_json::from_value(
        json!({"filter":json!({"room":{"rooms":[]}}).to_string(),"set_presence":"offline"}),
    )
    .unwrap();
    let first = sync_events(&user, &device, &args).await.unwrap();
    assert_eq!(first.to_device.events.len(), 100);
    assert!(first.rooms.join.is_empty());
    let retry = sync_events(&user, &device, &args).await.unwrap();
    assert_eq!(
        serde_json::to_value(&retry.to_device).unwrap(),
        serde_json::to_value(&first.to_device).unwrap()
    );
    assert_eq!(retry.next_batch, first.next_batch);
    // A later arrival must remain behind the original outstanding backlog.
    data::user::device::add_to_device_event(
        &user,
        &user,
        &device,
        "org.example.test",
        json!({"n":257}),
    )
    .await
    .unwrap();
    let mut observed = Vec::new();
    let mut response = first;
    loop {
        for event in &response.to_device.events {
            let value: serde_json::Value = serde_json::from_str(event.as_str()).unwrap();
            observed.push(value["content"]["n"].as_u64().unwrap());
        }
        args.since = Some(response.next_batch);
        response = sync_events(&user, &device, &args).await.unwrap();
        assert!(response.to_device.events.len() <= 100);
        if response.to_device.events.is_empty() {
            break;
        }
    }
    assert_eq!(observed, (0..258).collect::<Vec<_>>());
    assert!(
        data::user::device::get_to_device_events(&user, &device, None, None)
            .await
            .unwrap()
            .is_empty()
    );
    // A count-only cap would exceed the proxy's raw response limit.
    for n in 0..4 {
        data::user::device::add_to_device_event(
            &user,
            &user,
            &device,
            "org.example.large",
            json!({"n":n,"pad":"x".repeat(200_000)}),
        )
        .await
        .unwrap();
    }
    let response = sync_events(&user, &device, &args).await.unwrap();
    assert_eq!(response.to_device.events.len(), 2);
    assert!(serde_json::to_vec(&response.to_device).unwrap().len() <= 512 * 1024);
    args.since = Some(response.next_batch);
    let response = sync_events(&user, &device, &args).await.unwrap();
    assert_eq!(response.to_device.events.len(), 2);
    args.since = Some(response.next_batch);
    // A single legal event larger than the preferred batch budget still progresses.
    data::user::device::add_to_device_event(
        &user,
        &user,
        &device,
        "org.example.large",
        json!({"pad":"x".repeat(600_000)}),
    )
    .await
    .unwrap();
    let response = sync_events(&user, &device, &args).await.unwrap();
    assert_eq!(response.to_device.events.len(), 1);
    args.since = Some(response.next_batch);
    assert!(
        sync_events(&user, &device, &args)
            .await
            .unwrap()
            .to_device
            .events
            .is_empty()
    );
}
