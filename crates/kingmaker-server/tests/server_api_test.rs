use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use kingmaker_core::*;
use kingmaker_server::routes::create_router;
use kingmaker_server::state::AppState;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower::ServiceExt;

fn create_mock_state() -> AppState {
    let mut world = WorldState::default();
    world.campaign = CampaignHeader {
        campaign_id: "world-306".to_string(),
        revision: 1,
        year: 100,
        month: 1,
        day: 1,
        ..Default::default()
    };
    world.provinces.push(ProvinceRecord {
        id: ProvinceId(1),
        name: "Test Province".to_string(),
        owner_state_id: Some(1u32.into()),
        controller_state_id: Some(1u32.into()),
        pop_range: (0, 1),
        ..Default::default()
    });
    world.pops.push(PopRecord {
        id: PopId(1),
        external_id: "pop:1".into(),
        currency_id: Some("COIN".into()),
        workforce: 60.0,
        employed: 0.0,
        unemployed: 60.0,
        province_id: ProvinceId(1),
        pop_type: "peasant".to_string(),
        size: 100,
        culture: "Imperial".to_string(),
        religion: "Sun".to_string(),
        account_id: Some(AccountId(1)),
        literacy: Some(0.1),
        militancy: Some(0.0),
        consciousness: Some(0.0),
        ..Default::default()
    });
    world.accounts.push(AccountBalance {
        id: AccountId(1),
        owner_id: "pop:1".into(),
        currency_id: "COIN".into(),
        purpose: "personal_savings".into(),
        external_id: "account:1".into(),
        balance: Money(500),
    });
    AppState {
        world: Arc::new(RwLock::new(world)),
        recipes: Arc::new(HashMap::new()),
    }
}

#[tokio::test]
async fn test_api_health() {
    let state = create_mock_state();
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "read_only");
    assert_eq!(json["service"], "teyvankal-kingmaker-rust");
    assert_eq!(json["provinces"], 1);
}

#[tokio::test]
async fn test_api_map() {
    let state = create_mock_state();
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/campaign/world-306/map")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["data"]["provinces"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn test_api_reports() {
    let state = create_mock_state();
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/campaign/world-306/reports")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["mode"], "read_only");
    assert!(json["data"].as_array().is_some());
}

#[tokio::test]
async fn test_api_province_details() {
    let state = create_mock_state();
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/campaign/world-306/province/1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["data"]["name"], "Test Province");
}

#[tokio::test]
async fn test_api_execute_week() {
    let state = create_mock_state();
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/campaign/world-306/execute")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["code"], "SERVER_WEEK_EXECUTION_NOT_IMPLEMENTED");
    assert_eq!(json["campaign"]["revision"], 1);
    assert_eq!(json["native_ram_week_available"], true);
    assert_eq!(json["native_checkpoint_available"], true);
    assert_eq!(json["native_measured_runner_available"], true);
    assert_eq!(
        json["missing_workflows"],
        serde_json::json!(["server_isolated_campaign_commit"])
    );
}

#[tokio::test]
async fn execute_wrong_campaign_and_incomplete_engine_preserve_world() {
    let state = create_mock_state();
    let before = state.world.read().await.clone();
    for (id, status) in [
        ("wrong", StatusCode::NOT_FOUND),
        ("world-306", StatusCode::SERVICE_UNAVAILABLE),
    ] {
        let response = create_router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/campaign/{id}/execute"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), status);
        assert_eq!(*state.world.read().await, before);
    }
}
