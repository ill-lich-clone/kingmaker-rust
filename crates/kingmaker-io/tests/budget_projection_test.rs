mod common;
use common::{envelope, Fixture};
use kingmaker_core::ExternalId;
use kingmaker_io::SqliteLoader;
use serde_json::json;

#[test]
fn budget_projection_preserves_typed_employment_rates_and_authored_fields() {
    let f = Fixture::new();
    f.add_record("registry/employment",0,json!(7),envelope("registry/employment",json!({"employment_id":7,"pop_id":"pop:1","contract_id":"7","job_id":9,"fte":3.5,"author":"keep"})));
    f.add_record(
        "registry/jobs",
        0,
        json!(9),
        envelope(
            "registry/jobs",
            json!({"job_id":9,"slots_total":2000,"source":"levels"}),
        ),
    );
    f.add_record("registry/adventurerIncomeRates",0,json!("rate"),envelope("registry/adventurerIncomeRates",json!({"id":"rate","pop_id":"pop:1","currency_id":"COIN","numerator":"123","denominator":"17","calibration_date":{"year":306},"source":"fixed"})));
    f.add_record(
        "registry/foundationBudgetPolicy",
        0,
        json!("#global"),
        envelope(
            "registry/foundationBudgetPolicy",
            json!({"source":"approved","reserve_weeks":2,"author_formula":"keep"}),
        ),
    );
    let w = SqliteLoader::load_from_path(&f.path, "fixture").unwrap();
    assert_eq!(
        w.budget_data.employment[0].employment_id,
        ExternalId::Number(7.into())
    );
    assert_eq!(
        w.budget_data.employment[0].contract_id,
        ExternalId::String("7".into())
    );
    assert_eq!(w.budget_data.employment[0].extra["author"], json!("keep"));
    assert_eq!(w.budget_data.jobs[0]["slots_total"], json!(2000));
    assert_eq!(
        w.budget_data.adventurer_income_rates[0].rate.denominator,
        "17"
    );
    assert_eq!(
        w.budget_data.foundation_budget_policy.unwrap()["author_formula"],
        json!("keep")
    );
}

#[test]
fn absent_null_policy_stays_absent_multiple_singletons_rejected() {
    let f = Fixture::new();
    f.add_record(
        "registry/foundationBudgetPolicy",
        0,
        json!("#global"),
        envelope("registry/foundationBudgetPolicy", json!(null)),
    );
    assert!(SqliteLoader::load_from_path(&f.path, "fixture")
        .unwrap()
        .budget_data
        .foundation_budget_policy
        .is_none());
    f.add_record(
        "registry/foundationBudgetPolicy",
        1,
        json!("#other"),
        envelope(
            "registry/foundationBudgetPolicy",
            json!({"reserve_weeks":2}),
        ),
    );
    assert!(SqliteLoader::load_from_path(&f.path, "fixture").is_err());
}
