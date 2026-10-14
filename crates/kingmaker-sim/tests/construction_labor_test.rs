#[path = "common/production.rs"]
mod common;
#[path = "common/income.rs"]
mod income_common;
use kingmaker_core::{construction::ConstructionRequest, *};
use kingmaker_sim::construction_labor::plan_construction_labor;
use serde_json::json;
fn request() -> ConstructionRequest {
    ConstructionRequest {
        expected_revision: 20,
        period: "w".into(),
        policies: Default::default(),
    }
}
fn fixture(cash: i64) -> WorldState {
    let mut w = common::world();
    w.production_data.factories.clear();
    w.production_data.contracts.clear();
    w.pops[0].pop_type_id = "farmers".into();
    w.pops[0].pop_type = "farmers".into();
    w.pops[0].size = 2;
    w.pops[0].workforce = 2.;
    w.pops[0].employed = 2.;
    w.accounts
        .push(common::account(3, "project:p", "construction_escrow", cash));
    w.rebuild_account_index().unwrap();
    w.production_data.contracts.push(serde_json::from_value(json!({"contract_id":"build-c","employer_entity_id":"project:p","currency_id":"COIN","payment_purpose":"construction_escrow","weekly_wage_rate":{"numerator":"10","denominator":"1"}})).unwrap());
    w.production_data.construction_projects.push(serde_json::from_value(json!({"id":"p","kind":"factory","owner_id":"project:p","province_id":1,"region_id":"r","status":"funded","construction":{"status":"ready","source":"fixture","currency_id":"COIN","duration_days":30,"labor_required_fte_days":7,"materials":[{"goods_id":"iron","amount":1}]},"assignments":[{"employment_id":"builder-e","contract_id":"build-c","pop_id":"p","workers_count":2,"time_fraction":1}]})).unwrap());
    w.stockpiles[0].owner_id = "project:p".into();
    w.stockpiles[0].quantity = 1.;
    w
}
#[test]
fn paid_builder_plan_limits_remaining_labor_and_partial_cash_and_matches_source() {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    for cash in [100, 5, 0] {
        let w = fixture(cash);
        let before = serde_json::to_value(&w).unwrap();
        let actual = plan_construction_labor(&w, &request()).unwrap();
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
        let mut snapshot = income_common::snapshot(&w);
        snapshot["registry"]["constructionProjects"] =
            json!(w.production_data.construction_projects);
        p.stdin.take().unwrap().write_all(json!({"action":"labor","snapshot":snapshot,"period":"w","policies":w.production_data.policies}).to_string().as_bytes()).unwrap();
        let out = p.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let expected: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        let expected_requests: Vec<kingmaker_sim::production_feasibility::PayrollRequest> =
            serde_json::from_value(expected["payrollRequests"].clone()).unwrap();
        assert_eq!(json!(actual.requests), json!(expected_requests));
        assert_eq!(json!(actual.blocked), expected["blocked"]);
        assert_eq!(actual.requests[0].gross_minor, Money(cash.min(10)));
        assert_eq!(actual.requests[0].planned_fte, cash.min(10) as f64 / 10.);
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
    }
}
#[test]
fn missing_materials_wrong_pop_contract_or_finished_labor_have_no_payroll() {
    for case in 0..4 {
        let mut w = fixture(100);
        match case {
            0 => w.stockpiles[0].quantity = 0.,
            1 => w.pops[0].pop_type_id = "craftsmen".into(),
            2 => w.production_data.contracts[0].employer_entity_id = Some("other".into()),
            _ => {
                w.production_data.construction_projects[0]
                    .extra
                    .insert("labor_fte_days".into(), json!(7));
            }
        }
        let plan = plan_construction_labor(&w, &request()).unwrap();
        assert!(plan.requests.is_empty());
        if case < 3 {
            assert!(!plan.blocked.is_empty());
        }
    }
}

#[test]
fn building_limit_blocks_construction_payroll_before_any_project_spending() {
    let mut w = fixture(100);
    w.market_data.institutions=(0..12).map(|i|json!({"id":format!("school:{i}"),"province_id":1,"operational_status":"operating"})).collect();
    let plan = plan_construction_labor(&w, &request()).unwrap();
    assert!(plan.requests.is_empty());
    assert_eq!(plan.blocked[0]["reason"], json!("province_building_limit"));
}
