mod common;
use common::{envelope, Fixture};
use kingmaker_io::SqliteLoader;
use serde_json::json;

#[test]
fn construction_author_data_preserves_exact_actor_ids_terms_catalogs_and_unknown_fields() {
    let f = Fixture::new();
    for (table, id, row) in [
        (
            "investmentOpportunities",
            json!(7),
            json!({"id":7,"owner_id":9,"method_id":"factory:tools","province_id":1,"terms":{"currency_id":"COIN","budget_minor":"9007199254740993","version":3,"unknown":"keep"},"source":{"row":8},"factory_configuration":{"facility_id":"future","people_slots":40000}}),
        ),
        (
            "constructionCatalog",
            json!("school"),
            json!({"id":"school","cost_minor":"123","duration_days":90,"materials":[{"goods_id":"wood","amount":1.25}],"effects":{"capacity":2000},"unknown":"keep"}),
        ),
        (
            "buildingSiteGrants",
            json!("site"),
            json!({"id":"site","owner_id":"9","province_id":1,"kinds":["factory"],"status":"approved","approval_ref":"author"}),
        ),
    ] {
        let name = format!("registry/{table}");
        f.add_record(&name, 0, id, envelope(&name, row));
    }
    f.add_record("registry/constructionCatalogByCurrency",0,json!("#global"),envelope("registry/constructionCatalogByCurrency",json!({"COIN":[{"id":"school","cost_minor":"456","cash_cost_basis":"includes_wages","source":"author"}]})));
    let mut w = SqliteLoader::load_from_path(&f.path, "fixture").unwrap();
    assert_eq!(
        w.construction_data.investment_opportunities[0]["owner_id"],
        json!(9)
    );
    assert_eq!(
        w.construction_data.building_site_grants[0]["owner_id"],
        json!("9")
    );
    assert_eq!(
        w.construction_data.investment_opportunities[0]["terms"]["budget_minor"],
        json!("9007199254740993")
    );
    assert_eq!(
        w.construction_data.construction_catalog[0]["materials"][0]["amount"],
        json!(1.25)
    );
    assert_eq!(
        w.construction_data
            .construction_catalog_by_currency
            .as_ref()
            .unwrap()["COIN"][0]["cost_minor"],
        json!("456")
    );
    w.construction_data.investment_opportunities[0]["terms"]["unknown"] = json!("native update");
    let loaded: kingmaker_core::WorldState =
        serde_json::from_value(serde_json::to_value(&w).unwrap()).unwrap();
    assert_eq!(loaded.construction_data, w.construction_data);
}

#[test]
fn changed_author_terms_are_read_directly_without_runtime_substitution() {
    let f = Fixture::new();
    f.add_record(
        "registry/investmentOpportunities",
        0,
        json!("offer"),
        envelope(
            "registry/investmentOpportunities",
            json!({"id":"offer","terms":{"budget_minor":"200","version":1},"unknown":"keep"}),
        ),
    );
    let before = SqliteLoader::load_from_path(&f.path, "fixture").unwrap();
    f.sql("UPDATE km_entity SET payload=json_set(payload,'$.value.terms.budget_minor','300','$.value.terms.version',2) WHERE table_name='registry/investmentOpportunities'");
    let after = SqliteLoader::load_from_path(&f.path, "fixture").unwrap();
    assert_eq!(
        before.construction_data.investment_opportunities[0]["terms"]["budget_minor"],
        json!("200")
    );
    assert_eq!(
        after.construction_data.investment_opportunities[0]["terms"]["budget_minor"],
        json!("300")
    );
    assert_eq!(
        after.construction_data.investment_opportunities[0]["terms"]["version"],
        json!(2)
    );
    assert_eq!(
        after.construction_data.investment_opportunities[0]["unknown"],
        json!("keep")
    );
    assert!(after.construction_data.construction_catalog.is_empty());
    assert!(after
        .construction_data
        .construction_catalog_by_currency
        .is_none());
    assert_eq!(before.accounts, after.accounts);
    assert_eq!(before.stockpiles, after.stockpiles);
    assert_eq!(before.pops, after.pops);
}
