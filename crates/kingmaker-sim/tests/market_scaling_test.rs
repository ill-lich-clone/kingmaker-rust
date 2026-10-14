//! Synthetic RAM-only allocation checks. No database, year, or wall-clock threshold.
mod common {
    pub mod market;
}
use common::market::*;
use kingmaker_core::{market::*, production::ProductionRequest, *};
use kingmaker_sim::{
    market_cycle::execute_market_phase, operation_session::OperationSession,
    production_validation::maintenance_verified,
};
use serde_json::json;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Mutex,
};
static TRACK: AtomicBool = AtomicBool::new(false);
static BYTES: AtomicUsize = AtomicUsize::new(0);
static MEASURE: Mutex<()> = Mutex::new(());
struct Allocator;
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if TRACK.load(Ordering::Relaxed) {
            BYTES.fetch_add(l.size(), Ordering::Relaxed);
        }
        System.alloc(l)
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        if TRACK.load(Ordering::Relaxed) {
            BYTES.fetch_add(l.size(), Ordering::Relaxed);
        }
        System.alloc_zeroed(l)
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        if TRACK.load(Ordering::Relaxed) {
            BYTES.fetch_add(n, Ordering::Relaxed);
        }
        System.realloc(p, l, n)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        System.dealloc(p, l)
    }
}
#[global_allocator]
static ALLOCATOR: Allocator = Allocator;
fn world(n: usize, slaves: bool) -> WorldState {
    let mut w = market_world();
    let pop = w.pops[0].clone();
    let cash = w.accounts[0].clone();
    let reference = w.market_data.needs.as_ref().unwrap().profile_refs[0].clone();
    let seller = w.accounts[2].clone();
    w.pops.clear();
    w.accounts.clear();
    w.market_data.needs.as_mut().unwrap().profile_refs.clear();
    w.market_data.needs.as_mut().unwrap().profiles[0].norms[0]
        .extra
        .insert("quantity_per_person_week".into(), json!(0.001));
    w.stockpiles[0].quantity = n as f64 * 1.03;
    w.production_data.storage_facilities[2].capacity = Some(n as f64 * 2.0);
    w.accounts.push(seller);
    for i in 0..n {
        let id: ExternalId = format!("pop:{i}").into();
        let mut p = pop.clone();
        p.external_id = id.clone();
        p.id = PopId(i as u32 + 1);
        p.account_id = None;
        p.account_status = "absent_in_source".into();
        if slaves {
            p.pop_type_id = "slaves".into();
            w.market_data.maintenance_agreements.push(
                serde_json::from_value(
                    json!({"pop_id":id,"payer_id":"payer","currency_id":"COIN"}),
                )
                .unwrap(),
            );
        } else {
            let mut a = cash.clone();
            a.id = AccountId(i as u32 + 10);
            a.owner_id = id.clone();
            a.external_id = format!("cash:{i}").into();
            w.accounts.push(a);
        }
        w.pops.push(p);
        let mut r = reference.clone();
        r.pop_id = id;
        w.market_data.needs.as_mut().unwrap().profile_refs.push(r);
    }
    if slaves {
        let mut a = cash;
        a.id = AccountId(n as u32 + 10);
        a.owner_id = "payer".into();
        a.purpose = "sponsored_maintenance_reserve".into();
        a.balance = Money(n as i64 * 1000);
        w.accounts.push(a);
    }
    w.provinces[0].pop_range = (0, n as u32);
    w.provinces[1].pop_range = (n as u32, n as u32);
    w.rebuild_account_index().unwrap();
    w
}
fn allocated(n: usize, maintenance: bool) -> usize {
    let _guard = MEASURE.lock().unwrap();
    let mut w = world(n, maintenance);
    let req = MarketRequest {
        expected_revision: 20,
        period: "w".into(),
        stage: if maintenance {
            MarketStage::Maintenance
        } else {
            MarketStage::EarlyFood
        },
        profile: Default::default(),
        priorities: None,
    };
    if maintenance {
        let receipt = execute_market_phase(&mut w, &req).unwrap();
        let s = OperationSession::new(&w).unwrap();
        let p = ProductionRequest {
            expected_revision: 20,
            period: "w".into(),
            policies: Default::default(),
            profile: Default::default(),
            maintenance_receipts: vec![],
        };
        BYTES.store(0, Ordering::Relaxed);
        TRACK.store(true, Ordering::Relaxed);
        for r in &receipt.maintenance_receipts {
            assert!(maintenance_verified(&s, &p, r));
        }
        TRACK.store(false, Ordering::Relaxed);
    } else {
        BYTES.store(0, Ordering::Relaxed);
        TRACK.store(true, Ordering::Relaxed);
        let result = execute_market_phase(&mut w, &req);
        TRACK.store(false, Ordering::Relaxed);
        assert!(result.is_ok(), "{result:?}");
    }
    BYTES.load(Ordering::Relaxed)
}
#[test]
fn market_policy_allocations_grow_linearly() {
    let small = allocated(512, false);
    let large = allocated(1024, false);
    println!("market allocated bytes:512={small},1024={large}");
    assert!(
        large * 2 < small * 5,
        "market allocated bytes:512={small},1024={large}"
    );
}
#[test]
fn all_maintenance_verifications_resolve_the_session_once() {
    let small = allocated(40, true);
    let large = allocated(80, true);
    assert!(
        large < small * 3,
        "maintenance allocated bytes:40={small},80={large}"
    );
}
