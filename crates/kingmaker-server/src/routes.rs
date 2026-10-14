use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::{get, post},
    Router,
};
use kingmaker_core::{ProvinceId, WorldState};
use serde_json::{json, Value};

pub fn create_router(state: AppState) -> Router {
    Router::new()
        .route("/api/health", get(health_handler))
        .route("/api/session", get(session_handler))
        .route("/api/campaigns", get(campaigns_handler))
        .route("/api/campaign/{id}/map", get(map_handler))
        .route("/api/campaign/{id}/reports", get(reports_handler))
        .route(
            "/api/campaign/{id}/province/{province_id}",
            get(province_handler),
        )
        .route("/api/campaign/{id}/execute", post(execute_handler))
        .with_state(state)
}
fn campaign(world: &WorldState) -> Value {
    json!({"campaign_id":world.campaign.campaign_id,"revision":world.campaign.revision,
        "date":{"year":world.campaign.year,"month":world.campaign.month,"day":world.campaign.day},
        "engine_hash":world.campaign.engine_hash,"rules_hash":world.campaign.rules_hash,"source_hash":world.campaign.source_hash})
}
fn generation(world: &WorldState) -> String {
    format!(
        "rust-read-only:{}:{}",
        world.campaign.campaign_id, world.campaign.revision
    )
}
fn missing_campaign() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(json!({"error":"Campaign not found","code":"CAMPAIGN_NOT_FOUND"})),
    )
        .into_response()
}
async fn health_handler(State(state): State<AppState>) -> Json<Value> {
    let world = state.world.read().await;
    Json(
        json!({"service":"teyvankal-kingmaker-rust","version":1,"revision":world.campaign.revision,
        "provinces":world.provinces.len(),"pops":world.pops.len(),"status":"read_only",
        "native_week_ready":false,"native_ram_week_available":true,"native_checkpoint_available":true,"native_measured_runner_available":true,"native_isolated_market":true,"market_stages":["maintenance","early_food","closing_market"],"missing_workflows":["server_isolated_campaign_commit"],"generation":generation(&world)}),
    )
}
async fn session_handler(State(state): State<AppState>) -> Json<Value> {
    let world = state.world.read().await;
    Json(
        json!({"token":"read-only-no-write-token","generation":generation(&world),"mode":"read_only"}),
    )
}
async fn campaigns_handler(State(state): State<AppState>) -> Json<Value> {
    let world = state.world.read().await;
    Json(json!({"data":[campaign(&world)],"mode":"read_only","generation":generation(&world)}))
}
async fn map_handler(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let world = state.world.read().await;
    if id != world.campaign.campaign_id {
        return missing_campaign();
    }
    let provs:Vec<_>=world.provinces.iter().map(|p|json!({"province_id":p.id.0,"name":p.name,
        "owner_state_id":p.owner_state_id,"controller_state_id":p.controller_state_id,"goods_id":p.goods_id,"installed_capacity":p.installed_capacity})).collect();
    Json(json!({"campaign":campaign(&world),"generation":generation(&world),"mode":"read_only","data":{"provinces":provs}})).into_response()
}
async fn reports_handler(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let world = state.world.read().await;
    if id != world.campaign.campaign_id {
        return missing_campaign();
    }
    Json(json!({"campaign":campaign(&world),"generation":generation(&world),"mode":"read_only","data":[]})).into_response()
}
async fn province_handler(
    State(state): State<AppState>,
    Path((id, prov_id)): Path<(String, u32)>,
) -> Response {
    let world = state.world.read().await;
    if id != world.campaign.campaign_id {
        return missing_campaign();
    }
    let pid = ProvinceId(prov_id);
    let Some(p) = world.provinces.iter().find(|p| p.id == pid) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error":"Province not found","code":"PROVINCE_NOT_FOUND"})),
        )
            .into_response();
    };
    let (start, end) = p.pop_range;
    let Some(pops) = world.pops.get(start as usize..end as usize) else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error":"Invalid POP range","code":"BROKEN_REFERENCE"})),
        )
            .into_response();
    };
    let enterprises: Vec<_> = world
        .enterprises
        .iter()
        .filter(|e| e.province_id == pid)
        .collect();
    let stockpiles: Vec<_> = world
        .stockpiles
        .iter()
        .filter(|s| s.province_id == pid)
        .collect();
    Json(json!({"campaign":campaign(&world),"generation":generation(&world),"mode":"read_only",
        "data":{"province_id":p.id.0,"name":p.name,"owner_state_id":p.owner_state_id,
            "controller_state_id":p.controller_state_id,"goods_id":p.goods_id,"pops":pops,"enterprises":enterprises,"stockpiles":stockpiles}})).into_response()
}
async fn execute_handler(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    // The measured CLI owns its private campaign; this read-only server has no commit workflow.
    let world = state.world.read().await;
    if id != world.campaign.campaign_id {
        return missing_campaign();
    }
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(
            json!({"campaign":campaign(&world),"generation":generation(&world),
        "error":"Для расчёта используйте терминальную программу прогона. Серверный commit пока не реализован; состояние и дата сохранены.",
        "code":"SERVER_WEEK_EXECUTION_NOT_IMPLEMENTED","native_ram_week_available":true,"native_checkpoint_available":true,"native_measured_runner_available":true,"missing_workflows":["server_isolated_campaign_commit"]}),
        ),
    )
        .into_response()
}
