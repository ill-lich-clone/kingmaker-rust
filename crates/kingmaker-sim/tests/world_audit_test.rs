#[path = "common/labor.rs"]
mod labor;
#[path = "common/native_week.rs"]
mod week;
use kingmaker_core::*;
use kingmaker_sim::world_audit::audit_initial_world;
use serde_json::json;

fn dormant_world() -> WorldState {
    let mut w = week::empty();
    w.provinces.push(ProvinceRecord {
        id: ProvinceId(1),
        goods_id: Some(Goods::Grain),
        ..Default::default()
    });
    let catalog = w.production_data.catalog.as_mut().unwrap();
    catalog.goods.push(
        serde_json::from_value(json!({"goods_id":"grain","kind":"raw_resource","unit":"t"}))
            .unwrap(),
    );
    catalog.methods.push(
        serde_json::from_value(
            json!({"method_id":"rgo:grain","kind":"rgo","status":"ready",
        "output_goods_id":"grain","output_unit":"t","weekly_productivity":1,"staffing":["farmers"],
        "inputs":[],"capacity":{"base_people":100,"source":"explicit miniature"}}),
        )
        .unwrap(),
    );
    w.production_data.resource_operations.push(
        serde_json::from_value(json!({"id":"rgo:1","province_id":1,
        "owner_id":null,"method_id":"rgo:grain","resource_id":"grain","resource_status":"source",
        "land_access":false,"initial_multiplier":1,"assignments":[]}))
        .unwrap(),
    );
    w
}

#[test]
fn explicitly_empty_sea_does_not_require_an_invented_rgo() {
    for kind in ["sea", "lake"] {
        let mut w = dormant_world();
        // Deliberately outside the real map's numeric sea range.
        w.provinces.push(ProvinceRecord {
            id: ProvinceId(2),
            name: "any author name".into(),
            ..Default::default()
        });
        w.transport_data
            .navigation_geography
            .push(json!({"province_id":2,"kind":kind}));
        let before = serde_json::to_value(&w).unwrap();
        assert!(
            audit_initial_world(&w).can_run(),
            "{:?}",
            audit_initial_world(&w).issues
        );
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
    }
}

#[test]
fn missing_rgo_on_land_unknown_or_economically_used_sea_still_fails() {
    for case in 0..3 {
        let mut w = dormant_world();
        w.provinces.push(ProvinceRecord {
            id: ProvinceId(2),
            name: "Морская провинция".into(),
            ..Default::default()
        });
        if case == 0 {
            w.transport_data
                .navigation_geography
                .push(json!({"province_id":2,"kind":"land"}));
        }
        if case == 2 {
            w.transport_data
                .navigation_geography
                .push(json!({"province_id":2,"kind":"sea"}));
            w.provinces[1].goods_id = Some(Goods::Grain);
        }
        assert!(
            audit_initial_world(&w)
                .issues
                .iter()
                .any(|r| r.code == "raw_province_count" && r.province_id == Some(ProvinceId(2))),
            "case{case}"
        );
    }
}

#[test]
fn conflicting_author_geography_is_rejected() {
    let mut w = dormant_world();
    w.market_data
        .territory
        .push(serde_json::from_value(json!({"province_id":1,"kind":"land"})).unwrap());
    w.transport_data
        .navigation_geography
        .push(json!({"province_id":1,"kind":"sea"}));
    assert!(audit_initial_world(&w)
        .issues
        .iter()
        .any(|r| r.code == "province_kind_conflict" && r.province_id == Some(ProvinceId(1))));
}

#[test]
fn only_genuinely_unowned_closed_rgo_without_people_or_positive_assets_is_admitted() {
    let w = dormant_world();
    assert!(
        audit_initial_world(&w).can_run(),
        "{:?}",
        audit_initial_world(&w).issues
    );
    for case in 0..11 {
        let mut w = dormant_world();
        match case {
            0=>w.production_data.resource_operations[0].land_access=Some(true),
            1=>w.production_data.resource_operations[0].base.owner_id=Some("unknown-owner".into()),
            2=>w.provinces[0].owner_state_id=Some("claimed".into()),
            3=>w.pops.push(PopRecord {id:PopId(1),external_id:"person".into(),province_id:ProvinceId(1),size:1,..Default::default()}),
            4=>w.accounts.push(AccountBalance {id:AccountId(1),external_id:"cash".into(),owner_id:"rgo:1".into(),currency_id:"X".into(),purpose:"operating_capital".into(),balance:Money(1)}),
            5=>w.stockpiles.push(StockpileRecord {id:1,external_id:"stock".into(),province_id:ProvinceId(1),owner_id:"rgo:1".into(),quantity:1.,..Default::default()}),
            6=>w.production_data.obligations.push(serde_json::from_value(json!({"obligation_id":"debt","payer_id":"rgo:1","payee_id":"creditor","currency_id":"X","purpose":"wages","amount_minor":"1"})).unwrap()),
            7=>w.production_data.money_reservations.push(serde_json::from_value(json!({"id":"reserve","owner_id":"rgo:1","currency_id":"X","purpose":"operating_capital","amount_minor":"1"})).unwrap()),
            8=>w.production_data.storage_reservations.push(serde_json::from_value(json!({"id":"held","owner_id":"rgo:1","facility_id":"warehouse","goods_id":"grain","unit":"t","status":"held","quantity":1})).unwrap()),
            9=>{w.production_data.resource_operations[0].base.extra.insert("shares".into(),json!([{"owner_id":"claimant","numerator":1,"denominator":1}]));},
            _=>{
                w.production_data.contracts.push(serde_json::from_value(json!({"contract_id":"remote-contract","employer_entity_id":"rgo:1","currency_id":"X"})).unwrap());
                w.budget_data.employment.push(serde_json::from_value(json!({"employment_id":"remote-worker","pop_id":"remote","contract_id":"remote-contract","job_id":"remote-job","workers_count":1,"time_fraction":1,"fte":1})).unwrap());
            },
        }
        assert!(
            audit_initial_world(&w)
                .issues
                .iter()
                .any(|r| r.code == "producer_owner_unresolved" && r.id == Some("rgo:1".into())),
            "case{case}: {:?}",
            audit_initial_world(&w).issues
        );
    }
}

#[test]
fn factories_cannot_inherit_the_dormant_rgo_owner_exception() {
    let mut w = dormant_world();
    let mut factory = week::factory(false).production_data.factories.remove(0);
    factory.base.id = "unowned-factory".into();
    factory.base.owner_id = None;
    factory.base.assignments.clear();
    w.production_data.factories.push(factory);
    assert!(audit_initial_world(&w)
        .issues
        .iter()
        .any(|r| r.code == "producer_owner_unresolved" && r.id == Some("unowned-factory".into())));
}

#[test]
fn actual_native_week_keeps_unowned_closed_rgo_inactive_without_minting_anything() {
    let mut w = dormant_world();
    let request = week::request(&w);
    kingmaker_sim::native_week::execute_native_week(&mut w, &request, |_| {}).unwrap();
    assert!(w.pops.is_empty());
    assert!(w.accounts.iter().all(|a| a.balance == Money::ZERO));
    assert!(w.stockpiles.iter().all(|s| s.quantity == 0.));
    assert!(w
        .production_data
        .phase_receipts
        .iter()
        .flat_map(|r| &r.outputs)
        .all(|o| o.quantity == 0. && o.fte == 0.));
    assert!(w.production_data.resource_operations[0]
        .base
        .owner_id
        .is_none());
    assert_eq!(w.campaign.revision, 21);
}

#[test]
fn actual_income_phase_still_refuses_an_unowned_rgo_with_cash_without_partial_effects() {
    let mut w = dormant_world();
    w.accounts.push(AccountBalance {
        id: AccountId(1),
        external_id: "cash".into(),
        owner_id: "rgo:1".into(),
        currency_id: "X".into(),
        purpose: "operating_capital".into(),
        balance: Money(1),
    });
    w.rebuild_account_index().unwrap();
    let request = week::request(&w);
    let before = serde_json::to_value(&w).unwrap();
    let error =
        kingmaker_sim::native_week::execute_native_week(&mut w, &request, |_| {}).unwrap_err();
    assert!(
        error.to_string().contains("Reserve owner missing"),
        "{error}"
    );
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}

fn world() -> WorldState {
    let mut w = labor::world(100000);
    let prototype = w.production_data.resource_operations[0].clone();
    w.production_data.resource_operations.clear();
    for p in &mut w.provinces {
        p.goods_id = Some(Goods::Grain);
        let mut r = prototype.clone();
        r.base.id = format!("rgo:{}", p.id.0).into();
        r.base.province_id = p.id;
        r.base.facility_id = None;
        w.production_data.resource_operations.push(r);
    }
    w
}
#[test]
fn valid_partial_economy_is_readonly_and_missing_measurement_is_explicit() {
    let w = world();
    let before = serde_json::to_value(&w).unwrap();
    let audit = audit_initial_world(&w);
    assert!(audit.can_run(), "{:?}", audit.issues);
    assert!(audit.not_measured.iter().any(|s| s.contains("property")));
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
#[test]
fn every_province_has_one_matching_raw_resource_and_only_raw_labor_recipe() {
    for case in 0..5 {
        let mut w = world();
        match case {
            0 => {
                w.production_data.resource_operations.pop();
            }
            1 => w
                .production_data
                .resource_operations
                .push(w.production_data.resource_operations[0].clone()),
            2 => w.production_data.resource_operations[0].resource_id = "salt".into(),
            3 => w.production_data.catalog.as_mut().unwrap().methods[0]
                .inputs
                .push(
                    serde_json::from_value(serde_json::json!({"goods_id":"grain","amount":1}))
                        .unwrap(),
                ),
            _ => w.provinces[0].goods_id = Some(Goods::Steel),
        }
        let audit = audit_initial_world(&w);
        assert!(!audit.can_run(), "case{case}");
        assert!(
            audit
                .issues
                .iter()
                .any(|e| e.code.starts_with("raw_") && e.province_id.is_some()),
            "{:?}",
            audit.issues
        );
    }
}
#[test]
fn negative_cash_nan_stock_and_unresolved_producer_owner_have_specific_ids() {
    for case in 0..3 {
        let mut w = world();
        let (code, id) = match case {
            0 => {
                w.accounts[0].balance = Money(-1);
                ("negative_cash", w.accounts[0].external_id.clone())
            }
            1 => {
                w.stockpiles[0].quantity = f64::NAN;
                (
                    "invalid_stock_quantity",
                    w.stockpiles[0].external_id.clone(),
                )
            }
            _ => {
                w.production_data.resource_operations[0].base.owner_id =
                    Some("nonexistent-owner".into());
                (
                    "producer_owner_unresolved",
                    w.production_data.resource_operations[0].base.id.clone(),
                )
            }
        };
        let audit = audit_initial_world(&w);
        assert!(!audit.can_run());
        assert!(
            audit
                .issues
                .iter()
                .any(|e| e.code == code && e.id.as_ref() == Some(&id)),
            "{:?}",
            audit.issues
        );
    }
}
#[test]
fn double_labor_and_more_than_eight_regional_factories_are_reported() {
    let mut w = week::factory(false);
    let mut other = w.production_data.factories[0].clone();
    other.base.id = "second-factory".into();
    other.base.assignments[0].employment_id = Some("second-employment".into());
    w.production_data.factories.push(other);
    let audit = audit_initial_world(&w);
    assert!(
        audit.issues.iter().any(|e| e.code == "labor_roster"),
        "{:?}",
        audit.issues
    );
    let mut w = week::factory(false);
    let template = w.production_data.factories[0].clone();
    w.production_data.factories.clear();
    for n in 0..9 {
        let mut f = template.clone();
        f.base.id = format!("factory:{n}").into();
        f.region_id = Some("r".into());
        f.base.assignments.clear();
        w.production_data.factories.push(f);
    }
    assert!(audit_initial_world(&w)
        .issues
        .iter()
        .any(|e| e.code == "factory_region_limit"));
}

#[test]
fn producer_capacity_is_not_hidden_by_free_workforce() {
    let mut w = week::factory(false);
    w.pops[0].size = 4;
    w.pops[0].workforce = 2.;
    w.production_data.factories[0].people_slots = Some(1.);
    w.production_data.factories[0].base.assignments[0].workers_count = 2.;
    let audit = audit_initial_world(&w);
    assert!(
        audit.issues.iter().any(|e| e.code == "producer_capacity"),
        "{:?}",
        audit.issues
    );
}
#[test]
fn invalid_held_warehouse_reservation_refuses_initial_state() {
    let mut w = week::factory(false);
    let f = &w.production_data.storage_facilities[0];
    w.production_data.storage_reservations.push(serde_json::from_value(serde_json::json!({"id":"invalid-held","owner_id":f.owner_id,"facility_id":f.id,"goods_id":"steel","unit":"t","status":"held","quantity":-1})).unwrap());
    assert!(audit_initial_world(&w)
        .issues
        .iter()
        .any(|e| e.code == "invalid_storage_reservation"));
}
