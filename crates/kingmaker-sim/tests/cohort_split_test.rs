#[path = "common/labor.rs"]
mod common;
use kingmaker_core::{
    mobility::{CohortSplitRequest, SplitTarget},
    *,
};
use kingmaker_sim::cohort_cycle::{apply_pop_split, execute_pop_split, prepare_pop_split};
use serde_json::json;
fn fixture() -> WorldState {
    let mut w = common::world(0);
    w.pops[0].size = 1001;
    w.pops[0].extra.insert("literate_count".into(), json!(333));
    w.labor_data.pop_mobility = Some(
        json!({"unemployment":{"a":[{"workers":600,"weeks":3}]},"remainders":{"a":{"profession":0.3}}}),
    );
    w
}
fn request() -> CohortSplitRequest {
    CohortSplitRequest {
        expected_revision: 20,
        event_id: "split:w:a".into(),
        pop_id: "a".into(),
        workforce_count: 60,
        target: SplitTarget::default(),
    }
}
#[test]
fn prepared_split_rejects_shifted_need_reference_positions_without_writes() {
    let mut w = fixture();
    let prepared = prepare_pop_split(&w, &request()).unwrap();
    let needs = w.market_data.needs.as_mut().unwrap();
    let mut unrelated = needs.profile_refs[0].clone();
    unrelated.pop_id = "b".into();
    needs.profile_refs.insert(0, unrelated);
    let before = serde_json::to_value(&w).unwrap();
    assert!(apply_pop_split(&mut w, prepared).is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
#[test]
fn prepared_split_rejects_shifted_territory_positions_without_writes() {
    let mut w = fixture();
    let prepared = prepare_pop_split(&w, &request()).unwrap();
    let mut unrelated = w.market_data.territory[0].clone();
    unrelated.province_id = ProvinceId(2);
    w.market_data.territory.insert(0, unrelated);
    let before = serde_json::to_value(&w).unwrap();
    assert!(apply_pop_split(&mut w, prepared).is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
#[test]
fn full_split_preserves_empty_stock_destinations_and_their_metadata() {
    let mut w = fixture();
    personal(&mut w);
    let stock = w
        .stockpiles
        .iter_mut()
        .find(|s| s.external_id == ExternalId::from("personal"))
        .unwrap();
    stock.quantity = 0.;
    w.production_data.goods_reservations.push(
        serde_json::from_value(json!({"stock_id":"personal","status":"held","quantity":0}))
            .unwrap(),
    );
    w.market_data.sale_offers.push(
        serde_json::from_value(
            json!({"id":"empty-offer","stock_id":"personal","seller_id":"a","goods_id":"grain"}),
        )
        .unwrap(),
    );
    let mut r = request();
    r.workforce_count = 600;
    let receipt = execute_pop_split(&mut w, &r).unwrap();
    let destination = &w.market_data.sale_offers[0].stock_id;
    let stock = w
        .stockpiles
        .iter()
        .find(|s| &s.external_id == destination)
        .expect("Remapped empty stock destination must exist");
    assert_eq!(stock.owner_id, receipt.child_id.unwrap());
    assert_eq!(stock.quantity, 0.);
    assert_eq!(stock.extra["author_formula"], json!("never replace"));
    assert_eq!(
        &w.production_data.goods_reservations[0].stock_id,
        destination
    );
    let restored: WorldState = serde_json::from_value(serde_json::to_value(&w).unwrap()).unwrap();
    assert!(restored
        .stockpiles
        .iter()
        .any(|s| &s.external_id == &restored.production_data.goods_reservations[0].stock_id));
}
fn personal(w: &mut WorldState) {
    let mut stock = w.stockpiles[0].clone();
    stock.id = 900;
    stock.external_id = "personal".into();
    stock.owner_id = "a".into();
    stock.province_id = ProvinceId(1);
    stock.quantity = 12.25;
    stock
        .extra
        .insert("author_formula".into(), json!("never replace"));
    w.stockpiles.push(stock);
}
#[test]
fn own_artisan_keeps_recipe_inputs_and_produced_stock_except_food_reserve() {
    let mut w = fixture();
    personal(&mut w);
    let mut method = w.production_data.catalog.as_ref().unwrap().methods[0].clone();
    method.method_id = "artisan:cloth".into();
    method.kind = "artisan".into();
    method.output_goods_id = "cloth".into();
    method.inputs = serde_json::from_value(json!([{"goods_id":"grain","amount":1}])).unwrap();
    w.production_data
        .catalog
        .as_mut()
        .unwrap()
        .methods
        .push(method);
    w.production_data.artisans.push(serde_json::from_value(json!({"id":"artisan:a","owner_id":"a","province_id":1,"method_id":"artisan:cloth","people_slots":600,"assignments":[]})).unwrap());
    let mut food = w.stockpiles.last().unwrap().clone();
    food.id = 901;
    food.external_id = "food".into();
    food.purpose = "food_reserve".into();
    w.stockpiles.push(food);
    let mut output = w.stockpiles.last().unwrap().clone();
    output.id = 902;
    output.external_id = "output".into();
    output.purpose = "commercial".into();
    output.goods = Goods::from_external("cloth".into());
    output.inventory_origin = Some("produced".into());
    w.stockpiles.push(output);
    let r = execute_pop_split(&mut w, &request()).unwrap();
    let child = r.child_id.unwrap();
    let transferred: Vec<_> = w
        .stockpiles
        .iter()
        .filter(|s| s.owner_id == child)
        .collect();
    assert_eq!(transferred.len(), 1);
    assert_eq!(transferred[0].purpose, "food_reserve");
    assert_eq!(
        w.stockpiles
            .iter()
            .find(|s| s.external_id == ExternalId::from("personal"))
            .unwrap()
            .quantity,
        12.25
    );
    assert_eq!(
        w.stockpiles
            .iter()
            .find(|s| s.external_id == ExternalId::from("output"))
            .unwrap()
            .quantity,
        12.25
    );
    assert_eq!(
        w.production_data.artisans[0].base.owner_id,
        Some("a".into())
    );
}
#[test]
fn physical_workers_cannot_split_while_their_fte_is_only_part_time() {
    let mut w = fixture();
    w.production_data.resource_operations[0].base.assignments=serde_json::from_value(json!([{"employment_id":"half-time","pop_id":"a","workers_count":600,"time_fraction":0.5,"worker_group_id":"same-people"}])).unwrap();
    let before = serde_json::to_value(&w).unwrap();
    let result = execute_pop_split(&mut w, &request()).unwrap();
    assert_eq!(
        result.blocked_reason.as_deref(),
        Some("unassigned_physical_workers_unresolved")
    );
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
#[test]
fn invalid_mobility_container_is_rejected_before_any_money_or_rows_publish() {
    let mut w = fixture();
    w.labor_data.pop_mobility = Some(json!("invalid"));
    let before = serde_json::to_value(&w).unwrap();
    assert!(execute_pop_split(&mut w, &request()).is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
#[test]
fn holds_stay_with_parent_and_source_metadata_survives() {
    let mut w = fixture();
    personal(&mut w);
    w.production_data.goods_reservations.push(
        serde_json::from_value(
            json!({"id":"held","stock_id":"personal","status":"held","quantity":10}),
        )
        .unwrap(),
    );
    let r = execute_pop_split(&mut w, &request()).unwrap();
    let stock = w
        .stockpiles
        .iter()
        .find(|s| Some(&s.owner_id) == r.child_id.as_ref())
        .unwrap();
    assert!((stock.quantity - 2.25 * 100. / 1001.).abs() < 1e-10);
    assert_eq!(stock.extra["author_formula"], json!("never replace"));
    assert_eq!(
        w.production_data.goods_reservations[0].stock_id,
        ExternalId::from("personal")
    );
}
#[test]
fn full_split_remaps_held_titles_all_accounts_and_native_owner_views() {
    let mut w = fixture();
    personal(&mut w);
    w.accounts.push(AccountBalance {
        id: AccountId(905),
        external_id: "reserve".into(),
        owner_id: "a".into(),
        currency_id: "OTHER".into(),
        purpose: "operating_capital".into(),
        balance: Money(101),
    });
    w.production_data.goods_reservations.push(
        serde_json::from_value(json!({"stock_id":"personal","status":"held","quantity":10}))
            .unwrap(),
    );
    w.market_data.sale_offers.push(serde_json::from_value(json!({"id":"offer","stock_id":"personal","seller_id":"a","goods_id":"grain","private_metadata":{"beneficiary_pop_id":"a"}})).unwrap());
    let mut r = request();
    r.workforce_count = 600;
    r.target.pop_type_id = Some("craftsmen".into());
    let receipt = execute_pop_split(&mut w, &r).unwrap();
    let child = receipt.child_id.unwrap();
    assert_eq!(
        w.pops
            .iter()
            .find(|p| p.external_id == ExternalId::from("a"))
            .unwrap()
            .size,
        0
    );
    assert_eq!(
        w.pops
            .iter()
            .find(|p| p.external_id == child)
            .unwrap()
            .pop_type,
        "craftsmen"
    );
    assert_eq!(
        w.accounts
            .iter()
            .find(|a| a.owner_id == child && a.currency_id == ExternalId::from("OTHER"))
            .unwrap()
            .balance,
        Money(101)
    );
    assert_eq!(
        w.production_data.goods_reservations[0].stock_id,
        ExternalId::from(format!("personal:{}", child.as_str().unwrap()))
    );
    assert_eq!(w.market_data.sale_offers[0].seller_id, child);
    assert_eq!(
        w.market_data.sale_offers[0].extra["private_metadata"]["beneficiary_pop_id"],
        child.json()
    );
    assert_eq!(
        w.stockpiles
            .iter()
            .find(|s| s.owner_id == child)
            .unwrap()
            .quantity,
        12.25
    );
    assert!(
        w.validate_links().errors.is_empty(),
        "{:?}",
        w.validate_links().errors
    );
}
#[test]
fn partial_warehouse_access_and_authorized_rent_contract_are_preserved() {
    for owned in [true, false] {
        let mut w = fixture();
        let mut f = w.production_data.storage_facilities[0].clone();
        f.id = "cohort-wh".into();
        f.owner_id = if owned { "a" } else { "landlord" }.into();
        f.access_contracts = if owned {
            vec![]
        } else {
            vec![serde_json::from_value(json!({"owner_id":"a","status":"approved","source":"author","approval_ref":"source","price_minor_per_space":"13","currency_id":"COIN","purpose":"personal_savings","author_terms":"unchanged"})).unwrap()]
        };
        w.production_data.storage_facilities.push(f);
        let child = execute_pop_split(&mut w, &request())
            .unwrap()
            .child_id
            .unwrap();
        let f = w.production_data.storage_facilities.last().unwrap();
        let c = f
            .access_contracts
            .iter()
            .find(|c| c.owner_id.as_ref() == Some(&child))
            .unwrap();
        assert_eq!(
            c.price_minor_per_space,
            Some(Money(if owned { 0 } else { 13 }))
        );
        if !owned {
            assert_eq!(c.extra["author_terms"], json!("unchanged"));
            assert_eq!(f.access_contracts[0].owner_id, Some("a".into()));
        }
    }
}
#[test]
fn property_pending_cargo_cash_hold_and_invalid_lease_block_without_writes() {
    for case in 0..5 {
        let mut w = fixture();
        match case{0=>w.mobility_data.production_beneficiaries.push(json!({"beneficiaries":[{"owner_id":"a"}]})),1=>w.mobility_data.shipments.push(json!({"id":"cargo","buyer_id":"a","status":"in_transit"})),2=>w.production_data.money_reservations.push(serde_json::from_value(json!({"owner_id":"a","currency_id":"COIN","purpose":"personal_savings","amount_minor":"0"})).unwrap()),_=>{let f=&mut w.production_data.storage_facilities[0];f.access_contracts.push(serde_json::from_value(if case==3{json!({"owner_id":"a","status":"approved"})}else{json!({"owner_id":"a","status":"approved","source":"author","approval_ref":"source","price_minor_per_space":"13","currency_id":"COIN","purpose":"personal_savings","fixed_fee_minor":"0"})}).unwrap());}}
        let before = serde_json::to_value(&w).unwrap();
        assert!(execute_pop_split(&mut w, &request())
            .unwrap()
            .blocked_reason
            .is_some());
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
    }
}
#[test]
fn distinct_numeric_and_string_property_ids_do_not_block_the_wrong_pop() {
    let mut w = fixture();
    let id = ExternalId::from(7u32);
    w.pops[0].external_id = id.clone();
    w.accounts[0].owner_id = id.clone();
    w.mobility_data
        .project_investments
        .push(json!({"investor_id":"7"}));
    let mut r = request();
    r.pop_id = id;
    assert!(execute_pop_split(&mut w, &r).unwrap().child_id.is_some());
}
#[test]
fn invalid_workforce_literacy_and_oversubscribed_goods_reject_atomically() {
    for case in 0..4 {
        let mut w = fixture();
        personal(&mut w);
        let mut r = request();
        match case {
            0 => r.workforce_count = 601,
            1 => w.pops[0].workforce = 599.5,
            2 => {
                w.pops[0].extra.insert("literate_count".into(), json!(1002));
            }
            _ => w.production_data.goods_reservations.push(
                serde_json::from_value(
                    json!({"stock_id":"personal","status":"held","quantity":13}),
                )
                .unwrap(),
            ),
        }
        let before = serde_json::to_value(&w).unwrap();
        let result = execute_pop_split(&mut w, &r);
        assert!(result.is_err() || result.unwrap().blocked_reason.is_some());
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
    }
}
#[test]
fn orphan_history_and_late_property_change_cannot_partially_publish() {
    let mut w = fixture();
    let p = prepare_pop_split(&w, &request()).unwrap();
    w.mobility_data
        .project_investments
        .push(json!({"investor_id":"a"}));
    let before = serde_json::to_value(&w).unwrap();
    assert!(apply_pop_split(&mut w, p).is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
    let mut w = fixture();
    w.production_data
        .committed_operations
        .push(kingmaker_core::operations::CommittedOperation {
            event_id: request().event_id.into(),
            value: json!({"orphan":true}),
        });
    let before = serde_json::to_value(&w).unwrap();
    assert!(execute_pop_split(&mut w, &request()).is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
#[test]
fn partial_population_literates_savings_and_oldest_history_match_js() {
    let mut w = fixture();
    let r = request();
    let mut snapshot = common::snapshot(&w);
    snapshot["registry"]["popMobility"] = w.labor_data.pop_mobility.clone().unwrap();
    let source = common::oracle(
        json!({"action":"cohort_split","snapshot":snapshot,"options":{"eventId":r.event_id,"popId":"a","workforceCount":60}}),
    );
    let total: i64 = w.accounts.iter().map(|a| a.balance.0).sum();
    let date = w.campaign.clone();
    let receipt = execute_pop_split(&mut w, &r).unwrap();
    assert_eq!(receipt.people_moved, 100);
    assert_eq!(
        receipt.child_id.as_ref().unwrap().json(),
        source["result"]["pop_id"]
    );
    let child = w
        .pops
        .iter()
        .find(|p| Some(&p.external_id) == receipt.child_id.as_ref())
        .unwrap();
    assert_eq!(child.size, 100);
    assert_eq!(child.workforce, 60.);
    assert_eq!(child.extra["literate_count"], json!(33));
    let parent = w
        .pops
        .iter()
        .find(|p| p.external_id == ExternalId::from("a"))
        .unwrap();
    assert_eq!(parent.size, 901);
    assert_eq!(parent.workforce, 540.);
    assert_eq!(parent.extra["literate_count"], json!(300));
    assert_eq!(w.accounts.iter().map(|a| a.balance.0).sum::<i64>(), total);
    let cash = w
        .accounts
        .iter()
        .find(|a| a.owner_id == child.external_id)
        .unwrap();
    assert_eq!(cash.balance, Money(999));
    assert_eq!(child.account_id, Some(cash.id));
    assert_eq!(w.campaign, date);
    assert_eq!(
        w.labor_data.pop_mobility.as_ref().unwrap()["unemployment"]
            [&child.external_id.as_str().unwrap()][0]["weeks"],
        json!(3)
    );
}
#[test]
fn replay_and_changed_request_or_stale_savings_do_not_duplicate_children() {
    let mut w = fixture();
    let r = request();
    let p = prepare_pop_split(&w, &r).unwrap();
    w.accounts[0].balance = Money(9999);
    let before = serde_json::to_value(&w).unwrap();
    assert!(apply_pop_split(&mut w, p).is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
    let mut w = fixture();
    let receipt = execute_pop_split(&mut w, &r).unwrap();
    let before = serde_json::to_value(&w).unwrap();
    assert_eq!(execute_pop_split(&mut w, &r).unwrap(), receipt);
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
    let mut different = r;
    different.workforce_count = 61;
    assert!(execute_pop_split(&mut w, &different).is_err());
}
#[test]
fn protected_or_linked_rights_block_without_money_or_population_mutation() {
    for case in 0..4 {
        let mut w = fixture();
        if case < 3 {
            w.pops[0].pop_type_id = ["aristocrats", "capitalists", "slaves"][case].into();
        } else {
            w.mobility_data
                .project_investments
                .push(json!({"id":"investment","investor_id":"a","amount_minor":"10"}));
        }
        let before = serde_json::to_value(&w).unwrap();
        let result = execute_pop_split(&mut w, &request()).unwrap();
        assert!(result.blocked_reason.is_some());
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
    }
}
#[test]
fn partial_stock_title_and_claim_shares_keep_location_and_real_money() {
    let mut w = fixture();
    let mut stock = w.stockpiles[0].clone();
    stock.id = 900;
    stock.external_id = "personal".into();
    stock.owner_id = "a".into();
    stock.quantity = 12.25;
    stock.province_id = ProvinceId(1);
    stock.facility_id = Some("wh:a".into());
    stock.beneficiary_pop_id = Some("a".into());
    w.stockpiles.push(stock);
    w.production_data.obligations.push(serde_json::from_value(json!({"obligation_id":"claim","payer_id":"a","payee_id":"creditor","currency_id":"COIN","purpose":"personal_savings","amount_minor":"1001"})).unwrap());
    let result = execute_pop_split(&mut w, &request()).unwrap();
    let child = result.child_id.unwrap();
    let moved = w.stockpiles.iter().find(|s| s.owner_id == child).unwrap();
    assert_eq!(moved.province_id, ProvinceId(1));
    assert!((moved.quantity - 12.25 * 100. / 1001.).abs() < 1e-10);
    assert_eq!(moved.beneficiary_pop_id, Some(child.clone()));
    let left = w
        .stockpiles
        .iter()
        .find(|s| s.external_id == ExternalId::from("personal"))
        .unwrap();
    assert!((left.quantity + moved.quantity - 12.25).abs() < 1e-10);
    assert_eq!(
        w.production_data
            .obligations
            .iter()
            .map(|d| d.amount_minor.0)
            .sum::<i64>(),
        1001
    );
    assert!(w
        .production_data
        .obligations
        .iter()
        .any(|d| d.payer_id == child && d.amount_minor == Money(100)));
}
