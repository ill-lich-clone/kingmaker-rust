//! Explicit release-only measurements; excluded from ordinary regression runs.
#[path="common/production.rs"] mod production;
#[path="common/shipping.rs"] mod shipping;
#[path="common/construction.rs"] mod construction;
use kingmaker_core::*;
use std::time::Instant;
fn report(name:&str,n:usize,mut run:impl FnMut()) {
    let mut samples=vec![];
    for trial in 0..6 {let start=Instant::now();run();let elapsed=start.elapsed().as_secs_f64();assert!(elapsed<60.,"local case exceeded 60 seconds");if trial>0{samples.push(elapsed);}}
    samples.sort_by(f64::total_cmp);
    println!("{}",serde_json::json!({"probe":name,"n":n,"median_s":samples[2],"samples_s":samples,"allocator_counter":false,"includes_fixture_clone":!name.starts_with("actual_storage")}));
}
fn producers(n:usize)->WorldState {
    let mut w=production::world();let pop=w.pops[0].clone();let factory=w.production_data.factories[0].clone();
    w.pops.clear();w.production_data.factories.clear();
    for i in 0..n {let mut p=pop.clone();p.id=PopId((i+1) as u32);p.external_id=format!("p{i}").into();
        let mut f=factory.clone();f.base.id=format!("f{i}").into();f.base.assignments[0].pop_id=p.external_id.clone();f.base.assignments[0].employment_id=Some(format!("e{i}").into());
        w.pops.push(p);w.production_data.factories.push(f);
    }
    w.provinces[0].pop_range=(0,n as u32);w.accounts[0].balance=Money(20*n as i64);
    for s in &mut w.stockpiles {s.quantity*=n as f64;}
    w.production_data.storage_facilities[0].capacity=Some(100.*n as f64);
    w.refresh_enterprise_views().unwrap();w
}
#[test]
#[ignore="run explicitly in release, sequentially after local correctness gates"]
fn remediation_release_phases() {
    assert!(!cfg!(debug_assertions));
    for n in [16,32,64] {
        let initial=producers(n);let total=initial.accounts.iter().map(|a|a.balance.0 as i128).sum::<i128>();
        report("actual_production_cycle",n,||{let mut w=initial.clone();let receipt=kingmaker_sim::production::execute_production_cycle(&mut w,&production::request()).unwrap();
            assert_eq!(receipt.outputs.len(),n);assert!(receipt.invariants.production_required_passed(),"{:?}",receipt.invariants);assert!(matches!(receipt.invariants.rights,kingmaker_core::audit::CheckStatus::NotMeasured{..}),"preserve existing explicit legal-title measurement limit");assert!((receipt.outputs.iter().map(|o|o.quantity).sum::<f64>()-n as f64).abs()<1e-7);
            assert_eq!(w.accounts.iter().map(|a|a.balance.0 as i128).sum::<i128>(),total);
        });
    }
    for n in [1000,2000,4000] {
        let mut initial=shipping::world(3.);
        initial.stockpiles.extend((0..n).map(|i|StockpileRecord{id:10000+i as u32,external_id:format!("neutral-{i}").into(),quantity:1.,..Default::default()}));
        report("actual_arrivals_neutral_stocks",n,||{let mut w=initial.clone();let receipt=kingmaker_sim::shipment_arrivals::execute_shipment_arrivals(&mut w,&shipping::request("probe")).unwrap();assert!(receipt.invariants.all_required_passed());assert_eq!(w.mobility_data.shipments[0]["status"],"arrived");});
        let mut initial=construction::ready(7);
        initial.production_data.construction_projects.extend((0..n).map(|i|serde_json::from_value(serde_json::json!({"id":format!("waiting-{i}"),"status":"awaiting_funding","owner_id":"owner","terms":{"rule":7}})).unwrap()));
        report("actual_construction_one_active_waiting_projects",n,||{let mut w=initial.clone();let receipt=kingmaker_sim::construction_cycle::execute_construction(&mut w,&construction::request()).unwrap();assert!(receipt.invariants.all_required_passed());assert_eq!(w.production_data.construction_projects[0].status,"completed");});
        let mut w=production::world();let mut facility=w.production_data.storage_facilities[0].clone();facility.extra.insert("capacity_rule".into(),serde_json::json!({"opening_space":2,"minimum_capacity":1,"buffer_weeks":12}));
        w.production_data.factories.clear();w.production_data.storage_facilities=(0..n).map(|i|{let mut f=facility.clone();f.id=format!("wh{i}").into();f}).collect();
        report("actual_storage_capacity_no_producers",n,||{let result=kingmaker_sim::warehouse_capacity::derive_storage_capacities(&w).unwrap();assert_eq!(result.len(),n);assert!(result.iter().all(|f|f.capacity==Some(2.)));});
    }
}

#[test]
#[ignore="isolated longer samples distinguish short-probe noise from a capacity regression"]
fn remediation_release_storage_isolated() {
    assert!(!cfg!(debug_assertions));
    for n in [1000,2000,4000] {
        let mut w=production::world();let mut facility=w.production_data.storage_facilities[0].clone();
        facility.extra.insert("capacity_rule".into(),serde_json::json!({"opening_space":2,"minimum_capacity":1,"buffer_weeks":12}));
        w.production_data.factories.clear();w.production_data.storage_facilities=(0..n).map(|i|{let mut f=facility.clone();f.id=format!("wh{i}").into();f}).collect();
        report("actual_storage_capacity_isolated_batch32",n,||{
            for _ in 0..32 {let result=kingmaker_sim::warehouse_capacity::derive_storage_capacities(&w).unwrap();assert_eq!(result.len(),n);assert!(result.iter().all(|f|f.capacity==Some(2.)));}
        });
    }
}
