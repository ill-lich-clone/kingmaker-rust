//! Compare optimized helpers against the pre-patch order and arithmetic.
use super::*;
use kingmaker_core::{AccountId, Goods, ProvinceId, WorldState};

fn reference_coalesce(rows: Vec<(usize, f64)>) -> Vec<(usize, f64)> {
    let mut allocated: Vec<(usize, f64)> = Vec::new();
    for (i, q) in rows {
        if let Some((_, n)) = allocated.iter_mut().find(|(s, _)| s == &i) {
            *n += q;
        } else {
            allocated.push((i, q));
        }
    }
    allocated
}
fn bits(rows: &[(usize, f64)]) -> Vec<(usize, u64)> {
    rows.iter().map(|(i, q)| (*i, q.to_bits())).collect()
}
fn order(id: &str, quantity: f64) -> MarketOrder {
    serde_json::from_value(json!({
        "id":id, "owner_id":"buyer", "province_id":1, "currency_id":"coin",
        "purpose":"personal_savings", "goods_id":"iron", "quantity":quantity,
        "priority":"basic", "consume":true
    })).unwrap()
}
fn bid(id: &str, requested: i64, candidates: Vec<usize>) -> Bid {
    Bid {
        order: order(id, 1.0), event: id.into(), price: 1.0,
        requested: BigInt::from(requested), budget: Money(0), own: false,
        candidates, wanted: 0.0, fulfilled: 0.0, allocations: vec![],
    }
}

#[test]
fn perf_coalescing_preserves_first_seen_order_and_float_bits() {
    let cases = vec![
        vec![], vec![(9, -0.0)], vec![(3, 0.0), (3, -0.0)],
        vec![(9, 1e16), (2, 3.0), (9, 1.0), (9, -1e16), (2, 0.25)],
        vec![(8, f64::from_bits(1)), (8, f64::from_bits(1)), (1, 0.1), (1, 0.2)],
    ];
    for rows in cases {
        assert_eq!(bits(&coalesce_allocations(rows.clone())), bits(&reference_coalesce(rows)));
    }
}

#[test]
fn perf_coalescing_matches_reference_for_deterministic_generated_sequences() {
    let mut state = 0x5a17_4e31_1234_9876u64;
    for case in 0..256 {
        let mut rows = Vec::new();
        for _ in 0..(case * 3) {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            let key = (state % 67) as usize;
            let q = ((state >> 12) % 1_000_000) as f64 / 97.0;
            rows.push((key, q));
        }
        assert_eq!(bits(&coalesce_allocations(rows.clone())), bits(&reference_coalesce(rows)), "case {case}");
    }
}

#[test]
fn perf_budget_assignment_moves_candidates_and_preserves_allocation_order() {
    let rows = vec![bid("z", 101, vec![1, 3, 8]), bid("a", 103, vec![9, 2])];
    let pointers: HashMap<_, _> = rows.iter().map(|b| (b.order.id.clone(), b.candidates.as_ptr())).collect();
    let expected = allocate_big_budget(
        &rows.iter().map(|b| (ExternalId::from(b.order.id.clone()), b.requested.clone())).collect::<Vec<_>>(),
        Money(99),
    ).unwrap();
    let actual = fund_bids(rows, Money(99)).unwrap();
    assert_eq!(actual.len(), expected.len());
    for (b, a) in actual.iter().zip(&expected) {
        assert_eq!(b.order.id.as_str(), a.id.as_str().unwrap());
        assert_eq!(b.budget, a.paid);
        assert_eq!(b.candidates.as_ptr(), pointers[&b.order.id], "candidate vector was cloned");
    }
}

fn market_world() -> WorldState {
    let mut w = WorldState::default();
    w.accounts.push(kingmaker_core::AccountBalance {
        id: AccountId(1), external_id: "wallet".into(), owner_id: "buyer".into(),
        currency_id: "coin".into(), purpose: "personal_savings".into(), balance: Money(100_000),
    });
    w.stockpiles = vec![
        StockpileRecord {
            id: 2, external_id: "s-a".into(), province_id: ProvinceId(1),
            goods: Goods::from_external("iron".into()), owner_id: "seller-a".into(),
            currency_id: Some("coin".into()), quantity: 30.0, ..Default::default()
        },
        StockpileRecord {
            id: 3, external_id: "s-b".into(), province_id: ProvinceId(1),
            goods: Goods::from_external("iron".into()), owner_id: "seller-b".into(),
            currency_id: Some("coin".into()), quantity: 70.0, ..Default::default()
        },
    ];
    w.market_data.price_calibration = Some(serde_json::from_value(json!({"pricesByCurrency":{"coin":{"iron":2.0}}})).unwrap());
    w
}

#[test]
fn perf_public_market_planning_keeps_input_and_matches_owned_path() {
    let w = market_world();
    let session = OperationSession::new(&w).unwrap();
    let view = MarketView::from_session(&session).unwrap();
    let stocks = view.stocks.clone();
    let cash = view.cash.clone();
    let events = view.events.clone();
    let orders = vec![order("z", 10.0), order("a", 20.0)];
    let priorities = vec!["basic".into()];
    let public = plan_local_orders(&view, "p", &orders, &priorities).unwrap();
    assert_eq!(view.stocks, stocks);
    assert_eq!(view.cash, cash);
    assert_eq!(view.events, events);
    let mut owned = view.clone();
    let direct = plan_local_orders_in_place(&mut owned, "p", &orders, &priorities).unwrap();
    assert_eq!(public.operations, direct.operations);
    assert_eq!(public.fulfilled, direct.fulfilled);
    assert_eq!(public.fulfilled.get("a"), Some(&20.0));
    assert_eq!(public.fulfilled.get("z"), Some(&10.0));
    assert_ne!(owned.cash, cash);
    let again = plan_local_orders(&view, "p", &orders, &priorities).unwrap();
    assert_eq!(again.operations, public.operations);
}

#[test]
fn perf_public_market_error_leaves_input_unchanged() {
    let w = market_world();
    let session = OperationSession::new(&w).unwrap();
    let view = MarketView::from_session(&session).unwrap();
    let stocks = view.stocks.clone();
    let cash = view.cash.clone();
    assert!(plan_local_orders(&view, "p", &[order("same", 1.0), order("same", 2.0)], &["basic".into()]).is_err());
    assert_eq!(view.stocks, stocks);
    assert_eq!(view.cash, cash);
}

#[test]
#[ignore = "manual release microbenchmark; not a full-world benchmark"]
fn perf_coalescing_microbenchmark() {
    use std::{hint::black_box, time::Instant};
    for n in [1000, 2000, 4000, 8000] {
        let rows = (0..n).map(|i| (i, 0.25)).collect::<Vec<_>>();
        let legacy_input = rows.clone();
        let started = Instant::now();
        let legacy = black_box(reference_coalesce(black_box(legacy_input)));
        let legacy_ns = started.elapsed().as_nanos();
        let started = Instant::now();
        let indexed = black_box(coalesce_allocations(black_box(rows)));
        let indexed_ns = started.elapsed().as_nanos();
        assert_eq!(bits(&indexed), bits(&legacy));
        println!("distinct_stocks={n}, legacy_ns={legacy_ns}, indexed_ns={indexed_ns}");
    }
}

include!("perf_market_reference.rs");

#[test]
fn perf_market_phase_matches_frozen_original_on_generated_fixtures() {
    for case in 0..96 {
        let mut w = market_world();
        w.accounts[0].balance = Money([0, 1, 101, 999, 100_000][case % 5]);
        w.stockpiles[0].quantity = [0.0, 1e-13, 0.125, 30.0][case % 4];
        w.stockpiles[1].quantity = [0.0, 7.5, 70.0][case % 3];
        if case % 7 == 0 { w.stockpiles[0].owner_id = "buyer".into(); }
        if case % 11 == 0 { w.stockpiles[1].currency_id = Some("other".into()); }
        let session = OperationSession::new(&w).unwrap();
        let view = MarketView::from_session(&session).unwrap();
        let mut expected_view = view.clone();
        let mut actual_view = view;
        let orders = vec![order("z", 2.0 + (case % 9) as f64), order("a", 5.0), order("10", 0.25)];
        let expected = reference_phase_clear(&mut expected_view, "phase", &orders).unwrap();
        let actual = phase_clear(&mut actual_view, "phase", &orders).unwrap();
        assert_eq!(actual.operations, expected.operations, "case {case}");
        assert_eq!(actual.fulfilled, expected.fulfilled, "case {case}");
        assert_eq!(actual_view.cash, expected_view.cash, "case {case}");
        assert_eq!(actual_view.stocks, expected_view.stocks, "case {case}");
        assert_eq!(actual_view.events, expected_view.events, "case {case}");
        assert_eq!(actual.proofs.len(), expected.proofs.len());
        for (id, (order, proof)) in &actual.proofs {
            let (original_order, original_proof) = &expected.proofs[id];
            assert_eq!(order, original_order);
            assert_eq!(proof.budget_minor, original_proof.budget_minor);
            let bits = |p: &PurchaseEvidence| p.allocations.iter()
                .map(|a| (a.stock_id.clone(), a.quantity.to_bits())).collect::<Vec<_>>();
            assert_eq!(bits(proof), bits(original_proof), "case {case}");
        }
    }
}
