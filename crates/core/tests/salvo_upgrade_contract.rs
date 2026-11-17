use std::time::Duration;

use palpo_core::appservice::ping::SendPingResBody;
use palpo_core::appservice::third_party::ForProtocolReqArgs;
use palpo_core::client::session::RefreshTokenResBody;
use palpo_core::client::sync_events::v3::SyncEventsReqArgs;
use palpo_core::client::voip::TurnServerResBody;
use salvo::oapi::{Components, ParameterIn, ToParameters, ToSchema};
use salvo::prelude::*;
use salvo::test::{ResponseExt, TestClient};
use serde_json::{Value, json};

#[endpoint]
fn protocol_lookup(args: ForProtocolReqArgs) -> String {
    format!(
        "{}|{}",
        args.protocol,
        serde_json::to_string(&args.fields).unwrap()
    )
}

#[tokio::test]
async fn protocol_lookup_extracts_custom_query_fields_without_replacing_the_path() {
    let service = Service::new(Router::with_path("{protocol}").get(protocol_lookup));
    let mut response = TestClient::get(
        "http://localhost/irc?channel=%23room%2Fpart&network=libera&protocol=query-value&access_token=secret",
    )
    .send(&service)
    .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    assert_eq!(
        response.take_string().await.unwrap(),
        "irc|{\"channel\":\"#room/part\",\"network\":\"libera\",\"protocol\":\"query-value\"}"
    );

    let mut response = TestClient::get("http://localhost/irc").send(&service).await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    assert_eq!(response.take_string().await.unwrap(), "irc|{}");

    let parameters = ForProtocolReqArgs::to_parameters(&mut Components::new());
    assert_eq!(parameters.0.len(), 1);
    assert!(parameters.contains("protocol", ParameterIn::Path));
}

#[endpoint]
fn sync_timeout(args: SyncEventsReqArgs) -> String {
    args.timeout
        .map(|timeout| timeout.as_millis().to_string())
        .unwrap_or_default()
}

#[tokio::test]
async fn sync_query_timeout_remains_optional_integer_milliseconds() {
    let service = Service::new(Router::new().get(sync_timeout));
    for (query, expected) in [("?timeout=1234", "1234"), ("", "")] {
        let mut response = TestClient::get(format!("http://localhost/{query}"))
            .send(&service)
            .await;
        assert_eq!(response.status_code, Some(StatusCode::OK));
        assert_eq!(response.take_string().await.unwrap(), expected);
    }
    for query in ["?timeout=-1", "?timeout=1.5", "?timeout=invalid"] {
        let response = TestClient::get(format!("http://localhost/{query}"))
            .send(&service)
            .await;
        assert_eq!(response.status_code, Some(StatusCode::BAD_REQUEST));
    }
}

fn schema<T: ToSchema>() -> Value {
    let mut components = Components::new();
    let schema = serde_json::to_value(T::to_schema(&mut components)).unwrap();
    match schema.get("$ref").and_then(Value::as_str) {
        Some(reference) => {
            let name = reference.strip_prefix("#/components/schemas/").unwrap();
            serde_json::to_value(&components.schemas[name]).unwrap()
        }
        None => schema,
    }
}

#[test]
fn duration_response_schemas_match_the_existing_json_wire_format() {
    let duration = Duration::from_millis(1234);
    assert_eq!(
        serde_json::to_value(SendPingResBody::new(duration)).unwrap(),
        json!({"duration_ms": 1234})
    );
    let ping_schema = schema::<SendPingResBody>();
    assert_eq!(ping_schema["properties"]["duration_ms"]["type"], "integer");
    assert!(
        ping_schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!("duration_ms"))
    );

    let mut refresh = RefreshTokenResBody::new("token".to_owned());
    assert_eq!(
        serde_json::to_value(&refresh).unwrap(),
        json!({"access_token": "token"})
    );
    refresh.expires_in_ms = Some(duration);
    assert_eq!(
        serde_json::to_value(&refresh).unwrap(),
        json!({"access_token": "token", "expires_in_ms": 1234})
    );
    let refresh_schema = schema::<RefreshTokenResBody>();
    let property_type = &refresh_schema["properties"]["expires_in_ms"]["type"];
    assert!(
        property_type == "integer"
            || property_type
                .as_array()
                .is_some_and(|types| types.contains(&json!("integer")))
    );
    assert!(
        !refresh_schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!("expires_in_ms"))
    );

    let turn = TurnServerResBody::new("user".to_owned(), "password".to_owned(), vec![], duration);
    assert_eq!(serde_json::to_value(turn).unwrap()["ttl"], 1);
    assert_eq!(
        schema::<TurnServerResBody>()["properties"]["ttl"]["type"],
        "integer"
    );
}
