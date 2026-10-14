#[path = "common/production.rs"]
mod common;
#[path = "common/income.rs"]
mod income_common;
use kingmaker_core::{construction::ConstructionRequest, production::ProductionPolicies, *};
use kingmaker_sim::{
    budget_cycle::execute_budget_cycle,
    investments::{
        apply_automatic_investments, execute_automatic_investments, prepare_automatic_investments,
    },
};
use serde_json::json;
fn request() -> ConstructionRequest {
    ConstructionRequest {
        expected_revision: 20,
        period: "w".into(),
        policies: ProductionPolicies::default(),
    }
}
fn fixture(cost: i64, budget: i64, reserve: i64) -> WorldState {
    let mut w = common::world();
    w.production_data.factories.clear();
    w.pops[0].employed = 0.;
    w.accounts
        .push(common::account(3, "a", "personal_savings", 3000));
    w.accounts
        .push(common::account(4, "b", "personal_savings", 7000));
    w.rebuild_account_index().unwrap();
    w.mobility_data.capitalist_entities = vec![
        json!({"id":"a","owner_id":"a","currency_id":"COIN","account_purpose":"personal_savings","implemented_technologies":[]}),
        json!({"id":"b","owner_id":"b","currency_id":"COIN","account_purpose":"personal_savings","implemented_technologies":[]}),
    ];
    w.market_data.territory.push(
        serde_json::from_value(json!({"province_id":1,"region_id":"r","owner_state_id":"s"}))
            .unwrap(),
    );
    w.production_data.catalog.as_mut().unwrap().methods[0].extra.insert("construction".into(),json!({"status":"ready","source":"fixture","cost_minor":cost.to_string(),"currency_id":"COIN","duration_days":30,"labor_required_fte_days":7,"cash_cost_basis":"includes_wages","provider_id":"builder","materials":[]}));
    w.construction_data.investment_opportunities = vec![
        json!({"id":"offer","status":"source","source":"fixture","method_id":"factory:steel","province_id":1,"currency_id":"COIN","demand":10,"margin_minor":"100","workforce_available":1,"inputs_accessible":true,"terms":{"version":1,"source":"fixture","budget_minor":budget.to_string(),"operating_reserve_minor":reserve.to_string(),"currency_id":"COIN","share_basis":"cash_contribution","refund_rule":"pro_rata_unspent"},"assignments":[],"future_assignments":[],"factory_configuration":{"facility_id":"new-wh","people_slots":40000,"capacity_source":"fixture","equipment_capacity_weekly":100,"equipment_source":"fixture"}}),
    ];
    execute_budget_cycle(
        &mut w,
        &kingmaker_core::budget::BudgetRequest {
            expected_revision: 20,
            period: "w".into(),
            policies: Default::default(),
        },
    )
    .unwrap();
    w
}
fn project(w: &WorldState) -> serde_json::Value {
    serde_json::to_value(&w.production_data.construction_projects[0]).unwrap()
}
fn money(w: &WorldState, owner: &str, purpose: &str) -> i64 {
    w.accounts
        .iter()
        .filter(|a| a.owner_id == ExternalId::from(owner) && a.purpose == purpose)
        .map(|a| a.balance.0)
        .sum()
}
#[test]
fn actual_two_investors_fund_one_project_protect_startup_money_and_replay_once() {
    let mut w = fixture(1000, 10000, 4000);
    let before = w.accounts.iter().map(|a| a.balance.0).sum::<i64>();
    let r = request();
    let receipt = execute_automatic_investments(&mut w, &r).unwrap();
    assert_eq!(w.production_data.construction_projects.len(), 1);
    let p = project(&w);
    assert_eq!(p["funded_minor"], json!("10000"));
    assert_eq!(p["status"], json!("funded"));
    assert_eq!(
        p["shares"],
        json!([{"investor_id":"a","numerator":"3","denominator":"10"},{"investor_id":"b","numerator":"7","denominator":"10"}])
    );
    assert_eq!(
        money(&w, "project:private-project:offer", "construction_escrow"),
        6000
    );
    assert_eq!(
        money(
            &w,
            "project:private-project:offer",
            "factory_startup_reserve"
        ),
        4000
    );
    assert_eq!(w.mobility_data.project_investments.len(), 2);
    assert_eq!(w.accounts.iter().map(|a| a.balance.0).sum::<i64>(), before);
    assert!(receipt.invariants.all_required_passed());
    let before = serde_json::to_value(&w).unwrap();
    assert_eq!(execute_automatic_investments(&mut w, &r).unwrap(), receipt);
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
#[test]
fn reserve_limit_and_shared_account_use_real_remaining_cash() {
    let mut w = fixture(1000, 10000, 0);
    w.mobility_data.capitalist_entities[0]["policy"] =
        json!({"cash_reserve_minor":"1000","contribution_limit_minor":"1500"});
    w.mobility_data.capitalist_entities[1]["owner_id"] = json!("a");
    execute_automatic_investments(&mut w, &request()).unwrap();
    assert_eq!(project(&w)["funded_minor"], json!("3000"));
    assert_eq!(money(&w, "a", "personal_savings"), 0);
    assert_eq!(money(&w, "b", "personal_savings"), 7000);
    assert_eq!(
        w.mobility_data.project_investments[0]["amount_minor"],
        json!("1500")
    );
    assert_eq!(
        w.mobility_data.project_investments[1]["amount_minor"],
        json!("1500")
    );
}
#[test]
fn invalid_terms_or_late_investor_currency_never_partially_publish() {
    for case in 0..3 {
        let mut w = fixture(1000, 10000, 4000);
        match case {
            0 => w.construction_data.investment_opportunities[0]["terms"]["version"] = json!(0),
            1 => {
                w.construction_data.investment_opportunities[0]["terms"]
                    ["operating_reserve_minor"] = json!("9500")
            }
            _ => {
                w.mobility_data.capitalist_entities[1]["policy"] =
                    json!({"cash_reserve_minor":"-1"})
            }
        };
        let before = serde_json::to_value(&w).unwrap();
        assert!(execute_automatic_investments(&mut w, &request()).is_err());
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
    }
}
#[test]
fn prepared_investment_checks_opportunities_account_rights_and_required_budget_history() {
    for case in 0..4 {
        let mut w = fixture(1000, 10000, 4000);
        let prepared = prepare_automatic_investments(&w, &request()).unwrap();
        match case {
            0 => {
                w.construction_data.investment_opportunities[0]["terms"]["source"] =
                    json!("changed")
            }
            1 => w.accounts[2].balance.0 -= 1,
            2 => w.mobility_data.capitalist_entities[0]["owner_id"] = json!("new"),
            _ => w.budget_data.phase_receipts.clear(),
        };
        let before = serde_json::to_value(&w).unwrap();
        assert!(apply_automatic_investments(&mut w, prepared).is_err());
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
    }
}
#[test]
fn mini_load_preserves_contributions_terms_and_required_replay_history() {
    let mut w = fixture(1000, 10000, 4000);
    let r = request();
    let receipt = execute_automatic_investments(&mut w, &r).unwrap();
    let mut w: WorldState = serde_json::from_value(serde_json::to_value(&w).unwrap()).unwrap();
    w.rebuild_account_index().unwrap();
    assert_eq!(execute_automatic_investments(&mut w, &r).unwrap(), receipt);
    w.production_data.committed_operations.pop();
    let before = serde_json::to_value(&w).unwrap();
    assert!(execute_automatic_investments(&mut w, &r).is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}

fn assessed_fixture() -> WorldState {
    let mut w = fixture(1000, 10000, 4000);
    w.mobility_data.reference =
        Some(json!({"goods":[{"goods_id":"steel","Конечная норма / 1000 / неделю":100}]}));
    w.market_data.price_calibration = Some(
        serde_json::from_value(json!({"pricesByCurrency":{"COIN":{"steel":3,"iron":1,"coal":1}}}))
            .unwrap(),
    );
    w.construction_data.investment_opportunities[0]["assessment_policy"] =
        json!("native_weekly_v1");
    w.construction_data.investment_opportunities[0]["future_assignments"] =
        json!([{"pop_id":"p","contract_id":"c","workers_count":1,"time_fraction":1}]);
    w.production_data.policies.taxes.insert("s".into(), 0.1);
    for stock in &mut w.stockpiles {
        stock.currency_id = Some("COIN".into());
    }
    w.market_data.sale_offers=vec![serde_json::from_value(json!({"id":"iron-offer","stock_id":"iron","seller_id":"o","goods_id":"iron","status":"source","source":"fixture"})).unwrap(),serde_json::from_value(json!({"id":"coal-offer","stock_id":"coal","seller_id":"o","goods_id":"coal","status":"source","source":"fixture"})).unwrap()];
    w
}
fn assessment_oracle(w: &WorldState) -> serde_json::Value {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let mut snapshot = income_common::snapshot(w);
    snapshot["registry"]["reference"] = json!(w.mobility_data.reference);
    let mut p = Command::new("node")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/tests/construction-oracle.cjs"
        ))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    p.stdin.take().unwrap().write_all(json!({"action":"assessment","snapshot":snapshot,"opportunities":w.construction_data.investment_opportunities,"policies":w.production_data.policies}).to_string().as_bytes()).unwrap();
    let out = p.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn ordinary_native_contributions_projects_and_staff_activation_match_actual_js_source() {
    use std::{io::Write,process::{Command,Stdio}};
    let mut w=fixture(1000,10000,4000);let mut snapshot=income_common::snapshot(&w);
    snapshot["registry"]["investmentOpportunities"]=json!(w.construction_data.investment_opportunities);
    snapshot["registry"]["capitalistEntities"]=json!(w.mobility_data.capitalist_entities);
    snapshot["registry"]["projectInvestments"]=json!(w.mobility_data.project_investments);
    let mut p=Command::new("node").arg(concat!(env!("CARGO_MANIFEST_DIR"),"/../../scripts/tests/construction-oracle.cjs")).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    p.stdin.take().unwrap().write_all(json!({"action":"automatic","snapshot":snapshot,"period":"w","policies":w.production_data.policies}).to_string().as_bytes()).unwrap();
    let out=p.wait_with_output().unwrap();assert!(out.status.success(),"{}",String::from_utf8_lossy(&out.stderr));let expected:serde_json::Value=serde_json::from_slice(&out.stdout).unwrap();
    let receipt=execute_automatic_investments(&mut w,&request()).unwrap();
    let expected_ops:Vec<kingmaker_core::operations::EconomicOperation>=serde_json::from_value(expected["operations"].clone()).unwrap();
    let actual_ops:Vec<_>=receipt.operations.iter().filter(|op|op.kind!=kingmaker_core::operations::OperationKind::ConstructionPhaseBoundary).cloned().collect();
    assert_eq!(actual_ops,expected_ops);
    let expected_projects:Vec<kingmaker_core::production::ConstructionProjectLabor>=serde_json::from_value(expected["registry"]["constructionProjects"].clone()).unwrap();
    assert_eq!(w.production_data.construction_projects,expected_projects);
    assert_eq!(json!(w.mobility_data.project_investments),expected["registry"]["projectInvestments"]);
    assert_eq!(json!(receipt.decisions),expected["result"]["decisions"]);
}

#[test]
fn exact_small_tranches_have_no_protected_reserve_rounding_leak() {
    let mut w=fixture(2,7,3);
    w.accounts[2].balance=Money(1);w.accounts[3].balance=Money(2);
    w.accounts.push(common::account(5,"c","personal_savings",4));w.rebuild_account_index().unwrap();
    let mut c=w.mobility_data.capitalist_entities[0].clone();c["id"]=json!("c");c["owner_id"]=json!("c");w.mobility_data.capitalist_entities.push(c);
    execute_automatic_investments(&mut w,&request()).unwrap();
    assert_eq!(money(&w,"project:private-project:offer","construction_escrow"),4);
    assert_eq!(money(&w,"project:private-project:offer","factory_startup_reserve"),3);
    assert_eq!(project(&w)["shares"],json!([{"investor_id":"a","numerator":"1","denominator":"7"},{"investor_id":"b","numerator":"2","denominator":"7"},{"investor_id":"c","numerator":"4","denominator":"7"}]));
}

#[test]
fn unavailable_technology_approval_currency_and_full_region_create_no_project_or_debit() {
    for case in 0..4 {
        let mut w=fixture(1000,10000,4000);
        match case {
            0=>w.production_data.catalog.as_mut().unwrap().methods[0].technology.push("missing-tech".into()),
            1=>w.construction_data.investment_opportunities[0]["status"]=json!("approved"),
            2=>w.construction_data.investment_opportunities[0]["currency_id"]=json!("OTHER"),
            _=>{let f=common::world().production_data.factories[0].clone();for i in 0..8 {let mut f=f.clone();f.base.id=format!("f:{i}").into();f.base.assignments.clear();f.base.province_id=ProvinceId(99);w.production_data.factories.push(f);}},
        }
        let before=w.accounts.clone();let receipt=execute_automatic_investments(&mut w,&request()).unwrap();
        assert!(receipt.decisions.is_empty());assert!(w.production_data.construction_projects.is_empty());assert_eq!(w.accounts,before);
    }
}
#[test]
fn native_assessment_uses_current_prices_free_labor_and_consented_inputs_like_source() {
    for case in 0..5 {
        let mut w = assessed_fixture();
        match case {
            1 => w
                .market_data
                .price_calibration
                .as_mut()
                .unwrap()
                .prices_by_currency
                .get_mut("COIN")
                .unwrap()
                .insert("steel".into(), json!(0.01)),
            2 => {
                w.pops[0].workforce = 0.5;
                None
            }
            3 => {
                w.stockpiles.clear();
                w.market_data.sale_offers.clear();
                None
            }
            4 => {
                w.construction_data.investment_opportunities[0]
                    .as_object_mut()
                    .unwrap()
                    .remove("assessment_policy");
                None
            }
            _ => None,
        };
        let before = serde_json::to_value(&w).unwrap();
        let actual =
            kingmaker_sim::investment_assessment::assess_investment_opportunities(&w, &request())
                .unwrap();
        let mut expected = assessment_oracle(&w);
        for row in expected.as_array_mut().unwrap() {
            if row["assessment_policy"] != "native_weekly_v1" {
                continue;
            }
            for key in ["demand", "workforce_available"] {
                if let Some(n) = row[key].as_f64() {
                    row[key] = json!(n);
                }
            }
        }
        assert_eq!(json!(actual), expected, "assessment case {case}");
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
        if case == 0 {
            assert_eq!(actual[0]["margin_minor"], json!("126"));
            assert_eq!(actual[0]["demand"], json!(0.1));
        }
    }
}
#[test]
fn actual_automatic_investment_does_not_fund_stale_profitable_hints() {
    let mut w = assessed_fixture();
    w.market_data
        .price_calibration
        .as_mut()
        .unwrap()
        .prices_by_currency
        .get_mut("COIN")
        .unwrap()
        .insert("steel".into(), json!(0.01));
    let receipt = execute_automatic_investments(&mut w, &request()).unwrap();
    assert!(receipt.decisions.is_empty());
    assert!(w.production_data.construction_projects.is_empty());
    assert_eq!(money(&w, "a", "personal_savings"), 3000);
}

#[test]
fn funded_staff_transfer_uses_current_workers_and_a_late_stale_transfer_moves_nobody() {
    for stale in [false, true] {
        let mut w = fixture(1000, 10000, 4000);
        w.mobility_data.capitalist_entities.clear();
        w.construction_data.investment_opportunities.clear();
        w.pops[0].pop_type_id = "farmers".into();
        w.pops[0].pop_type = "farmers".into();
        w.pops[0].employed = 1.;
        w.production_data.resource_operations.push(serde_json::from_value(json!({"id":"raw","owner_id":"o","province_id":1,"method_id":"rgo:iron","resource_id":"iron","resource_status":"source","land_access":true,"assignments":[{"employment_id":"e","contract_id":"c","pop_id":"p","workers_count":1,"time_fraction":1}]})).unwrap());
        w.budget_data.employment.push(serde_json::from_value(json!({"employment_id":"e","contract_id":"c","pop_id":"p","workers_count":1,"time_fraction":1,"fte":1,"effective_labor":1})).unwrap());
        w.production_data.construction_projects.push(serde_json::from_value(json!({"id":"p-build","status":"funded","owner_id":"project:p-build","province_id":1,"planned_assignments":[{"employment_id":"builder-e","contract_id":"builder-c","pop_id":"p","workers_count":1,"time_fraction":1}],"staff_transfers":[{"source_rgo_id":"raw","employment_id":"e","pop_id":"p","expected_workers_count":1,"workers_count":1}],"assignments":[]})).unwrap());
        if stale {
            w.production_data.construction_projects[0].extra.get_mut("staff_transfers").unwrap().as_array_mut().unwrap().push(json!({"source_rgo_id":"missing","employment_id":"late","pop_id":"p","expected_workers_count":1,"workers_count":1}));
        }
        let raw = w.production_data.resource_operations.clone();
        let employment = w.budget_data.employment.clone();
        let receipt = execute_automatic_investments(&mut w, &request()).unwrap();
        if stale {
            assert_eq!(w.production_data.resource_operations, raw);
            assert_eq!(w.budget_data.employment, employment);
            assert!(w.production_data.construction_projects[0]
                .assignments
                .is_empty());
            assert!(receipt
                .diagnostics
                .iter()
                .any(|d| d["reason"] == "construction_staff_transfer_unavailable"));
        } else {
            assert_eq!(
                w.production_data.resource_operations[0].base.assignments[0].workers_count,
                0.
            );
            assert_eq!(w.budget_data.employment[0].fte, 0.);
            assert_eq!(
                w.production_data.construction_projects[0].assignments[0].workers_count,
                1.
            );
            assert_eq!(
                w.production_data.construction_projects[0].extra["staff_activated"],
                json!(true)
            );
        }
    }
}

#[path = "common/allocation_meter.rs"] mod investment_meter;
#[global_allocator] static INVESTMENT_ALLOC: investment_meter::Allocator = investment_meter::Allocator;
fn scaled_assessment(n: usize) -> WorldState {
    let mut w = assessed_fixture();
    let pop = w.pops[0].clone();
    for i in 1..n { let mut p=pop.clone(); p.external_id=format!("neutral-{i}").into(); p.id=PopId((i+1) as u32); w.pops.push(p); }
    let offer=w.construction_data.investment_opportunities[0].clone();
    w.construction_data.investment_opportunities=(0..n).map(|i| {let mut o=offer.clone();o["id"]=json!(format!("offer-{i}"));o}).collect();
    w
}
#[test]
fn investment_assessment_allocation_is_linear_in_people_and_offers() {
    let _lock=investment_meter::MEASURE.lock().unwrap();
    for n in [512,1024,2048] {
        let w=scaled_assessment(n);
        let measure=investment_meter::Measurement::start();
        let rows=kingmaker_sim::investment_assessment::assess_investment_opportunities(&w,&request()).unwrap();
        let (bytes,peak)=measure.finish();
        assert_eq!(rows.len(),n);
        eprintln!("assessment N={n} allocated={bytes} peak={peak}");
        assert!(bytes <= n*16384,"whole population copied per opportunity: N={n}, allocated={bytes}");
        assert!(rows.iter().all(|r|r["workforce_available"]==json!(1.0)));
    }
}
#[test]
fn investment_assessment_repeated_workers_and_independent_opportunities() {
    let mut w=assessed_fixture();
    w.pops[0].workforce=2.;
    let mut occupied=common::world().production_data.factories.remove(0);
    occupied.base.assignments[0].workers_count=1.;
    w.production_data.factories.push(occupied);
    let mut offer=w.construction_data.investment_opportunities[0].clone();
    offer["future_assignments"]=json!([
        {"pop_id":"p","contract_id":"c","workers_count":1,"time_fraction":0.75},
        {"pop_id":"p","contract_id":"c","workers_count":1,"time_fraction":0.75}]);
    w.construction_data.investment_opportunities=vec![offer.clone(),offer];
    let rows=kingmaker_sim::investment_assessment::assess_investment_opportunities(&w,&request()).unwrap();
    assert_eq!(rows[0]["workforce_available"],json!(0.0));
    assert_eq!(rows[0],rows[1],"assessment must not reserve workers across independent proposals");
    w.construction_data.investment_opportunities.iter_mut().for_each(|o| {o["future_assignments"].as_array_mut().unwrap().pop();});
    let rows=kingmaker_sim::investment_assessment::assess_investment_opportunities(&w,&request()).unwrap();
    assert_eq!(rows[0]["workforce_available"],json!(0.75));
    assert_eq!(rows[0],rows[1]);
}
#[test]
#[ignore = "explicit bounded release timing"]
fn investment_assessment_release_probe() {
    for n in [1000,2000,4000] {
        let w=scaled_assessment(n); let mut times=vec![];
        for _ in 0..3 {let t=std::time::Instant::now();let rows=kingmaker_sim::investment_assessment::assess_investment_opportunities(&w,&request()).unwrap();times.push(t.elapsed().as_secs_f64());assert_eq!(rows.len(),n);}
        times.sort_by(f64::total_cmp); eprintln!("assessment_release N={n} median_s={}",times[1]);
    }
}

#[test]
fn investment_assessment_cancels_inside_opportunity_loop_without_mutation() {
    use std::sync::{Arc,atomic::{AtomicUsize,Ordering}};
    let w=scaled_assessment(512); let before=serde_json::to_value(&w).unwrap();
    let count=Arc::new(AtomicUsize::new(0)); let seen=count.clone();
    let control=kingmaker_sim::run_control::RunControl::new(move |label| {
        if label=="investment_assessment_offer" && seen.fetch_add(1,Ordering::SeqCst)==1 {return Err(kingmaker_sim::operation_session::OperationError("cancel assessment".into()));} Ok(())
    });
    let result=control.run(||kingmaker_sim::investment_assessment::assess_investment_opportunities(&w,&request()));
    assert!(result.is_err(),"assessment ignored cancellation inside its loop");
    assert_eq!(count.load(Ordering::SeqCst),2);
    assert_eq!(serde_json::to_value(&w).unwrap(),before);
}

#[test]
fn six_invalid_future_worker_components_never_receive_funding() {
 for (workers,time) in [(json!(-1),json!(-1)),(json!(0.5),json!(1)),(json!(1),json!(1.5)),(json!("bad"),json!(1))] {
  let mut w=assessed_fixture();w.construction_data.investment_opportunities[0]["future_assignments"][0]["workers_count"]=workers;
  w.construction_data.investment_opportunities[0]["future_assignments"][0]["time_fraction"]=time;
  let before=serde_json::to_value(&w).unwrap();assert!(execute_automatic_investments(&mut w,&request()).is_err());assert_eq!(serde_json::to_value(&w).unwrap(),before);
 }
}
#[test]
fn six_effective_construction_override_is_validated_before_funding() {
 for field in ["currency_id","status"] {
  let mut w=fixture(1000,10000,4000);let mut c=w.production_data.catalog.as_ref().unwrap().methods[0].extra["construction"].clone();
  c[field]=json!(if field=="currency_id" {"OTHER"}else{"unresolved"});w.construction_data.investment_opportunities[0]["construction"]=c;
  let before=serde_json::to_value(&w).unwrap();assert!(execute_automatic_investments(&mut w,&request()).is_err());assert_eq!(serde_json::to_value(&w).unwrap(),before);
 }
 let mut w=fixture(1000,10000,4000);let mut c=w.production_data.catalog.as_ref().unwrap().methods[0].extra["construction"].clone();c["cost_minor"]=json!("2000");c["duration_days"]=json!(45);
 w.construction_data.investment_opportunities[0]["construction"]=c.clone();execute_automatic_investments(&mut w,&request()).unwrap();assert_eq!(project(&w)["construction"],c);assert_eq!(project(&w)["funded_minor"],json!("10000"));
}

#[test]
fn six_staff_preparation_does_not_copy_world_per_project() {
 let _lock=investment_meter::MEASURE.lock().unwrap();
 for n in [64,128,256] {
  let mut w=fixture(1000,10000,4000);w.mobility_data.capitalist_entities.clear();w.construction_data.investment_opportunities.clear();
  for i in 0..n {
   w.production_data.resource_operations.push(serde_json::from_value(json!({"id":format!("raw-{i}"),"owner_id":"o","province_id":1,"method_id":"rgo:iron","resource_id":"iron","assignments":[]})).unwrap());
   w.production_data.construction_projects.push(serde_json::from_value(json!({"id":format!("build-{i}"),"status":"funded","planned_assignments":[],"assignments":[]})).unwrap());
  }
  let m=investment_meter::Measurement::start();execute_automatic_investments(&mut w,&request()).unwrap();let (bytes,_)=m.finish();
  eprintln!("staff N={n} allocated={bytes}");assert!(bytes < n*160000,"per-project world copy: {bytes}");
  assert!(w.production_data.construction_projects.iter().all(|p|p.extra.get("staff_activated")==Some(&json!(true))));
 }
}
#[test]
fn six_stop_in_investor_offer_matching_is_atomic() {
 let mut w=fixture(1000,10000,4000);let o=w.construction_data.investment_opportunities[0].clone();
 w.construction_data.investment_opportunities=(0..512).map(|i| {let mut r=o.clone();r["id"]=json!(format!("o-{i}"));r}).collect();
 w.production_data.catalog.as_mut().unwrap().methods[0].technology=vec!["unavailable".into()];
 let before=serde_json::to_value(&w).unwrap();
 let c=kingmaker_sim::run_control::RunControl::new(|label| if label=="investment_match" {Err(kingmaker_sim::operation_session::OperationError("stop matching".into()))}else{Ok(())});
 assert!(c.run(||execute_automatic_investments(&mut w,&request())).is_err());assert_eq!(serde_json::to_value(&w).unwrap(),before);
}

#[test]
fn six_late_staff_stop_discards_already_prepared_projects() {
 use std::sync::{Arc,atomic::{AtomicUsize,Ordering}};
 let mut w=fixture(1000,10000,4000);w.mobility_data.capitalist_entities.clear();w.construction_data.investment_opportunities.clear();
 for i in 0..300 {w.production_data.construction_projects.push(serde_json::from_value(json!({"id":format!("build-{i}"),"status":"funded","planned_assignments":[],"assignments":[]})).unwrap());}
 let before=serde_json::to_value(&w).unwrap();let seen=Arc::new(AtomicUsize::new(0));let count=seen.clone();
 let c=kingmaker_sim::run_control::RunControl::new(move |label| {if label=="investment_staff" && count.fetch_add(1,Ordering::SeqCst)==1 {return Err(kingmaker_sim::operation_session::OperationError("late staff stop".into()));}Ok(())});
 assert!(c.run(||execute_automatic_investments(&mut w,&request())).is_err());assert_eq!(seen.load(Ordering::SeqCst),2);assert_eq!(serde_json::to_value(&w).unwrap(),before);
}
#[test]
#[ignore="explicit release wall-time STOP check"]
fn six_file_stop_during_matching_detected_under_one_second() {
 use std::sync::{Arc,Mutex};
 let mut w=fixture(1000,10000,4000);let offer=w.construction_data.investment_opportunities[0].clone();
 w.construction_data.investment_opportunities=(0..4096).map(|i|{let mut o=offer.clone();o["id"]=json!(format!("offer-{i}"));o}).collect();
 w.production_data.catalog.as_mut().unwrap().methods[0].technology=vec!["missing".into()];
 let before=serde_json::to_value(&w).unwrap();let path=std::env::temp_dir().join(format!("kingmaker-six-stop-{}",std::process::id()));assert!(!path.exists());
 let file=kingmaker_sim::run_control::RunControl::from_stop_file(path.clone());let started=Arc::new(Mutex::new(None));let time=started.clone();let stop=path.clone();
 let control=kingmaker_sim::run_control::RunControl::new(move |label| {if label=="investment_match" {let mut t=time.lock().unwrap();if t.is_none(){std::fs::write(&stop,b"STOP").unwrap();*t=Some(std::time::Instant::now());}}file.check(label)});
 let result=control.run(||execute_automatic_investments(&mut w,&request()));let elapsed=started.lock().unwrap().expect("matching reached").elapsed();std::fs::remove_file(path).unwrap();
 assert!(result.is_err());assert!(elapsed<std::time::Duration::from_secs(1),"STOP delay {elapsed:?}");assert_eq!(serde_json::to_value(&w).unwrap(),before);eprintln!("matching file STOP delay_ms={}",elapsed.as_secs_f64()*1000.);
}
#[test]
fn six_static_offer_index_keeps_first_choice_and_error_order() {
 for malformed_first in [false,true] {
  let mut w=fixture(1000,10000,4000);let valid=w.construction_data.investment_opportunities[0].clone();let mut bad=valid.clone();bad["id"]=json!("bad");bad["margin_minor"]=json!("not-money");
  w.mobility_data.capitalist_entities.truncate(1);
  w.construction_data.investment_opportunities=if malformed_first{vec![bad,valid]}else{vec![valid,bad]};let before=serde_json::to_value(&w).unwrap();
  let outcome=execute_automatic_investments(&mut w,&request());if malformed_first {assert!(outcome.is_err());assert_eq!(serde_json::to_value(&w).unwrap(),before);}else{outcome.unwrap();assert_eq!(w.production_data.construction_projects[0].id,ExternalId::from("private-project:offer"));}
 }
}
