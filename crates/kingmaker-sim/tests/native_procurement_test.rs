#[path = "common/budget.rs"]
mod budget;
#[path = "common/income.rs"]
mod income_common;
#[path = "common/physical_supply.rs"]
mod physical;
use kingmaker_core::*;
use kingmaker_sim::raw_procurement::{
    apply_raw_procurement, execute_raw_procurement, free_procurement_budget,
    prepare_raw_procurement,
};
use serde_json::json;
#[test]
fn distinct_nonfood_orders_allocate_incoming_goods_only_once() {
    let mut w = physical::ready_with(|w| {
        let norms = &mut w.market_data.needs.as_mut().unwrap().profiles[0].norms;
        let mut second = norms[0].clone();
        second.need_id = "tools".into();
        norms.push(second);
    });
    let receipt =
        kingmaker_sim::physical_supply::execute_physical_supply(&mut w, &physical::request())
            .unwrap();
    assert_eq!(w.market_data.phase_receipts.last().unwrap().orders.len(), 2);
    assert_eq!(
        w.mobility_data
            .shipments
            .iter()
            .map(|s| s["remaining_quantity"].as_f64().unwrap())
            .sum::<f64>(),
        2.
    );
    assert_eq!(receipt.decisions.len(), 2);
    assert_eq!(w.stockpiles[0].quantity, 3.);
}
#[test]
fn existing_pipeline_is_consumed_once_and_keeps_destination_title() {
    let mut prior = physical::ready();
    kingmaker_sim::physical_supply::execute_physical_supply(&mut prior, &physical::request())
        .unwrap();
    let cargo = prior.mobility_data.shipments[0].clone();
    for other_destination in [false, true] {
        let mut w = physical::ready_with(|w| {
            let norms = &mut w.market_data.needs.as_mut().unwrap().profiles[0].norms;
            norms[0].need_id = "hardware".into();
            let mut second = norms[0].clone();
            second.need_id = "tools".into();
            norms.push(second);
            let mut cargo = cargo.clone();
            if other_destination {
                cargo["facility_id"] = json!("another-warehouse");
            }
            w.mobility_data.shipments.push(cargo);
        });
        // Existing title covers one matching request; another destination cannot cover either need.
        let receipt =
            kingmaker_sim::physical_supply::execute_physical_supply(&mut w, &physical::request())
                .unwrap();
        let unfulfilled = receipt
            .diagnostics
            .iter()
            .filter_map(|d| d["unfulfilled_quantity"].as_f64())
            .sum::<f64>();
        let dispatched = receipt
            .decisions
            .iter()
            .filter_map(|d| d["quantity"].as_f64())
            .sum::<f64>();
        assert_eq!(
            dispatched + unfulfilled,
            if other_destination { 2. } else { 1. },
            "cargo {:?}; orders {:?}; decisions {:?}",
            cargo,
            w.market_data.phase_receipts.last().unwrap().orders,
            receipt.decisions
        );
        assert_eq!(w.stockpiles[0].quantity, 5. - dispatched);
    }
}
#[test]
fn procurement_requires_matching_sale_title_and_payment_terms() {
    for case in 0..5 {
        let mut w = ready(1000);
        match case {
            0 => w.market_data.sale_offers[0].seller_id = "not-the-owner".into(),
            1 => w.market_data.sale_offers[0].goods_id = "steel".into(),
            2 => w.market_data.sale_offers[0].seller_purpose = None,
            3 => {
                w.market_data.sale_offers[0]
                    .extra
                    .insert("unit".into(), json!("litres"));
            }
            _ => {
                w.market_data.sale_offers[0]
                    .extra
                    .insert("currency_id".into(), json!("OTHER"));
            }
        }
        let request = req(&w);
        let accounts = w.accounts.clone();
        let stocks = w.stockpiles.clone();
        let receipt = execute_raw_procurement(&mut w, &request).unwrap();
        assert_eq!(w.accounts, accounts, "case {case}");
        assert_eq!(w.stockpiles, stocks, "case {case}");
        assert!(!receipt.diagnostics.is_empty(), "case {case}");
    }
}
#[test]
fn full_cohort_transfer_preserves_existing_approved_recovery_in_scalar_and_scope() {
    use kingmaker_core::{mobility::*, transport::*};
    for scoped in [false, true] {
        let mut w = physical::ready();
        kingmaker_sim::physical_supply::execute_physical_supply(&mut w, &physical::request())
            .unwrap();
        let cargo = ExternalId::from_json(w.mobility_data.shipments[0]["id"].clone()).unwrap();
        let interrupt = ShipmentActionRequest {
            expected_revision: 20,
            event_id: "real-interrupt".into(),
            shipment_id: cargo.clone(),
            actor_id: None,
            action: ShipmentAction::Interrupt {
                reason: "closed road".into(),
            },
        };
        kingmaker_sim::shipment_actions::execute_shipment_action(&mut w, &interrupt).unwrap();
        w.transport_data.shipment_recovery_plans.push(json!({"id":"real-plan","shipment_id":cargo,"action":"resume","status":"source","source":"author","accepted_buyer_id":"a","accepted_carrier_id":"carrier","currency_id":"COIN","remaining_travel_days":4,"extra_freight_minor":"0","author_formula":"keep"}));
        let split = CohortSplitRequest {
            expected_revision: 20,
            event_id: "real-split".into(),
            pop_id: "a".into(),
            workforce_count: 600,
            target: SplitTarget::default(),
        };
        let result = if scoped {
            let (selected, receipt) = kingmaker_sim::cohort_scope::CohortScope::new(&w)
                .unwrap()
                .preview(&w, &split)
                .unwrap();
            assert_eq!(
                selected.transport_data.shipment_recovery_plans[0]["accepted_buyer_id"],
                receipt.child_id.as_ref().unwrap().json()
            );
            let actual = kingmaker_sim::cohort_cycle::execute_pop_split(&mut w, &split).unwrap();
            assert_eq!(actual, receipt);
            actual
        } else {
            kingmaker_sim::cohort_cycle::execute_pop_split(&mut w, &split).unwrap()
        };
        let child = result.child_id.unwrap();
        assert_eq!(
            w.transport_data.shipment_recovery_plans[0]["accepted_buyer_id"],
            child.json()
        );
        assert_eq!(
            w.transport_data.shipment_recovery_plans[0]["author_formula"],
            json!("keep")
        );
        let resume = ShipmentActionRequest {
            expected_revision: 20,
            event_id: "real-resume".into(),
            shipment_id: cargo,
            actor_id: Some(child),
            action: ShipmentAction::Resume {
                recovery_id: "real-plan".into(),
            },
        };
        kingmaker_sim::shipment_actions::execute_shipment_action(&mut w, &resume).unwrap();
    }
}
#[test]
fn actual_paid_recovery_is_authenticated_in_income_exactly_once() {
    use kingmaker_core::transport::*;
    let (mut w, basis, _) = income_common::fixture_configure(false, |w| {
        w.mobility_data.shipments.push(json!({"id":"prior-cargo","from_province_id":2,"to_province_id":1,"buyer_id":"o","seller_id":"vendor","carrier_id":"carrier","buyer_purpose":"employer_operating_capital","seller_purpose":"operating_capital","currency_id":"COIN","goods_id":"iron","goods_price_minor":"0","freight_paid_minor":"0","status":"in_transit","remaining_quantity":1,"shipped_quantity":1,"space_per_unit":1,"asset_id":"cart","return_days":2,"arrival_date":{"year":306,"month":2,"day":1},"carrier_release_date":{"year":306,"month":2,"day":3},"storage_reservation_id":"prior-reservation"}));
        w.transport_data.shipment_recovery_plans.push(json!({"id":"recover","shipment_id":"prior-cargo","action":"resume","status":"source","source":"author","accepted_buyer_id":"o","accepted_carrier_id":"carrier","currency_id":"COIN","remaining_travel_days":4,"extra_freight_minor":"2"}));
    });
    let cost = |w: &WorldState| {
        kingmaker_sim::income_recognition::recognize_producer_income(
            w,
            &income_common::request(),
            &basis,
        )
        .map(|rows| {
            rows.iter()
                .find(|r| r.owner_id == ExternalId::from("o"))
                .unwrap()
                .recognized_cost_minor
                .unwrap()
                .0
        })
    };
    let before = cost(&w).unwrap();
    let interrupt = ShipmentActionRequest {
        expected_revision: 20,
        event_id: "recover-interrupt".into(),
        shipment_id: "prior-cargo".into(),
        actor_id: None,
        action: ShipmentAction::Interrupt {
            reason: "closed road".into(),
        },
    };
    kingmaker_sim::shipment_actions::execute_shipment_action(&mut w, &interrupt).unwrap();
    let recovery = ShipmentActionRequest {
        expected_revision: 20,
        event_id: "recover-paid".into(),
        shipment_id: "prior-cargo".into(),
        actor_id: Some("o".into()),
        action: ShipmentAction::Resume {
            recovery_id: "recover".into(),
        },
    };
    kingmaker_sim::shipment_actions::execute_shipment_action(&mut w, &recovery).unwrap();
    assert_eq!(cost(&w).unwrap(), before + 2);
    kingmaker_sim::shipment_actions::execute_shipment_action(&mut w, &recovery).unwrap();
    assert_eq!(cost(&w).unwrap(), before + 2);
    for case in 0..3 {
        let mut bad = w.clone();
        let position = bad
            .transport_data
            .phase_receipts
            .iter()
            .position(|p| p.request.stage == TransportStage::Recovery)
            .unwrap();
        match case {
            0 => {
                bad.transport_data.phase_receipts.remove(position);
            }
            1 => {
                bad.transport_data.phase_receipts[position].registry_changes =
                    json!({"tampered":true})
            }
            _ => bad.transport_data.phase_receipts[position].date.day += 1,
        }
        assert!(cost(&bad).is_err(), "case {case}");
    }
}
#[test]
fn raw_budget_protects_actual_two_week_service_shortfalls_and_state_debts() {
    let mut w = budget::world(4500);
    w.accounts.push(budget::account(
        11,
        "s",
        "X",
        "state_service:education",
        1000,
    ));
    w.rebuild_account_index().unwrap();
    w.production_data.obligations=serde_json::from_value(json!([{"obligation_id":"ordinary","payer_id":"treasury:s","payee_id":"creditor","currency_id":"X","purpose":"operating_capital","amount_minor":"600"},{"obligation_id":"other-currency","payer_id":"s","payee_id":"other","currency_id":"Y","purpose":"operating_capital","amount_minor":"20000"}])).unwrap();
    let before = serde_json::to_value(&w).unwrap();
    assert_eq!(
        free_procurement_budget(&w, &"s".into(), &"X".into(), &w.production_data.policies).unwrap(),
        Money(900)
    );
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
    w.production_data.obligations.push(serde_json::from_value(json!({"obligation_id":"service-debt","payer_id":"s","payee_id":"teacher","currency_id":"X","purpose":"state_service:education","amount_minor":"500","budget_state_id":"s"})).unwrap());
    assert_eq!(
        free_procurement_budget(&w, &"s".into(), &"X".into(), &w.production_data.policies).unwrap(),
        Money(400)
    );
    w.accounts[0].balance = Money(4000);
    assert_eq!(
        free_procurement_budget(&w, &"s".into(), &"X".into(), &w.production_data.policies).unwrap(),
        Money(0)
    );
}
#[test]
fn procurement_protection_uses_actual_service_funding_sliders() {
    let mut w = budget::world(4500);
    budget::funding(&mut w, 50., 0.);
    assert_eq!(
        free_procurement_budget(&w, &"s".into(), &"X".into(), &w.production_data.policies).unwrap(),
        Money(3500)
    );
    budget::funding(&mut w, 101., 0.);
    assert!(
        free_procurement_budget(&w, &"s".into(), &"X".into(), &w.production_data.policies).is_err()
    );
}
fn ready(cash: i64) -> WorldState {
    let mut w = physical::ready_with(|w| {
        for p in &mut w.market_data.territory {
            p.extra.insert("owner_state_id".into(), json!("state:s"));
        }
        w.production_data.catalog.as_mut().unwrap().methods.push(serde_json::from_value(json!({"method_id":"raw-iron","kind":"rgo","output_goods_id":"iron","output_unit":"t","status":"ready","weekly_productivity":1,"inputs":[],"staffing":["laborers"]})).unwrap());
        w.production_data.resource_operations.push(serde_json::from_value(json!({"id":"raw","owner_id":"seller","province_id":1,"method_id":"raw-iron","resource_id":"iron","resource_status":"source","land_access":true,"initial_multiplier":1,"facility_id":"wh:seller","assignments":[]})).unwrap());
        let mut f = w
            .production_data
            .storage_facilities
            .iter()
            .find(|f| f.owner_id == ExternalId::from("seller"))
            .unwrap()
            .clone();
        f.id = "state-wh".into();
        f.owner_id = "treasury:state:s".into();
        f.extra.insert("state_raw_warehouse".into(), json!(true));
        w.production_data.storage_facilities.push(f);
        w.market_data
            .price_calibration
            .as_mut()
            .unwrap()
            .prices_by_currency
            .get_mut("COIN")
            .unwrap()
            .insert("iron".into(), json!(1));
        w.accounts.push(AccountBalance {
            id: AccountId(50),
            external_id: "state-cash".into(),
            owner_id: "treasury:state:s".into(),
            currency_id: "COIN".into(),
            purpose: "operating_capital".into(),
            balance: Money(cash),
        });
        w.rebuild_account_index().unwrap();
    });
    kingmaker_sim::physical_supply::execute_physical_supply(&mut w, &physical::request()).unwrap();
    w
}
fn req(w: &WorldState) -> kingmaker_core::budget::BudgetRequest {
    kingmaker_core::budget::BudgetRequest {
        expected_revision: 20,
        period: "w".into(),
        policies: w.production_data.policies.clone(),
    }
}
fn cash(w: &WorldState, owner: &str) -> i64 {
    w.accounts
        .iter()
        .filter(|a| a.owner_id == ExternalId::from(owner) && a.purpose == "operating_capital")
        .map(|a| a.balance.0)
        .sum()
}
#[test]
fn actual_state_purchase_pays_seller_and_creates_state_owned_raw_stock() {
    let mut w = ready(1000);
    let total = w.accounts.iter().map(|a| a.balance.0).sum::<i64>();
    let initial_request = req(&w);
    let receipt = execute_raw_procurement(&mut w, &initial_request).unwrap();
    assert!(receipt.invariants.production_required_passed());
    assert_eq!(cash(&w, "seller"), 400);
    assert_eq!(cash(&w, "treasury:state:s"), 600);
    assert_eq!(w.accounts.iter().map(|a| a.balance.0).sum::<i64>(), total);
    assert_eq!(w.stockpiles[0].quantity, 0.);
    let s = w
        .stockpiles
        .iter()
        .find(|s| s.owner_id == ExternalId::from("treasury:state:s"))
        .unwrap();
    assert_eq!(s.quantity, 4.);
    assert_eq!(s.facility_id, Some("state-wh".into()));
    assert!(receipt
        .operations
        .iter()
        .any(|o| o.meta.get("public_procurement") == Some(&json!(true))));
    let r = req(&w);
    let before = serde_json::to_value(&w).unwrap();
    assert_eq!(execute_raw_procurement(&mut w, &r).unwrap(), receipt);
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
#[test]
fn limited_treasury_warehouse_and_slider_constrain_actual_purchase() {
    for case in 0..3 {
        let mut w = ready(500);
        match case{0=>w.production_data.obligations.push(serde_json::from_value(json!({"obligation_id":"protected","payer_id":"state:s","payee_id":"creditor","currency_id":"COIN","purpose":"operating_capital","amount_minor":"300"})).unwrap()),1=>w.production_data.storage_facilities.last_mut().unwrap().capacity=Some(2.),_=>{w.production_data.policies.extra.insert("state_budgets".into(),json!({"state:s":{"raw_procurement":50}}));}}
        let request = req(&w);
        execute_raw_procurement(&mut w, &request).unwrap();
        let expected = if case == 2 { 2.5 } else { 2. };
        assert_eq!(
            w.stockpiles
                .iter()
                .filter(|s| s.owner_id == ExternalId::from("treasury:state:s"))
                .map(|s| s.quantity)
                .sum::<f64>(),
            expected
        );
        assert_eq!(cash(&w, "seller"), if case == 2 { 250 } else { 200 });
    }
}
#[test]
fn unavailable_storage_disabled_policy_and_unapproved_offer_do_not_buy_free_goods() {
    for case in 0..4 {
        let mut w = ready(500);
        match case {
            0 => w.production_data.storage_facilities.pop().map(|_| ()),
            1 => {
                w.production_data.policies.extra.insert(
                    "state_budgets".into(),
                    json!({"state:s":{"raw_procurement":0}}),
                );
                None
            }
            2 => {
                w.market_data.sale_offers[0]
                    .extra
                    .insert("source".into(), json!(null));
                None
            }
            _ => {
                w.market_data.territory[0].extra.remove("owner_state_id");
                None
            }
        };
        let stocks = w.stockpiles.clone();
        let money = w.accounts.clone();
        let r = req(&w);
        execute_raw_procurement(&mut w, &r).unwrap();
        assert_eq!(w.stockpiles, stocks);
        assert_eq!(w.accounts, money);
    }
}
#[test]
fn stale_procurement_and_missing_boundary_publish_nothing() {
    let mut w = ready(500);
    let r = req(&w);
    let p = prepare_raw_procurement(&w, &r).unwrap();
    w.accounts.last_mut().unwrap().balance = Money(0);
    let before = serde_json::to_value(&w).unwrap();
    assert!(apply_raw_procurement(&mut w, p).is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
    let mut w = ready(500);
    let r = req(&w);
    execute_raw_procurement(&mut w, &r).unwrap();
    let id = w
        .transport_data
        .phase_receipts
        .last()
        .unwrap()
        .operations
        .last()
        .unwrap()
        .event_id
        .clone();
    w.production_data
        .committed_operations
        .retain(|o| o.event_id != id);
    let before = serde_json::to_value(&w).unwrap();
    assert!(execute_raw_procurement(&mut w, &r).is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
#[test]
fn procurement_replay_retains_explicit_request_policy_and_rejects_conflicts() {
    let mut w = ready(500);
    let mut r = req(&w);
    r.policies.extra.insert(
        "state_budgets".into(),
        json!({"state:s":{"raw_procurement":50}}),
    );
    let receipt = execute_raw_procurement(&mut w, &r).unwrap();
    let before = serde_json::to_value(&w).unwrap();
    assert_eq!(execute_raw_procurement(&mut w, &r).unwrap(), receipt);
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
    r.policies.extra.insert(
        "state_budgets".into(),
        json!({"state:s":{"raw_procurement":100}}),
    );
    assert!(execute_raw_procurement(&mut w, &r).is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
#[test]
fn state_source_filter_excludes_private_and_foreign_raw_stock_in_same_market() {
    let mut w = ready(1000);
    let mut private = w.stockpiles[0].clone();
    private.external_id = "AAA-private".into();
    private.owner_id = "private".into();
    private.quantity = 99.;
    w.stockpiles.push(private);
    let mut offer = w.market_data.sale_offers[0].clone();
    offer.id = Some("private-offer".into());
    offer.stock_id = "AAA-private".into();
    offer.seller_id = "private".into();
    w.market_data.sale_offers.push(offer);
    let mut foreign = w.stockpiles[0].clone();
    foreign.external_id = "AAA-foreign".into();
    foreign.province_id = ProvinceId(3);
    foreign.quantity = 99.;
    w.stockpiles.push(foreign);
    let mut offer = w.market_data.sale_offers[0].clone();
    offer.id = Some("foreign-offer".into());
    offer.stock_id = "AAA-foreign".into();
    w.market_data.sale_offers.push(offer);
    let p = w
        .market_data
        .territory
        .iter_mut()
        .find(|p| p.province_id == ProvinceId(3))
        .unwrap();
    p.region_id = Some("r1".into());
    p.extra.insert("owner_state_id".into(), json!("state:t"));
    let mut f = w.production_data.storage_facilities.last().unwrap().clone();
    f.id = "foreign-wh".into();
    f.owner_id = "treasury:state:t".into();
    f.province_id = ProvinceId(3);
    w.production_data.storage_facilities.push(f);
    let r = req(&w);
    execute_raw_procurement(&mut w, &r).unwrap();
    assert_eq!(
        w.stockpiles
            .iter()
            .find(|s| s.external_id == ExternalId::from("AAA-private"))
            .unwrap()
            .quantity,
        99.
    );
    assert_eq!(
        w.stockpiles
            .iter()
            .find(|s| s.external_id == ExternalId::from("AAA-foreign"))
            .unwrap()
            .quantity,
        99.
    );
    assert_eq!(
        w.stockpiles
            .iter()
            .filter(|s| s.owner_id == ExternalId::from("treasury:state:s"))
            .map(|s| s.quantity)
            .sum::<f64>(),
        4.
    );
    assert_eq!(cash(&w, "seller"), 400);
}
#[test]
fn raw_slider_inherits_authored_value_when_request_only_changes_another_sector() {
    let mut w = ready(500);
    w.production_data.policies.extra.insert(
        "state_budgets".into(),
        json!({"state:s":{"raw_procurement":0}}),
    );
    let mut r = req(&w);
    r.policies
        .extra
        .insert("state_budgets".into(), json!({"state:s":{"education":50}}));
    let stocks = w.stockpiles.clone();
    let accounts = w.accounts.clone();
    let receipt = execute_raw_procurement(&mut w, &r).unwrap();
    assert_eq!(w.stockpiles, stocks);
    assert_eq!(w.accounts, accounts);
    assert!(receipt
        .diagnostics
        .iter()
        .any(|d| d["reason"] == json!("state_procurement_disabled")));
}
#[test]
fn actual_js_state_procurement_matches_real_payments_limited_budget_and_warehouse() {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    for case in 0..3 {
        let mut w = ready(if case == 0 { 1000 } else { 250 });
        if case == 2 {
            w.production_data
                .storage_facilities
                .last_mut()
                .unwrap()
                .capacity = Some(2.);
        }
        let r = req(&w);
        let mut snapshot = income_common::snapshot(&w);
        snapshot["campaign"]["campaign_id"] = json!(w.campaign.campaign_id);
        snapshot["registry"]["resourceOperations"] = json!(w.production_data.resource_operations);
        snapshot["registry"]["policies"] = json!(w.production_data.policies);
        snapshot["registry"]["consumerFacilityByProvince"] =
            json!(w.market_data.consumer_facility_by_province);
        snapshot["appliedEventIds"] = json!(w
            .production_data
            .committed_operations
            .iter()
            .map(|o| &o.event_id)
            .collect::<Vec<_>>());
        let mut child = Command::new("node")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../scripts/tests/shipping-oracle.cjs"
            ))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(json!({"action":"state-procurement","snapshot":snapshot,"period":"w","policies":r.policies}).to_string().as_bytes()).unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let js: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        let mut ops: Vec<kingmaker_core::operations::EconomicOperation> =
            serde_json::from_value(js["operations"].clone()).unwrap();
        for op in &mut ops {
            for key in ["quantity", "requested_quantity"] {
                if let Some(q) = op.meta.get(key).and_then(serde_json::Value::as_f64) {
                    op.meta.insert(key.into(), json!(q));
                }
            }
        }
        let receipt = execute_raw_procurement(&mut w, &r).unwrap();
        assert_eq!(
            &receipt.operations[..receipt.operations.len() - 1],
            ops,
            "case {case}: {js}"
        );
    }
}
