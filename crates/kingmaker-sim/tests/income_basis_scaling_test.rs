#[path="common/labor.rs"] mod common;
#[path="common/allocation_meter.rs"] mod meter;
#[global_allocator] static ALLOC:meter::Allocator=meter::Allocator;
use kingmaker_core::income::IncomeRequest;
use kingmaker_sim::income_basis::capture_income_opening;
#[test]
fn opening_basis_retains_income_metadata_without_unrelated_author_payload() {
    let _lock=meter::MEASURE.lock().unwrap();let mut w=common::world(1540);
    w.stockpiles[0].extra.insert("author_notes".into(),serde_json::json!("x".repeat(1024*1024)));
    w.mobility_data.shipments.push(serde_json::json!({"id":"cargo","goods_id":"grain","remaining_quantity":2.5,"producer_contributions":[{"producer_id":"p","quantity":2.5}],"author_notes":"y".repeat(1024*1024)}));
    let req=IncomeRequest {expected_revision:w.campaign.revision,period:"w".into(),policies:Default::default()};
    let m=meter::Measurement::start();let basis=capture_income_opening(&w,&req).unwrap();let (bytes,_)=m.finish();
    assert!(bytes<128*1024,"unrelated opening payload copied: {bytes}");
    let frozen=serde_json::to_value(basis).unwrap();
    assert_eq!(frozen["stocks"][0]["owner_id"],w.stockpiles[0].owner_id.json());
    assert_eq!(frozen["stocks"][0]["quantity"],serde_json::json!(w.stockpiles[0].quantity));
    assert_eq!(frozen["shipments"][0]["producer_contributions"],w.mobility_data.shipments[0]["producer_contributions"]);
    assert_eq!(w.stockpiles[0].extra["author_notes"].as_str().unwrap().len(),1024*1024);
}
