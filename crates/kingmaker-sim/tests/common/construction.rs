#![allow(dead_code)]
#[path = "labor.rs"]
mod labor;
#[path = "production.rs"]
mod production;
use kingmaker_core::{construction::ConstructionRequest, *};
use serde_json::json;
pub fn request() -> ConstructionRequest {
    ConstructionRequest {
        expected_revision: 20,
        period: "w".into(),
        policies: Default::default(),
    }
}
/// Explicit miniature author world. Its stocks/capital are the measured initial state.
pub fn opening(duration: u32) -> WorldState {
    let mut w = production::world();
    w.production_data.factories.clear();
    w.production_data.contracts.clear();
    w.pops[0].pop_type_id = "farmers".into();
    w.pops[0].pop_type = "farmers".into();
    w.pops[0].employed = 0.;
    w.pops[0]
        .extra
        .insert("species_labor_coefficient".into(), json!(1));
    let mut craft = w.pops[0].clone();
    craft.id = PopId(2);
    craft.external_id = "craft".into();
    craft.pop_type_id = "craftsmen".into();
    craft.pop_type = "craftsmen".into();
    w.pops.push(craft);
    for (ordinal, id) in [(3, "a"), (4, "b")] {
        let mut p = w.pops[0].clone();
        p.id = PopId(ordinal);
        p.external_id = id.into();
        p.pop_type_id = "capitalists".into();
        p.pop_type = "capitalists".into();
        p.workforce = 0.;
        w.pops.push(p);
    }
    w.accounts
        .push(production::account(3, "a", "personal_savings", 3000));
    w.accounts
        .push(production::account(4, "b", "personal_savings", 7000));
    w.rebuild_account_index().unwrap();
    let market = labor::world(0).market_data;
    w.market_data = market;
    w.market_data.territory.truncate(1);
    w.market_data.territory[0].region_id = Some("r".into());
    let needs = w.market_data.needs.as_mut().unwrap();
    needs.profile_refs[0].pop_id = "craft".into();
    w.market_data
        .price_calibration
        .as_mut()
        .unwrap()
        .prices_by_currency
        .get_mut("COIN")
        .unwrap()
        .extend([
            ("iron".into(), json!(1)),
            ("coal".into(), json!(1)),
            ("steel".into(), json!(3)),
        ]);
    let owner = "project:private-project:offer";
    let enterprise = "enterprise:private-project:offer";
    let c = json!({"status":"ready","source":"fixture","cost_minor":"1000","currency_id":"COIN","duration_days":duration,"labor_required_fte_days":7,"cash_cost_basis":"includes_wages","provider_id":"builder","materials":[{"goods_id":"iron","amount":1}]});
    w.production_data.catalog.as_mut().unwrap().methods[0]
        .extra
        .insert("construction".into(), c.clone());
    w.production_data.contracts.push(serde_json::from_value(json!({"contract_id":"build-c","employer_entity_id":owner,"currency_id":"COIN","payment_purpose":"construction_escrow","weekly_wage_rate":{"numerator":"10","denominator":"1"}})).unwrap());
    w.production_data.policies.taxes.insert("s".into(), 0.1);
    w.mobility_data.capitalist_entities = vec![
        json!({"id":"a","owner_id":"a","pop_id":"a","currency_id":"COIN","account_purpose":"personal_savings","implemented_technologies":[]}),
        json!({"id":"b","owner_id":"b","pop_id":"b","currency_id":"COIN","account_purpose":"personal_savings","implemented_technologies":[]}),
    ];
    w.construction_data.investment_opportunities = vec![
        json!({"id":"offer","status":"source","source":"fixture","method_id":"factory:steel","province_id":1,"currency_id":"COIN","demand":10,"margin_minor":"100","workforce_available":1,"inputs_accessible":true,"terms":{"version":1,"source":"fixture","budget_minor":"10000","operating_reserve_minor":"4000","currency_id":"COIN","share_basis":"cash_contribution","refund_rule":"pro_rata_unspent"},"assignments":[{"employment_id":"builder-e","contract_id":"build-c","pop_id":"p","workers_count":1,"time_fraction":1}],"future_assignments":[],"factory_configuration":{"facility_id":"new-wh","people_slots":40000,"capacity_source":"fixture","equipment_capacity_weekly":100,"equipment_source":"fixture","parameters":{"source":"fixture","status":"source","tenure_Bmax":0,"tenure_Tsat":30},"implemented_technologies":[]}}),
    ];
    w.production_data.storage_facilities[0].id = "new-wh".into();
    w.production_data.storage_facilities[0].owner_id = enterprise.into();
    w.stockpiles[0].owner_id = owner.into();
    w.stockpiles[0].quantity = 1.;
    w.stockpiles[1].quantity = 0.;
    for (ordinal, goods, quantity) in [(20, Goods::Iron, 1.1), (21, Goods::Coal, 0.4)] {
        let mut s = w.stockpiles[0].clone();
        s.id = ordinal;
        s.external_id = format!("factory-input:{ordinal}").into();
        s.owner_id = enterprise.into();
        s.goods = goods;
        s.quantity = quantity;
        w.stockpiles.push(s);
    }
    for s in &mut w.stockpiles {
        s.currency_id = Some("COIN".into());
    }
    w.refresh_enterprise_views().unwrap();
    w
}
pub fn ready(duration: u32) -> WorldState {
    ready_with(duration, |_| {})
}
pub fn ready_with(duration: u32, configure: impl FnOnce(&mut WorldState)) -> WorldState {
    let mut w = opening(duration);
    configure(&mut w);
    let r = request();
    kingmaker_sim::budget_cycle::execute_budget_cycle(
        &mut w,
        &kingmaker_core::budget::BudgetRequest {
            expected_revision: 20,
            period: r.period.clone(),
            policies: Default::default(),
        },
    )
    .unwrap();
    kingmaker_sim::investments::execute_automatic_investments(&mut w, &r).unwrap();
    kingmaker_sim::construction_payroll::execute_construction_payroll(&mut w, &r).unwrap();
    kingmaker_sim::production::execute_production_cycle(&mut w, &production::request()).unwrap();
    w
}
pub fn cash(w: &WorldState, owner: &str, purpose: &str) -> i64 {
    w.accounts
        .iter()
        .filter(|a| a.owner_id == ExternalId::from(owner) && a.purpose == purpose)
        .map(|a| a.balance.0)
        .sum()
}
