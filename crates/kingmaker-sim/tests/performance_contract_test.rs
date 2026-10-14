mod common { pub mod allocation_meter; pub mod market; }
use common::{allocation_meter::*, market::market_world};
use kingmaker_core::{market::*, production::ProductionRequest, *};
use kingmaker_sim::{market_cycle::execute_market_phase, production::execute_production_cycle};
use serde_json::json;
#[global_allocator]
static ALLOCATOR: Allocator = Allocator;

fn surplus_world(n: usize) -> WorldState {
    let mut w = market_world();
    w.pops.clear();
    for p in &mut w.provinces { p.pop_range = (0, 0); }
    w.market_data.needs.as_mut().unwrap().profile_refs.clear();
    let raw = w.stockpiles[0].clone();
    let facility = w.production_data.storage_facilities[2].clone();
    w.stockpiles.clear();
    w.production_data.storage_facilities.clear();
    for i in 0..n {
        let mut f = facility.clone();
        f.id = format!("warehouse:{i}").into();
        f.capacity = Some(50.0);
        f.extra.insert("author_notes".into(), json!("a".repeat(4096)));
        let mut s = raw.clone();
        s.id = (i + 1) as u32;
        s.external_id = format!("grain:{i}").into();
        s.facility_id = Some(f.id.clone());
        s.quantity = 10.3;
        w.stockpiles.push(s);
        w.production_data.storage_facilities.push(f);
    }
    execute_production_cycle(&mut w, &ProductionRequest {
        expected_revision: 20, period: "w".into(), policies: Default::default(),
        profile: Default::default(), maintenance_receipts: vec![],
    }).unwrap();
    w
}
fn closing_allocated(n: usize) -> usize {
    let mut w = surplus_world(n);
    let request = MarketRequest { expected_revision: 20, period: "w".into(),
        stage: MarketStage::ClosingMarket, profile: Default::default(), priorities: None };
    let meter = Measurement::start();
    let result = execute_market_phase(&mut w, &request);
    let (bytes, peak) = meter.finish();
    result.unwrap();
    // Hand-derived: each 10.3 tonnes of grain produces exactly 10 tonnes flour.
    let grain: f64 = w.stockpiles.iter().filter(|s| s.goods.external_id().as_str()==Some("grain")).map(|s|s.quantity).sum();
    let flour: f64 = w.stockpiles.iter().filter(|s| s.goods.external_id().as_str()==Some("flour")).map(|s|s.quantity).sum();
    assert!(grain.abs()<1e-8);
    assert!((flour-10.0*n as f64).abs()<1e-8);
    assert!(w.production_data.storage_facilities.iter().all(|f| f.extra["author_notes"].as_str().unwrap().len()==4096));
    println!("closing surplus n={n}: allocated={bytes}, peak_additional_live={peak}");
    bytes
}
#[test]
fn closing_surplus_allocations_do_not_grow_quadratically_with_stocks_and_facilities() {
    let _guard = MEASURE.lock().unwrap();
    let startup = closing_allocated(0);
    let a = closing_allocated(128).checked_sub(startup).unwrap();
    let b = closing_allocated(256).checked_sub(startup).unwrap();
    let c = closing_allocated(512).checked_sub(startup).unwrap();
    assert!(a>0 && b>0);
    assert!(b*2<=a*5, "128={a}, 256={b}: allocation growth exceeds 2.5x");
    assert!(c*2<=b*5, "256={b}, 512={c}: allocation growth exceeds 2.5x");
}

#[test]
fn cancel_inside_surplus_keeps_published_world() {
    let _lock=MEASURE.lock().unwrap();
    let mut w=surplus_world(600);
    let before=serde_json::to_value(&w).unwrap();
    let hits=std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen=hits.clone();
    let control=kingmaker_sim::run_control::RunControl::new(move |label| {
        if label=="food_surplus" && seen.fetch_add(1,std::sync::atomic::Ordering::Relaxed)==1 {
            Err(kingmaker_sim::operation_session::OperationError("CANCELLED_BY_OPERATOR".into()))
        } else {Ok(())}
    });
    let request=MarketRequest {expected_revision:20,period:"w".into(),stage:MarketStage::ClosingMarket,profile:Default::default(),priorities:None};
    let result=control.run(||execute_market_phase(&mut w,&request));
    assert!(result.unwrap_err().to_string().contains("CANCELLED_BY_OPERATOR"));
    assert_eq!(hits.load(std::sync::atomic::Ordering::Relaxed),2);
    assert_eq!(serde_json::to_value(&w).unwrap(),before);
}
