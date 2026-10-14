// Review diagnostics: these assert economic contracts, and failures reproduce bugs.
// No source edits and no writes to the active campaign. All worlds here are tiny RAM fixtures.
use std::{collections::HashMap, sync::Arc};
use kingmaker_core::*;
use kingmaker_sim::{production::{execute_world_production, ProductionRecipe},
    ledger::{LedgerSession, TransferRequest}, needs::calculate_pop_demands_with_reserves,
    state_procurement::execute_state_raw_procurement, invariants::check_conservation,
    market::{MarketOrder, OrderPriority, clear_market_orders}, simulate_week};
use kingmaker_server::{state::AppState, routes::create_router};
use axum::{body::Body, http::{Request, StatusCode}};
use tokio::sync::RwLock;
use tower::ServiceExt;

fn province() -> ProvinceRecord {
    ProvinceRecord { id: ProvinceId(1), name: "Fixture".into(), owner_state_id: Some(ExternalId::from("1")),
        controller_state_id: Some(ExternalId::from("1")), pop_range: (0, 1), ..Default::default() }
}
fn pop() -> PopRecord {
    PopRecord { id: PopId(1), external_id:ExternalId::from(1u32), province_id: ProvinceId(1), pop_type: "farmers".into(),
        size: 100, culture: "fixture".into(), religion: "fixture".into(),
        account_id: Some(AccountId(1)), currency_id:Some(ExternalId::from("COIN")), workforce:60.0, unemployed:60.0,
        literacy: Some(0.1), militancy: Some(0.0), consciousness: Some(0.0), ..Default::default() }
}
fn stock(goods: Goods, qty: f64, owner: u64, purpose: &str) -> StockpileRecord {
    StockpileRecord { id:owner as u32, external_id:ExternalId::from(owner as u32), province_id: ProvinceId(1), goods, quantity: qty,
        purpose: purpose.into(), owner_id: ExternalId::from(owner as u32), ..Default::default() }
}
fn account(id:u32, owner:ExternalId, balance:i64) -> AccountBalance {
    AccountBalance {id:AccountId(id),external_id:ExternalId::from(format!("account:{id}").as_str()),owner_id:owner,
        currency_id:ExternalId::from("COIN"),purpose:if id==1{"personal_savings"}else{"operating_capital"}.into(),balance:Money(balance)}
}
fn world() -> WorldState {
    WorldState { campaign: CampaignHeader { campaign_id: "fixture".into(), revision: 20,
        year: 306, month: 1, day: 1, ..Default::default() }, provinces: vec![province()], pops: vec![pop()],
        accounts: vec![account(1,ExternalId::from(1u32),1000)], ..WorldState::default() }
}
fn factory() -> EnterpriseRecord {
    EnterpriseRecord { id: EnterpriseId(1), province_id: ProvinceId(1),
        recipe_id: Some(ExternalId::from("steel")), level: 1, account_id: Some(AccountId(1)), employees: Some(100.0),
        is_artisan: false, operating_capital: Some(0), ..Default::default() }
}
fn recipes() -> HashMap<String, ProductionRecipe> {
    HashMap::from([("steel".into(), ProductionRecipe { inputs: vec![(Goods::Iron, 10.0)],
        outputs: vec![(Goods::Steel, 5.0)], base_labor: 100 })])
}

#[test]
#[ignore = "R4 deferred: prototype output ignores input availability; not called by a native week"]
fn factory_cannot_produce_without_inputs() {
    let mut stocks = vec![stock(Goods::Steel, 0.0, 1, "commercial")];
    let result = execute_world_production(&[province()], &[factory()], &mut stocks, &recipes());
    assert_eq!(result.total_produced.get(&Goods::Steel).copied().unwrap_or(0.0), 0.0);
}
#[test]
#[ignore = "R4 deferred: prototype does not consume recipe inputs"]
fn factory_must_consume_recipe_inputs() {
    let mut stocks = vec![stock(Goods::Steel, 0.0, 1, "commercial"),
        stock(Goods::Iron, 10.0, 1, "commercial")];
    execute_world_production(&[province()], &[factory()], &mut stocks, &recipes());
    assert_eq!(stocks[1].quantity, 0.0);
}
#[test]
#[ignore = "R4 deferred: prototype reports output without creating a real owned stock"]
fn reported_production_must_reach_real_stock() {
    let mut stocks = vec![];
    let result = execute_world_production(&[province()], &[factory()], &mut stocks, &recipes());
    let real: f64 = stocks.iter().filter(|s| s.goods == Goods::Steel).map(|s| s.quantity).sum();
    assert_eq!(result.total_produced.get(&Goods::Steel).copied().unwrap_or(0.0), real);
}
#[test]
#[ignore = "R5 deferred: native week refuses; consumption not yet ported"]
fn food_consumption_must_reduce_real_stocks() {
    let mut w = world();
    w.stockpiles.push(stock(Goods::Grain, 100.0, 1, "food_reserve"));
    let _ = simulate_week(&mut w, &HashMap::new());
    assert!(w.stockpiles[0].quantity < 100.0, "Food was never eaten");
}
#[test]
#[ignore = "R5 deferred: prototype food demand does not resolve beneficiary rights"]
fn another_owners_reserve_must_not_hide_pop_food_demand() {
    let demands = calculate_pop_demands_with_reserves(&[pop()],
        &[stock(Goods::Grain, 100.0, 999, "food_reserve")]);
    assert!(demands.iter().any(|d| d.goods == Goods::Grain && d.quantity > 0.0),
        "POP silently uses another owner's food");
}
#[test]
#[ignore = "R5 deferred: prototype scalar supply is not a commodity-aware paid market"]
fn grain_supply_cannot_fulfill_cloth_orders() {
    // Exactly the pipeline's supply: total_grain, regardless of order.goods.
    let mut orders = [MarketOrder { order_id: 1, buyer_province: ProvinceId(1),
        goods: Goods::Cloth, requested_qty: 5.0, priority: OrderPriority::Everyday,
        allocated_qty: 0.0 }];
    clear_market_orders(&mut orders, 100.0);
    assert_eq!(orders[0].allocated_qty, 0.0);
}
#[test]
fn self_transfer_must_not_create_money() {
    let a=account(1,ExternalId::from(1u32),100);
    let mut ledger=LedgerSession::from_accounts(&[a.clone()]).unwrap();
    ledger.transfer_once(TransferRequest{event_id:"self".into(),from:a.address(),to:a.address(),amount:Money(50),reason:"self".into()}).unwrap();
    assert_eq!(ledger.get_balance(a.id),Some(Money(100)));
}
#[test]
fn negative_transfer_must_be_rejected() {
    let a=account(1,ExternalId::from(1u32),100); let b=account(2,ExternalId::from(2u32),100);
    let mut ledger=LedgerSession::from_accounts(&[a.clone(),b.clone()]).unwrap();
    assert!(ledger.transfer_once(TransferRequest{event_id:"negative".into(),from:a.address(),to:b.address(),amount:Money(-50),reason:"negative".into()}).is_err());
}
#[test]
fn unknown_payee_must_be_rejected() {
    let a=account(1,ExternalId::from(1u32),100); let unknown=account(999,ExternalId::from(999u32),0);
    let mut ledger=LedgerSession::from_accounts(&[a.clone()]).unwrap();
    assert!(ledger.transfer_once(TransferRequest{event_id:"unknown".into(),from:a.address(),to:unknown.address(),amount:Money(50),reason:"unknown".into()}).is_err());
}
#[test]
#[ignore = "R6 deferred: prototype state purchase debits treasury but omits seller credit"]
fn procurement_must_pay_the_seller_and_conserve_cash() {
    let mut w = world();
    w.accounts[0].balance = Money(0);
    w.accounts.push(account(2,ExternalId::from("treasury:1"),10000));
    w.stockpiles.push(stock(Goods::Iron, 100.0, 1, "commercial"));
    let before = w.clone();
    let result = execute_state_raw_procurement(&w.provinces, &mut w.stockpiles, &mut w.accounts);
    assert_eq!(result.total_cost_minor, 5000); // Confirms the purchase actually triggered.
    assert!(matches!(check_conservation(&before, &w).money,kingmaker_sim::invariants::CheckStatus::Passed),
        "Treasury debit has no corresponding seller credit");
}
#[test]
#[ignore = "R6 deferred: prototype purchase does not transfer legal stock owner"]
fn purchased_stock_must_leave_the_seller_ownership() {
    let mut stocks = vec![stock(Goods::Iron, 100.0, 999, "commercial")];
    let mut accounts = vec![account(2,ExternalId::from("treasury:1"),10000)];
    execute_state_raw_procurement(&[province()], &mut stocks, &mut accounts);
    assert_ne!(stocks[0].owner_id, ExternalId::from(999u32));
}
#[test]
fn invariant_must_detect_unexplained_goods_creation() {
    let before = world();
    let mut after = before.clone();
    after.stockpiles.push(stock(Goods::Grain, 1000.0, 1, "commercial"));
    assert!(matches!(check_conservation(&before, &after).goods,kingmaker_sim::invariants::CheckStatus::Failed{..}));
}
#[test]
fn invariant_must_check_people_not_just_pop_row_count() {
    let before = world();
    let mut after = before.clone();
    after.pops[0].size = 0;
    assert!(matches!(check_conservation(&before, &after).people,kingmaker_sim::invariants::CheckStatus::Failed{..}));
}
fn api_state() -> AppState {
    AppState { world: Arc::new(RwLock::new(world())), recipes: Arc::new(HashMap::new()) }
}
#[tokio::test]
async fn viewer_bootstrap_requires_campaign_list() {
    let response = create_router(api_state()).oneshot(Request::builder()
        .uri("/api/campaigns").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}
#[tokio::test]
#[ignore = "R9 deferred: full viewer reports API not ported; no fake success response"]
async fn viewer_campaign_load_requires_reports() {
    let response = create_router(api_state()).oneshot(Request::builder()
        .uri("/api/campaign/fixture/reports?revision=20")
        .body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}
#[tokio::test]
async fn wrong_campaign_must_not_execute_active_world() {
    let state = api_state();
    let response = create_router(state.clone()).oneshot(Request::builder().method("POST")
        .uri("/api/campaign/nonexistent/execute").body(Body::empty()).unwrap())
        .await.unwrap();
    let revision = state.world.read().await.campaign.revision;
    assert_eq!(revision, 20, "Invalid campaign unexpectedly advanced the active world");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
#[test]
fn calendar_fixture_crosses_year_with_thirty_day_months() {
    let mut w = world();
    w.campaign.month = 12;
    w.campaign.day = 27;
    assert_eq!(kingmaker_sim::pipeline::advance_week_date(w.campaign.year,w.campaign.month,w.campaign.day).unwrap(),(307,1,4));
    assert!(simulate_week(&mut w, &HashMap::new()).is_err());
    assert_eq!((w.campaign.year,w.campaign.month,w.campaign.day),(306,12,27));
}

// Tiny isolated SQLite fixture in the diagnostics directory, never the active DB.
fn sqlite_fixture(name: &str) -> std::path::PathBuf {
    let f=common::Fixture::new();
    let path=f.path.with_extension(format!("{name}.sqlite"));
    std::fs::copy(&f.path,&path).unwrap();
    path
}
#[path="../../../../crates/kingmaker-io/tests/common/mod.rs"]
mod common;
#[test]
fn sqlite_loader_must_preserve_encoded_campaign_date() {
    let path = sqlite_fixture("campaign-date-fixture");
    let w = kingmaker_io::SqliteLoader::load_from_path(&path, "fixture").unwrap();
    std::fs::remove_file(path).unwrap();
    assert_eq!(w.campaign.revision, 123);
    assert_eq!((w.campaign.year, w.campaign.month, w.campaign.day), (307, 4, 12));
}
#[test]
fn sqlite_loader_must_reject_nonexistent_campaign() {
    let path = sqlite_fixture("missing-campaign-fixture");
    assert!(kingmaker_io::SqliteLoader::load_from_path(&path, "missing").is_err(),
        "A fabricated revision-20 empty world must not count as a successfully loaded campaign");
    std::fs::remove_file(path).unwrap();
}

