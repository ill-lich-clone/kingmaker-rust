#![allow(dead_code)]
use kingmaker_core::market::*;
use kingmaker_core::*;
use serde_json::{json, Value};

pub fn need_fixture() -> (NeedsData, NeedProfileRef) {
    let reference:NeedProfileRef=serde_json::from_value(json!({"pop_id":"a","need_profile_id":"profile:food","size":1000,"wealth_band_id":"poor","literacy":0.5})).unwrap();
    let needs=serde_json::from_value(json!({"schema_version":"weekly-needs-v1","profileRefs":[reference],"profiles":[{"need_profile_id":"profile:food","species_food_factor":1,"norms":[{"need_id":"food","period":"week","status":"known","quantity_status":"authored","quantity_per_person_week":0.01,"unit":"t","alternatives":[{"goods_id":"grain","share":1}]}]}],"resolver":{"period":"week","period_days":7}})).unwrap();
    (needs, reference)
}

pub fn market_world() -> WorldState {
    let mut w = WorldState::default();
    w.campaign = CampaignHeader {
        campaign_id: "market-fixture".into(),
        revision: 20,
        year: 306,
        month: 1,
        day: 1,
        ..Default::default()
    };
    for n in 1..=3 {
        w.provinces.push(ProvinceRecord {
            id: ProvinceId(n),
            name: format!("Province {n}"),
            owner_state_id: Some("state:s".into()),
            ..Default::default()
        });
        w.market_data.territory.push(serde_json::from_value(json!({"province_id":n,"region_id":if n<3 {"r"} else {"other"},"is_floating":false,"landmass_id":"land:1","neighbors":if n==1 {vec![2]} else if n==2 {vec![1,3]} else {vec![2]}})).unwrap());
    }
    for (i, name) in ["a", "b"].iter().enumerate() {
        let n = i as u32 + 1;
        w.pops.push(PopRecord {
            id: PopId(n),
            external_id: (*name).into(),
            province_id: ProvinceId(n),
            pop_type: "farmers".into(),
            pop_type_id: "farmers".into(),
            size: 1000,
            workforce: 600.0,
            unemployed: 600.0,
            currency_id: Some("COIN".into()),
            account_status: "bound".into(),
            account_id: Some(AccountId(n)),
            ..Default::default()
        });
        w.accounts.push(AccountBalance {
            id: AccountId(n),
            external_id: format!("cash:{name}").into(),
            owner_id: (*name).into(),
            currency_id: "COIN".into(),
            purpose: "personal_savings".into(),
            balance: Money(10000),
        });
    }
    w.accounts.push(AccountBalance {
        id: AccountId(3),
        external_id: "cash:seller".into(),
        owner_id: "seller".into(),
        currency_id: "COIN".into(),
        purpose: "employer_operating_capital".into(),
        balance: Money(0),
    });
    w.provinces[0].pop_range = (0, 1);
    w.provinces[1].pop_range = (1, 2);
    let (mut needs, mut reference) = need_fixture();
    reference.pop_id = "b".into();
    needs.profile_refs.push(reference);
    w.market_data.needs = Some(needs);
    w.market_data.parameters =
        Some(serde_json::from_value(json!({"instant_food":true,"regional_market":true})).unwrap());
    w.market_data.price_calibration=Some(serde_json::from_value(json!({"pricesByCurrency":{"COIN":{"grain":1,"flour":1,"meat":1,"fish":1,"coal":1,"iron":1,"steel":1,"clothing":1,"tools":1}}})).unwrap());
    w.stockpiles.push(StockpileRecord {
        id: 1,
        external_id: "grain:2".into(),
        province_id: ProvinceId(2),
        goods: Goods::Grain,
        quantity: 20.0,
        owner_id: "seller".into(),
        unit: Some(json!("t")),
        currency_id: Some("COIN".into()),
        facility_id: Some("wh:seller".into()),
        inventory_origin: Some("opening".into()),
        ..Default::default()
    });
    w.production_data.catalog=Some(serde_json::from_value(json!({"version":"production-foundation-v1","goods":[{"goods_id":"grain","kind":"raw_resource","unit":"t"},{"goods_id":"livestock","kind":"raw_resource","unit":"t"},{"goods_id":"flour","kind":"processed_good","unit":"t"},{"goods_id":"meat","kind":"processed_good","unit":"t"},{"goods_id":"fish","kind":"raw_resource","unit":"t"}],"methods":[]})).unwrap());
    for (owner, prov) in [("a", 1), ("b", 2), ("seller", 2)] {
        w.production_data.storage_facilities.push(serde_json::from_value(json!({"id":format!("wh:{owner}"),"owner_id":owner,"province_id":prov,"capacity":100,"unit":"space","status":"source","source":"fixture","goods":{"grain":{"unit":"t","space_per_unit":1},"flour":{"unit":"t","space_per_unit":1},"meat":{"unit":"t","space_per_unit":1},"fish":{"unit":"t","space_per_unit":1}},"access_contracts":[]})).unwrap());
    }
    w.rebuild_account_index().unwrap();
    w
}

pub fn order(
    id: &str,
    buyer: &str,
    good: &str,
    quantity: f64,
    priority: &str,
    consume: bool,
) -> MarketOrder {
    MarketOrder {
        id: id.into(),
        owner_id: buyer.into(),
        pop_id: consume.then(|| buyer.into()),
        province_id: ProvinceId(1),
        currency_id: Some("COIN".into()),
        purpose: if consume {
            "personal_savings".into()
        } else {
            "employer_operating_capital".into()
        },
        goods_id: good.into(),
        quantity,
        priority: priority.into(),
        consume,
        need_id: (consume && priority == "basic").then(|| "food".into()),
        maintenance: false,
        facility_id: (!consume).then(|| format!("wh:{buyer}").into()),
        food_alternatives: vec![],
        food_gross_quantity: None,
        military_food_substitution: None,
        extra: Default::default(),
    }
}

pub fn assert_close(actual: f64, expected: f64, eps: f64) {
    assert!(
        (actual - expected).abs() <= eps * actual.abs().max(expected.abs()).max(1.0),
        "{actual} != {expected}"
    );
}
pub fn assert_source_values(native: &Value, js: &Value) {
    match (native, js) {
        (Value::Number(a), Value::Number(b)) => {
            assert_close(a.as_f64().unwrap(), b.as_f64().unwrap(), 1e-12)
        }
        (Value::Array(a), Value::Array(b)) => {
            assert_eq!(a.len(), b.len());
            for (a, b) in a.iter().zip(b) {
                assert_source_values(a, b);
            }
        }
        (Value::Object(a), Value::Object(b)) => {
            assert_eq!(
                a.keys().collect::<Vec<_>>(),
                b.keys().collect::<Vec<_>>(),
                "{native}\n{js}"
            );
            for (key, a) in a {
                assert_source_values(a, &b[key]);
            }
        }
        _ => assert_eq!(native, js),
    }
}

pub fn snapshot(w: &WorldState) -> Value {
    json!({"campaign":w.campaign,"accounts":w.accounts.iter().map(|a|json!({"owner_id":a.owner_id,"currency_id":a.currency_id,"purpose":a.purpose,"amount_minor":a.balance})).collect::<Vec<_>>(),"stocks":w.stockpiles.iter().map(|s|json!({"opening_stock_id":s.external_id,"owner_id":s.owner_id,"province_id":s.province_id.0,"goods_id":s.goods.external_id(),"amount":s.quantity,"unit":s.unit,"currency_id":s.currency_id,"facility_id":s.facility_id,"inventory_origin":s.inventory_origin,"inventory_purpose":s.purpose,"beneficiary_pop_id":s.beneficiary_pop_id})).collect::<Vec<_>>(),"obligations":[],"registry":{"needs":w.market_data.needs,"priceCalibration":w.market_data.price_calibration,"foundationParameters":w.market_data.parameters,"territory":w.market_data.territory,"maintenanceAgreements":w.market_data.maintenance_agreements,"saleOffers":w.market_data.sale_offers,"pops":w.pops.iter().map(|p|json!({"pop_id":p.external_id,"province_id":p.province_id.0,"currency_id":p.currency_id,"pop_type_id":p.pop_type_id})).collect::<Vec<_>>(),"productionCatalog":w.production_data.catalog,"factoryEnterprises":w.production_data.factories,"artisanEnterprises":w.production_data.artisans,"constructionProjects":w.production_data.construction_projects,"storageFacilities":w.production_data.storage_facilities,"storageReservations":w.production_data.storage_reservations,"goodsReservations":w.production_data.goods_reservations}})
}
pub fn oracle(input: Value) -> Value {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new("node")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/tests/market-oracle.cjs"
        ))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.to_string().as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

pub fn full_snapshot(w: &WorldState) -> Value {
    let mut value = snapshot(w);
    let registry = value["registry"].as_object_mut().unwrap();
    for (key, rows) in [
        (
            "contracts",
            serde_json::to_value(&w.production_data.contracts).unwrap(),
        ),
        (
            "resourceOperations",
            serde_json::to_value(&w.production_data.resource_operations).unwrap(),
        ),
        (
            "completedConstruction",
            serde_json::to_value(&w.production_data.completed_construction).unwrap(),
        ),
        (
            "completedExpansions",
            serde_json::to_value(&w.production_data.expansions).unwrap(),
        ),
        ("institutions", json!(w.market_data.institutions)),
        (
            "constructionCatalog",
            json!(w.market_data.construction_catalog),
        ),
    ] {
        registry.insert(key.into(), rows);
    }
    value
}
