#[path = "common/production.rs"]
mod common;
use kingmaker_core::*;
use kingmaker_sim::production::*;
use serde_json::{json, Value};
use std::{collections::BTreeMap, process::Command};
fn snapshot(w: &WorldState) -> Value {
    json!({"campaign":{"campaign_id":w.campaign.campaign_id},"registry":{"pops":w.pops.iter().map(|p|json!({"pop_id":p.external_id,"state_id":p.state_id,"province_id":p.province_id.0,"pop_type_id":p.pop_type_id,"workforce":p.workforce})).collect::<Vec<_>>(),"productionCatalog":w.production_data.catalog,"resourceOperations":w.production_data.resource_operations,"factoryEnterprises":w.production_data.factories,"artisanEnterprises":w.production_data.artisans,"contracts":w.production_data.contracts,"completedExpansions":w.production_data.expansions,"constructionProjects":w.production_data.construction_projects,"completedConstruction":w.production_data.completed_construction,"goodsReservations":w.production_data.goods_reservations,"storageFacilities":w.production_data.storage_facilities,"storageReservations":w.production_data.storage_reservations},"accounts":w.accounts.iter().map(|a|json!({"account_id":a.external_id,"owner_id":a.owner_id,"currency_id":a.currency_id,"purpose":a.purpose,"amount_minor":a.balance})).collect::<Vec<_>>(),"stocks":w.stockpiles.iter().map(|s|json!({"opening_stock_id":s.external_id,"owner_id":s.owner_id,"province_id":s.province_id.0,"goods_id":s.goods.external_id(),"unit":s.unit,"facility_id":s.facility_id,"inventory_origin":s.inventory_origin,"inventory_purpose":s.purpose,"currency_id":s.currency_id,"amount":s.quantity})).collect::<Vec<_>>(),"obligations":w.production_data.obligations,"reservations":w.production_data.money_reservations})
}
#[test]
fn original_storage_rounding_fixture_matches_native_batch() {
    use kingmaker_sim::{operation_session::*, storage::*};
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "../../../Проект карты Тейванкаля/scripts/tests/fixtures/kingmaker-storage-rounding.json",
    );
    let fixture: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut w = WorldState::default();
    w.production_data.storage_facilities =
        serde_json::from_value(fixture["snapshot"]["registry"]["storageFacilities"].clone())
            .unwrap();
    for (i, row) in fixture["snapshot"]["stocks"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        w.stockpiles.push(StockpileRecord {
            id: i as u32,
            external_id: serde_json::from_value(row["opening_stock_id"].clone()).unwrap(),
            province_id: ProvinceId::from_external(
                &serde_json::from_value(row["province_id"].clone()).unwrap(),
            )
            .unwrap(),
            owner_id: serde_json::from_value(row["owner_id"].clone()).unwrap(),
            goods: Goods::from_external(serde_json::from_value(row["goods_id"].clone()).unwrap()),
            unit: Some(row["unit"].clone()),
            facility_id: Some(serde_json::from_value(row["facility_id"].clone()).unwrap()),
            quantity: row["amount"].as_f64().unwrap(),
            ..Default::default()
        });
    }
    let expected = oracle(json!({"mode":"storage"}));
    let requests: Vec<StorageRequest> =
        serde_json::from_value(fixture["requests"].clone()).unwrap();
    let mut s = OperationSession::new(&w).unwrap();
    let mut batch = StorageBatch::new(&s).unwrap();
    for req in requests {
        let out = reserve_storage(
            &mut s,
            &mut batch,
            &req,
            fixture["period"].as_str().unwrap(),
        )
        .unwrap();
        assert!(out.reason.is_none());
        let row = out.reservation.unwrap();
        let reference = expected["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == row.id.json())
            .unwrap();
        assert_eq!(row.quantity, reference["quantity"].as_f64().unwrap());
        assert_eq!(
            out.partial,
            expected["partial"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["id"] == row.id.json())
        );
    }
}
fn oracle(input: Value) -> Value {
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/tests/production-oracle.cjs");
    let out = Command::new("node")
        .arg(script)
        .arg(input.to_string())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}
#[test]
fn native_cycle_matches_real_js_across_nine_economic_cases() {
    for case in 0..9 {
        let mut w = common::world();
        let mut request = common::request();
        match case {
            1 => w.accounts[0].balance = Money(4),
            2 => w.stockpiles[0].quantity = 0.0,
            3 => {
                w.production_data.contracts[0]
                    .weekly_wage_rate
                    .as_mut()
                    .unwrap()
                    .numerator = "0".into();
                w.production_data.contracts[0].guaranteed_idle_weekly_rate = None;
            }
            4 => w.production_data.factories[0].equipment_capacity_weekly = Some(0.5),
            5 => request.profile.productivity_multiplier = Some(2.0),
            6 => {
                w.production_data.factories.clear();
                w.pops[0].pop_type_id = "artisans".into();
                let m = &mut w.production_data.catalog.as_mut().unwrap().methods[0];
                m.kind = "artisan".into();
                m.staffing = vec!["artisans".into()];
                m.method_id = "artisan:steel".into();
                w.production_data.artisans.push(serde_json::from_value(json!({"id":"a","owner_id":"o","province_id":1,"method_id":"artisan:steel","facility_id":"wh","people_slots":1,"assignments":[{"pop_id":"p","workers_count":1,"time_fraction":1}]})).unwrap());
            }
            7 => {
                w.pops[0].workforce = 2.0;
                w.pops[0].size = 2;
                w.stockpiles[1].quantity = 0.8;
                let mut f = w.production_data.factories[0].clone();
                f.base.id = "g".into();
                f.base.assignments[0].employment_id = Some("e2".into());
                w.production_data.factories.push(f);
            }
            8 => {
                w.stockpiles[0].external_id = "Z".into();
                let mut extra = w.stockpiles[0].clone();
                extra.id = 13;
                extra.external_id = "a".into();
                w.stockpiles.push(extra);
            }
            _ => {}
        }
        let js = oracle(json!({"snapshot":snapshot(&w),"request":request}));
        let native = execute_production_cycle(&mut w, &request).unwrap();
        let accounts: BTreeMap<_, _> = w
            .accounts
            .iter()
            .map(|a| {
                (
                    json!([a.owner_id, a.currency_id, a.purpose]).to_string(),
                    a.balance.0.to_string(),
                )
            })
            .collect();
        let expected: BTreeMap<_, _> = js["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| {
                (
                    json!([a["owner_id"], a["currency_id"], a["purpose"]]).to_string(),
                    a["amount_minor"].as_str().unwrap().to_owned(),
                )
            })
            .collect();
        assert_eq!(accounts, expected, "accounts case {case}");
        for stock in &w.stockpiles {
            let j = js["stocks"]
                .as_array()
                .unwrap()
                .iter()
                .find(|s| s["opening_stock_id"] == stock.external_id.json())
                .unwrap();
            assert!(
                (stock.quantity - j["amount"].as_f64().unwrap()).abs() < 1e-10,
                "stocks case {case}"
            );
            assert_eq!(j["owner_id"], stock.owner_id.json());
            assert_eq!(j["goods_id"], stock.goods.external_id().json());
        }
        assert_eq!(js["stocks"].as_array().unwrap().len(), w.stockpiles.len());
        for out in &native.outputs {
            let a = js["activities"]
                .as_array()
                .unwrap()
                .iter()
                .find(|a| a["activity_id"] == out.activity_id)
                .unwrap();
            assert!(
                (out.quantity - a["output"].as_f64().unwrap()).abs() < 1e-10,
                "output case {case}"
            );
            assert!(
                (out.fte - a["fte"].as_f64().unwrap()).abs() < 1e-10,
                "fte case {case}"
            );
        }
        for p in &native.payroll {
            let j = js["payroll"]
                .as_array()
                .unwrap()
                .iter()
                .find(|j| j["employment_id"] == p.employment_id.json())
                .unwrap();
            for (field, value) in [
                ("gross_minor", p.gross_minor),
                ("paid_gross_minor", p.paid_gross_minor),
                ("tax_minor", p.tax_minor),
                ("net_minor", p.net_minor),
                ("idle_guarantee_minor", p.idle_guarantee_minor),
            ] {
                assert_eq!(j[field], json!(value), "{field} case {case}");
            }
            assert!((p.actual_fte - j["actual_fte"].as_f64().unwrap()).abs() < 1e-10);
        }
        for f in &w.production_data.factories {
            let j = js["factories"]
                .as_array()
                .unwrap()
                .iter()
                .find(|j| j["id"] == f.base.id.json())
                .unwrap();
            assert!((f.tenure_days.unwrap() - j["tenure_days"].as_f64().unwrap()).abs() < 1e-10);
        }
        for r in &w.production_data.storage_reservations {
            let j = js["storage"]
                .as_array()
                .unwrap()
                .iter()
                .find(|j| j["id"] == r.id.json())
                .unwrap();
            assert_eq!(r.status, j["status"].as_str().unwrap());
            assert!((r.quantity - j["quantity"].as_f64().unwrap()).abs() < 1e-10);
        }
    }
}
