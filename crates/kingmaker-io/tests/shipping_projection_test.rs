mod common;
use common::{envelope, Fixture};
use kingmaker_io::SqliteLoader;
use serde_json::json;
#[test]
fn authored_recovery_terms_preserve_id_types_rights_unknown_fields_and_changed_values() {
    let fixture = Fixture::new();
    for (ordinal, id) in [(0, json!(7)), (1, json!("7"))] {
        fixture.add_record("registry/shipmentRecoveryPlans",ordinal,id.clone(),envelope("registry/shipmentRecoveryPlans",json!({"id":id,"shipment_id":"cargo","status":"approved","source":"author","approval_ref":"author","action":"return","extra_freight_minor":"17","accepted_buyer_id":7,"accepted_seller_id":"7","unknown_authored_formula":{"keep":true}})));
    }
    let mut world = SqliteLoader::load_from_path(&fixture.path, "fixture").unwrap();
    assert_eq!(world.transport_data.shipment_recovery_plans.len(), 2);
    assert_eq!(
        world.transport_data.shipment_recovery_plans[0]["id"],
        json!(7)
    );
    assert_eq!(
        world.transport_data.shipment_recovery_plans[1]["id"],
        json!("7")
    );
    assert_eq!(
        world.transport_data.shipment_recovery_plans[0]["accepted_seller_id"],
        json!("7")
    );
    world.transport_data.shipment_recovery_plans[0]["extra_freight_minor"] = json!("19");
    let restored: kingmaker_core::WorldState =
        serde_json::from_value(serde_json::to_value(&world).unwrap()).unwrap();
    assert_eq!(restored.transport_data, world.transport_data);
    assert_eq!(
        restored.transport_data.shipment_recovery_plans[0]["unknown_authored_formula"],
        json!({"keep":true})
    );
}
