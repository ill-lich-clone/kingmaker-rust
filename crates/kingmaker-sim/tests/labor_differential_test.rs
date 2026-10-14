#[path = "common/budget.rs"]
mod common;
use kingmaker_core::WorldState;
use kingmaker_sim::{labor_release::*, labor_roster::*};
use serde_json::{json, Value};
use std::{
    io::Write,
    process::{Command, Stdio},
};
fn oracle(input: Value) -> Value {
    let mut child = Command::new("node")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/tests/labor-oracle.cjs"
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
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
fn snapshot(w: &WorldState) -> Value {
    json!({"registry":{"pops":w.pops.iter().map(|p|{let mut v=json!({"pop_id":p.external_id,"pop_type_id":p.pop_type_id,"province_id":p.province_id.0,"size":p.size,"workforce":p.workforce,"employed":p.employed,"unemployed":p.unemployed});v.as_object_mut().unwrap().extend(p.extra.clone());v}).collect::<Vec<_>>(),"jobs":w.budget_data.jobs,"employment":w.budget_data.employment,"contracts":w.production_data.contracts,"resourceOperations":w.production_data.resource_operations,"factoryEnterprises":w.production_data.factories,"artisanEnterprises":w.production_data.artisans,"constructionProjects":w.production_data.construction_projects,"completedExpansions":w.production_data.expansions,"institutions":w.market_data.institutions,"popMobility":w.labor_data.pop_mobility},"accounts":[],"stocks":[],"obligations":[],"reservations":[]})
}
#[test]
fn source_roster_and_releases_match_on_real_reference_rules() {
    for case in 0..3 {
        let mut w = common::world(0);
        if case == 1 {
            w.budget_data.jobs[0]["slots_total"] = json!(0);
        }
        if case == 2 {
            w.production_data.contracts[0]
                .extra
                .insert("pop_type_id".into(), json!("clerks"));
            w.pops[0].pop_type_id = "artisans".into();
        }
        let source = oracle(json!({"action":"release","snapshot":snapshot(&w),"period":"w"}));
        let changes = release_employment(&w, "w").unwrap();
        assert_eq!(
            changes.released_fte,
            source["result"]["released_fte"].as_f64().unwrap()
        );
        changes.apply(&mut w).unwrap();
        let source_employment = source["registry"]["employment"].as_array().unwrap();
        assert_eq!(source_employment.len(), w.budget_data.employment.len());
        for (n, j) in w.budget_data.employment.iter().zip(source_employment) {
            assert_eq!(n.employment_id.json(), j["employment_id"]);
            assert_eq!(n.fte, j["fte"].as_f64().unwrap());
        }
        let source = oracle(json!({"action":"roster","snapshot":snapshot(&w)}));
        let roster = build_labor_roster(&w).unwrap();
        for row in source["by_pop"].as_array().unwrap() {
            let id = kingmaker_core::ExternalId::from_json(row[0].clone()).unwrap();
            assert_eq!(
                roster.by_pop[&id].occupied,
                row[1]["occupied"].as_f64().unwrap()
            );
            assert_eq!(roster.by_pop[&id].free, row[1]["free"].as_f64().unwrap());
        }
    }
}
