use kingmaker_sim::labor_capacity::derive_institution_jobs;
use serde_json::{json, Value};
use std::{
    io::Write,
    process::{Command, Stdio},
};
#[path = "common/production.rs"]
mod production_fixture;

#[test]
fn rgo_completed_expansion_updates_existing_job_capacity_without_resource_substitution() {
    let mut w = production_fixture::world();
    w.production_data.factories.clear();
    w.production_data.resource_operations.push(serde_json::from_value(json!({"id":"mine","owner_id":"o","province_id":1,"method_id":"rgo:iron","resource_id":"iron","resource_status":"approved","land_access":true,"initial_multiplier":3,"assignments":[]})).unwrap());
    w.budget_data.jobs =
        vec![json!({"job_id":"mine-job","building_id":"mine","goods_id":"iron","slots_total":1})];
    w.production_data.expansions.push(serde_json::from_value(json!({"id":"exp","rgo_id":"mine","status":"completed","kind":"mine","completed_levels":2,"places_per_level":2000})).unwrap());
    let jobs = kingmaker_sim::labor_capacity::derive_labor_jobs(&w).unwrap();
    assert_eq!(jobs[0]["slots_total"].as_f64(), Some(124000.));
    assert_eq!(jobs[0]["goods_id"], json!("iron"));
    assert_eq!(
        w.production_data.resource_operations[0]
            .resource_id
            .as_str(),
        Some("iron")
    );
}

#[test]
fn production_and_institution_capacity_use_current_author_rows_together() {
    let mut w = production_fixture::world();
    w.budget_data.jobs =
        vec![json!({"job_id":"factory-job","building_id":"f","slots_total":1,"author":"keep"})];
    w.production_data.factories[0].people_slots = Some(80000.);
    let rows = kingmaker_sim::labor_capacity::derive_labor_jobs(&w).unwrap();
    assert_eq!(rows[0]["slots_total"].as_f64(), Some(80000.));
    assert_eq!(rows[0]["capacity_pool_total"].as_f64(), Some(80000.));
    assert_eq!(rows[0]["author"], json!("keep"));
    assert_eq!(w.budget_data.jobs[0]["slots_total"], json!(1));
}

#[test]
fn current_js_capacity_rows_match_including_ties_authored_extras_and_missing_status() {
    for override_roles in [false, true] {
        let institutions = vec![
            json!({"building_id":"school","building_type_id":"school","province_id":1,"owner_entity_id":"s","level":1}),
            json!({"building_id":"unknown","building_type_id":"unknown","level":5}),
        ];
        let jobs = vec![
            json!({"job_id":"Я","building_id":"school","role_id":"teacher","slots_total":1,"extra_author":true}),
            json!({"job_id":"ё","building_id":"school","role_id":"teacher","capacity_weight":"1"}),
            json!({"job_id":"a","building_id":"school","role_id":"teacher","capacity_weight":1}),
        ];
        let rules = json!({"school":{"places_per_level":2000,"roles":[{"role_id":"teacher","pop_type_id":"clergy","weight":1},{"role_id":"office","pop_type_id":"bureaucrats","weight":3}]}});
        let mut institutions = institutions;
        if override_roles {
            institutions[0]["staffing_roles"] = json!([{"role_id":"teacher","pop_type_id":"clergy","weight":7},{"role_id":"office","pop_type_id":"bureaucrats","weight":0}]);
        }
        let native = derive_institution_jobs(&institutions, &jobs, &rules).unwrap();
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
        child.stdin.take().unwrap().write_all(json!({"action":"institution_capacity","registry":{"institutions":institutions,"jobs":jobs,"institutionRules":rules}}).to_string().as_bytes()).unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let oracle: Vec<Value> = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(native.len(), oracle.len());
        for (mut n, mut j) in native.into_iter().zip(oracle.iter().cloned()) {
            for row in [&mut n, &mut j] {
                if let Some(w) = row.get_mut("capacity_weight") {
                    *w = json!(w.as_f64().unwrap());
                }
            }
            assert_eq!(n, j);
        }
    }
}

#[test]
fn invalid_job_weight_among_positive_weights_cannot_silently_gain_capacity() {
    let institutions = [json!({"building_id":"school","building_type_id":"school","level":1})];
    let jobs = [
        json!({"job_id":"a","building_id":"school","role_id":"teacher","capacity_weight":1}),
        json!({"job_id":"b","building_id":"school","role_id":"teacher","capacity_weight":"not-a-number"}),
    ];
    let rules = json!({"school":{"places_per_level":2000,"roles":[{"role_id":"teacher","pop_type_id":"clergy","weight":1}]}});
    assert!(derive_institution_jobs(&institutions, &jobs, &rules).is_err());
}
fn derive(level: u64) -> Vec<Value> {
    derive_institution_jobs(&[json!({"building_id":"school","building_type_id":"school","province_id":1,"state_id":"s","level":level,"operational_status":"active"})],&[],&json!({"school":{"places_per_level":2000,"roles":[{"role_id":"teacher","pop_type_id":"clergy","weight":1},{"role_id":"official","pop_type_id":"bureaucrats","weight":3}]}})).unwrap()
}
#[test]
fn levels_derive_capacity_with_exact_role_totals_and_no_pop_based_reseed() {
    for level in [0, 1, 2, 100] {
        let jobs = derive(level);
        assert_eq!(jobs[0]["slots_total"], json!(level * 500));
        assert_eq!(jobs[1]["slots_total"], json!(level * 1500));
        assert_eq!(
            jobs.iter()
                .map(|j| j["slots_total"].as_u64().unwrap())
                .sum::<u64>(),
            level * 2000
        );
        assert_eq!(jobs[0]["capacity_pool_total"], json!(level * 2000));
    }
}
#[test]
fn existing_rows_weights_extras_and_typed_building_ids_are_preserved() {
    let institutions = vec![
        json!({"building_id":7,"building_type_id":"school","province_id":1,"level":1}),
        json!({"building_id":"7","building_type_id":"school","province_id":2,"level":2}),
    ];
    let jobs = vec![
        json!({"job_id":"a","building_id":7,"role_id":"teacher","capacity_weight":1,"author":"keep"}),
        json!({"job_id":"b","building_id":7,"role_id":"teacher","capacity_weight":3}),
        json!({"job_id":"other","building_id":"other","slots_total":17}),
    ];
    let out=derive_institution_jobs(&institutions,&jobs,&json!({"school":{"places_per_level":2000,"roles":[{"role_id":"teacher","pop_type_id":"clergy","weight":1}]}})).unwrap();
    assert_eq!(out[0]["slots_total"], json!(500));
    assert_eq!(out[1]["slots_total"], json!(1500));
    assert_eq!(out[0]["author"], json!("keep"));
    assert_eq!(out[2], jobs[2]);
    assert_eq!(out[3]["building_id"], json!("7"));
    assert_eq!(out[3]["slots_total"], json!(4000));
}
#[test]
fn malformed_levels_rules_and_roles_fail_instead_of_defaulting() {
    for (level, roles, places) in [
        (
            json!(-1),
            json!([{"role_id":"a","pop_type_id":"clergy","weight":1}]),
            json!(2000),
        ),
        (
            json!(1.5),
            json!([{"role_id":"a","pop_type_id":"clergy","weight":1}]),
            json!(2000),
        ),
        (
            json!(1),
            json!([{"role_id":"a","pop_type_id":"clergy","weight":1}]),
            json!(5000),
        ),
        (
            json!(1),
            json!([{"role_id":"a","pop_type_id":"clergy","weight":-1}]),
            json!(2000),
        ),
        (
            json!(1),
            json!([{"role_id":"a","pop_type_id":"clergy","weight":1},{"role_id":"a","pop_type_id":"clergy","weight":1}]),
            json!(2000),
        ),
    ] {
        assert!(derive_institution_jobs(
            &[json!({"building_id":"x","building_type_id":"school","level":level})],
            &[],
            &json!({"school":{"places_per_level":places,"roles":roles}})
        )
        .is_err());
    }
}
