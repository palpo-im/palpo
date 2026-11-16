//! Run alone against an empty disposable PostgreSQL database.
use serde_json::json;

use super::*;

#[test]
fn sync_positions_reject_history_and_negative_values_without_overflow() {
    for input in ["12", "s12", "p12", "p12_i1_pABC_wAb2"] {
        assert_eq!(
            parse_sync_position(input).unwrap(),
            BatchToken::new_publication(12)
        );
    }
    for input in [
        "s-1000000",
        "-1000000",
        "t0--1000000",
        "s-9223372036854775808",
        "-9223372036854775808",
        "t0--9223372036854775808",
        "t0-1",
    ] {
        // These remain available to history readers, but cannot ACK a sync.
        assert!(input.parse::<BatchToken>().is_ok(), "{input}");
        assert!(parse_sync_position(input).is_err(), "{input}");
    }
    let page = BatchToken::new_publication_page(None, None, 0, 0).to_string();
    assert!(parse_sync_position(&page).is_err());
}

#[test]
fn sync_window_end_is_checked_at_sequence_limit() {
    assert_eq!(sync_window_end(0).unwrap(), 1);
    assert_eq!(sync_window_end(Seqnum::MAX - 1).unwrap(), Seqnum::MAX);
    assert!(sync_window_end(Seqnum::MAX).is_err());
}

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
        // A full-state request on an intermediate page must not skip backlog.
        args.full_state = observed.len() == 100;
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
    // A count-only cap would exceed the preferred serialized-event budget.
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
    let response = sync_events(&user, &device, &args).await.unwrap();
    assert!(response.to_device.events.is_empty());
    let resume = response.next_batch;

    // A forged future position must never ACK an unread event, including when
    // full_state is requested. Keep its cursor rather than issuing an ACKable one.
    data::user::device::add_to_device_event(
        &user,
        &user,
        &device,
        "org.example.guard",
        json!({"n":"unread"}),
    )
    .await
    .unwrap();
    let queued = serde_json::to_value(
        data::user::device::get_to_device_events(&user, &device, None, None)
            .await
            .unwrap(),
    )
    .unwrap();
    let curr_sn = crate::event::sticky::curr_sn_after_sync_writes(&user, &device)
        .await
        .unwrap();
    let future = BatchToken::new_publication(curr_sn + 2).to_string();
    for full_state in [false, true] {
        args.since = Some(future.clone());
        args.full_state = full_state;
        let response = sync_events(&user, &device, &args).await.unwrap();
        assert!(response.to_device.events.is_empty());
        assert_eq!(response.next_batch, future);
        assert_eq!(
            serde_json::to_value(
                data::user::device::get_to_device_events(&user, &device, None, None)
                    .await
                    .unwrap(),
            )
            .unwrap(),
            queued
        );
    }
    // Signed legacy/history values cannot become positive inbox ACK positions,
    // and MIN must be rejected before BatchToken::event_sn() can take its abs().
    for invalid in [
        "s-1000000",
        "-1000000",
        "t0--1000000",
        "s-9223372036854775808",
        "-9223372036854775808",
        "t0--9223372036854775808",
        "t0-1",
    ] {
        args.since = Some(invalid.into());
        args.full_state = true;
        assert!(
            sync_events(&user, &device, &args).await.is_err(),
            "{invalid}"
        );
        assert_eq!(
            serde_json::to_value(
                data::user::device::get_to_device_events(&user, &device, None, None)
                    .await
                    .unwrap(),
            )
            .unwrap(),
            queued
        );
    }
    args.since = Some(resume);
    args.full_state = false;
    let response = sync_events(&user, &device, &args).await.unwrap();
    assert_eq!(
        serde_json::to_value(&response.to_device.events).unwrap(),
        queued
    );
    assert_eq!(
        parse_sync_position(&response.next_batch)
            .unwrap()
            .event_sn(),
        curr_sn + 1
    );

    // A new arrival after that response's snapshot is not covered by its ACK.
    data::user::device::add_to_device_event(
        &user,
        &user,
        &device,
        "org.example.guard",
        json!({"n":"later"}),
    )
    .await
    .unwrap();
    args.since = Some(response.next_batch);
    let response = sync_events(&user, &device, &args).await.unwrap();
    assert_eq!(response.to_device.events.len(), 1);
    assert_eq!(
        serde_json::to_value(&response.to_device.events).unwrap()[0]["content"]["n"],
        "later"
    );
    assert_eq!(
        serde_json::to_value(
            data::user::device::get_to_device_events(&user, &device, None, None)
                .await
                .unwrap(),
        )
        .unwrap(),
        serde_json::to_value(&response.to_device.events).unwrap()
    );

    // The legitimate C+1 tail token must ACK even with full_state and no writes.
    args.since = Some(response.next_batch);
    args.full_state = true;
    assert!(
        sync_events(&user, &device, &args)
            .await
            .unwrap()
            .to_device
            .events
            .is_empty()
    );
    assert!(
        data::user::device::get_to_device_events(&user, &device, None, None)
            .await
            .unwrap()
            .is_empty()
    );
}
