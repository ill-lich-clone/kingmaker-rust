#[path = "common/income.rs"]
mod common;
#[path = "common/market.rs"]
mod market;
#[path = "common/production.rs"]
mod production;
use kingmaker_core::{transport::*, *};
use serde_json::json;
#[test]
fn closing_income_rejects_missing_shipping_phases_even_when_they_would_be_empty() {
    for stage in [
        TransportStage::Arrivals,
        TransportStage::LocalFoodReserves,
        TransportStage::Dispatch,
        TransportStage::StateProcurement,
    ] {
        let (mut w, basis, _) = common::fixture();
        w.transport_data
            .phase_receipts
            .retain(|p| p.request.stage != stage);
        let before = serde_json::to_value(&w).unwrap();
        assert!(kingmaker_sim::income_cycle::execute_income_closing(
            &mut w,
            &common::request(),
            &basis
        )
        .is_err());
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
    }
}
#[test]
fn frozen_income_opening_must_precede_actual_native_arrivals() {
    let mut w = production::world();
    let r = common::request();
    kingmaker_sim::shipment_arrivals::execute_shipment_arrivals(
        &mut w,
        &TransportRequest {
            expected_revision: 20,
            period: "w".into(),
            stage: TransportStage::Arrivals,
        },
    )
    .unwrap();
    assert!(kingmaker_sim::income_basis::capture_income_opening(&w, &r).is_err());
}
#[test]
fn income_rejects_tampered_shipping_evidence_and_complete_receipts_survive_snapshot() {
    for stage in [
        TransportStage::Arrivals,
        TransportStage::LocalFoodReserves,
        TransportStage::Dispatch,
        TransportStage::StateProcurement,
    ] {
        let (mut w, basis, _) = common::fixture();
        w.transport_data
            .phase_receipts
            .iter_mut()
            .find(|p| p.request.stage == stage)
            .unwrap()
            .registry_changes = json!({"fake":true});
        let before = serde_json::to_value(&w).unwrap();
        assert!(kingmaker_sim::income_cycle::execute_income_closing(
            &mut w,
            &common::request(),
            &basis
        )
        .is_err());
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
    }
    let (w, basis, _) = common::fixture();
    let mut loaded: WorldState = serde_json::from_value(serde_json::to_value(&w).unwrap()).unwrap();
    loaded.rebuild_account_index().unwrap();
    let receipt = kingmaker_sim::income_cycle::execute_income_closing(
        &mut loaded,
        &common::request(),
        &basis,
    )
    .unwrap();
    assert!(receipt.invariants.all_required_passed());
    assert!(receipt.input_evidence.len() >= 4);
}
#[test]
fn real_factory_output_travels_with_creator_rights_and_income_is_recognized_on_arrival() {
    for artisan in [false, true] {
        let (mut w, basis, _) = common::fixture_configure(false, |w| {
            if artisan {
                let f = w.production_data.factories.remove(0);
                w.production_data
                    .artisans
                    .push(serde_json::from_value(serde_json::to_value(f).unwrap()).unwrap());
                w.production_data.artisans[0].base.payment_purpose =
                    Some("operating_capital".into());
                w.production_data.artisans[0].base.extra.insert(
                    "income_policy".into(),
                    json!({"source":"fixture","cash_reserve_minor":"0","payout_fraction":1}),
                );
                w.production_data.artisans[0].base.assignments[0].contract_id = None;
                w.production_data.contracts.clear();
                let m = &mut w.production_data.catalog.as_mut().unwrap().methods[0];
                m.kind = "artisan".into();
                m.staffing = vec!["artisans".into()];
                w.pops[0].pop_type_id = "artisans".into();
                w.pops[0].pop_type = "artisans".into();
            }
            w.stockpiles.retain(|s| s.goods != Goods::Steel);
            w.market_data.parameters.as_mut().unwrap().instant_food = Some(false);
            let mut p = market::market_world().market_data.territory[1].clone();
            p.region_id = Some("other".into());
            w.market_data.territory.push(p);
            w.market_data.territory[0]
                .extra
                .insert("neighbor_ids".into(), json!([2]));
            w.market_data.territory[1]
                .extra
                .insert("neighbor_ids".into(), json!([1]));
            w.provinces.push(ProvinceRecord {
                id: ProvinceId(2),
                owner_state_id: Some("s".into()),
                pop_range: (1, 2),
                ..Default::default()
            });
            let mut buyer = w.pops[0].clone();
            buyer.id = PopId(2);
            buyer.external_id = "buyer".into();
            buyer.province_id = ProvinceId(2);
            buyer.workforce = 0.;
            buyer.employed = 0.;
            buyer.pop_type_id = "farmers".into();
            buyer.pop_type = "farmers".into();
            w.pops.push(buyer);
            w.accounts
                .push(production::account(120, "buyer", "personal_savings", 500));
            w.rebuild_account_index().unwrap();
            let needs = w.market_data.needs.as_mut().unwrap();
            needs.profile_refs[0].pop_id = "buyer".into();
            needs.profiles[0].norms[0]
                .extra
                .insert("quantity_per_person_week".into(), json!(1));
            w.production_data.storage_facilities.push(serde_json::from_value(json!({"id":"buyer-wh","owner_id":"buyer","province_id":2,"capacity":100,"unit":"space","status":"source","source":"fixture","goods":{"steel":{"unit":"t","space_per_unit":1}}})).unwrap());
            w.market_data.consumer_facility_by_province = Some(json!({"2":"buyer-wh"}));
            // Declared offer references the deterministic stock ID of actual native output; no stock is seeded.
            w.market_data.sale_offers=serde_json::from_value(json!([{"id":"steel-offer","stock_id":"produced:o:1:steel:wh","seller_id":"o","seller_purpose":"employer_operating_capital","goods_id":"steel","currency_id":"COIN","unit":"t","price_minor_per_unit":"300","status":"source","source":"declared miniature offer"}])).unwrap();
            if artisan {
                w.market_data.sale_offers[0].seller_purpose = Some("operating_capital".into());
            }
            w.transport_data.transport_assets = vec![
                json!({"id":"cart","owner_id":"carrier","home_province_id":1,"modes":["land"],"unit":"space","capacity":2,"status":"source","source":"fixture"}),
            ];
            w.transport_data.supply_routes = vec![
                json!({"id":"road","from_province_id":1,"to_province_id":2,"asset_id":"cart","carrier_id":"carrier","mode":"land","unit":"space","currency_id":"COIN","capacity_per_week":2,"source_handling_per_week":2,"destination_handling_per_week":2,"travel_days":7,"return_days":7,"freight_minor_per_space":"2","goods":{"steel":{"unit":"t","space_per_unit":1}},"path":[1,2],"status":"source","source":"fixture"}),
            ];
        });
        assert_eq!(
            w.mobility_data.shipments.len(),
            1,
            "diagnostics {:?}; stocks {:?}",
            w.transport_data
                .phase_receipts
                .iter()
                .map(|p| &p.diagnostics)
                .collect::<Vec<_>>(),
            w.stockpiles
        );
        assert_eq!(
            w.mobility_data.shipments[0]["goods_origin"],
            json!("produced")
        );
        assert_eq!(w.mobility_data.shipments[0]["seller_id"], json!("o"));
        assert_eq!(
            w.mobility_data.shipments[0]["stock_id"],
            json!("produced:o:1:steel:wh")
        );
        let income = kingmaker_sim::income_recognition::recognize_producer_income(
            &w,
            &common::request(),
            &basis,
        )
        .unwrap();
        assert!(income.iter().all(|i| i.revenue_minor == Money(0)));
        kingmaker_sim::income_cycle::execute_income_closing(&mut w, &common::request(), &basis)
            .unwrap();
        let mut w: WorldState = serde_json::from_value(serde_json::to_value(&w).unwrap()).unwrap();
        w.rebuild_account_index().unwrap();
        w.campaign.day = 8;
        w.campaign.revision = 21;
        let r = kingmaker_core::income::IncomeRequest {
            expected_revision: 21,
            period: "w2".into(),
            policies: Default::default(),
        };
        let basis = kingmaker_sim::income_basis::capture_income_opening(&w, &r).unwrap();
        common::opening_arrivals(&mut w, "w2");
        assert_eq!(
            w.mobility_data.shipments[0]["remaining_quantity"].as_f64(),
            Some(0.)
        );
        assert!(w
            .stockpiles
            .iter()
            .any(|s| s.owner_id == ExternalId::from("buyer")
                && s.goods == Goods::Steel
                && s.quantity == 1.));
        kingmaker_sim::budget_cycle::execute_budget_cycle(
            &mut w,
            &kingmaker_core::budget::BudgetRequest {
                expected_revision: 21,
                period: "w2".into(),
                policies: Default::default(),
            },
        )
        .unwrap();
        kingmaker_sim::production::execute_production_cycle(
            &mut w,
            &kingmaker_core::production::ProductionRequest {
                expected_revision: 21,
                period: "w2".into(),
                policies: Default::default(),
                profile: Default::default(),
                maintenance_receipts: vec![],
            },
        )
        .unwrap();
        kingmaker_sim::market_cycle::execute_market_phase(
            &mut w,
            &kingmaker_core::market::MarketRequest {
                expected_revision: 21,
                period: "w2".into(),
                stage: kingmaker_core::market::MarketStage::ClosingMarket,
                profile: Default::default(),
                priorities: None,
            },
        )
        .unwrap();
        common::finish_supply(&mut w, "w2");
        let income =
            kingmaker_sim::income_recognition::recognize_producer_income(&w, &r, &basis).unwrap();
        if artisan {
            let plan = kingmaker_sim::artisan_income::plan_artisan_income(&w, &r, &basis).unwrap();
            assert_eq!(
                plan.enterprises[0].base.extra["income_history"]["revenue_minor"],
                json!("300")
            );
        } else {
            assert_eq!(
                income
                    .iter()
                    .filter(|i| i.owner_id == ExternalId::from("o"))
                    .map(|i| i.revenue_minor.0)
                    .sum::<i64>(),
                300
            );
        }
        kingmaker_sim::income_cycle::execute_income_closing(&mut w, &r, &basis).unwrap();
    }
}
