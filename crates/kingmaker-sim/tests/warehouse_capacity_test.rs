#[path = "common/dynamic_week.rs"]
mod dynamic;
#[path = "common/market.rs"]
mod market;
#[path = "common/production.rs"]
mod production;
use kingmaker_core::*;
use kingmaker_sim::warehouse_capacity::derive_storage_capacities;
use serde_json::json;
fn fixture() -> WorldState {
    let mut w = production::world();
    w.market_data = market::market_world().market_data;
    let refs = &mut w.market_data.needs.as_mut().unwrap().profile_refs;
    refs.truncate(1);
    refs[0].pop_id = "p".into();
    refs[0].extra.insert("size".into(), json!(1));
    let f = &mut w.production_data.storage_facilities[0];
    f.extra.insert(
        "capacity_rule".into(),
        json!({"opening_space":2,"minimum_capacity":1,"buffer_weeks":12,"author_formula":"keep"}),
    );
    let mapping = f.goods["steel"].clone();
    f.goods.insert("iron".into(), mapping.clone());
    f.goods.insert("coal".into(), mapping);
    w
}
#[test]
fn installed_factory_outputs_and_recipe_inputs_determine_dynamic_capacity() {
    let mut w = fixture();
    let before = serde_json::to_value(&w).unwrap();
    let derived = derive_storage_capacities(&w).unwrap();
    assert_eq!(derived[0].capacity, Some(3002.));
    assert_eq!(
        derived[0].extra,
        w.production_data.storage_facilities[0].extra
    );
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
    w.production_data.factories[0].equipment_capacity_weekly = Some(200.);
    assert_eq!(
        derive_storage_capacities(&w).unwrap()[0].capacity,
        Some(6002.)
    );
}
#[test]
fn real_occupancy_and_held_reservations_cannot_be_shrunk_by_capacity_rules() {
    let mut w = fixture();
    w.stockpiles[0].facility_id = Some("wh".into());
    w.stockpiles[0].quantity = 10000.;
    w.production_data.storage_reservations.push(serde_json::from_value(json!({"id":"held","owner_id":"o","facility_id":"wh","goods_id":"iron","unit":"t","quantity":10,"status":"held"})).unwrap());
    assert_eq!(
        derive_storage_capacities(&w).unwrap()[0].capacity,
        Some(10010.)
    );
    w.production_data.storage_reservations[0].status = "released".into();
    assert_eq!(
        derive_storage_capacities(&w).unwrap()[0].capacity,
        Some(10000.)
    );
}
#[test]
fn no_rule_keeps_authored_capacity_and_invalid_rules_refuse_without_mutation() {
    let mut w = fixture();
    w.production_data.storage_facilities[0]
        .extra
        .remove("capacity_rule");
    assert_eq!(
        derive_storage_capacities(&w).unwrap(),
        w.production_data.storage_facilities
    );
    for bad in [
        json!({"opening_space":"bad","minimum_capacity":0,"buffer_weeks":12}),
        json!({"opening_space":0,"minimum_capacity":-1,"buffer_weeks":12}),
        json!({"opening_space":0,"minimum_capacity":0,"buffer_weeks":-1}),
    ] {
        w.production_data.storage_facilities[0]
            .extra
            .insert("capacity_rule".into(), bad);
        let before = serde_json::to_value(&w).unwrap();
        assert!(derive_storage_capacities(&w).is_err());
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
    }
}
#[test]
fn completed_infrastructure_effects_and_food_needs_update_capacity_as_in_source() {
    let mut w = fixture();
    w.production_data.completed_construction.push(serde_json::from_value(json!({"status":"completed","province_id":1,"level":2,"effects":{"factory_output_multiplier":2}})).unwrap());
    assert_eq!(
        derive_storage_capacities(&w).unwrap()[0].capacity,
        Some(12002.)
    );
    w.production_data.completed_construction.clear();
    w.pops[0].size = 100;
    w.market_data.needs.as_mut().unwrap().profile_refs[0]
        .extra
        .insert("size".into(), json!(100));
    w.market_data.consumer_facility_by_province = Some(json!({"1":"wh"}));
    let mapping = w.production_data.storage_facilities[0].goods["steel"].clone();
    w.production_data.storage_facilities[0]
        .goods
        .insert("flour".into(), mapping);
    assert_eq!(
        derive_storage_capacities(&w).unwrap()[0].capacity,
        Some(3014.)
    );
    let expected: Vec<kingmaker_core::storage::StorageFacility> =
        serde_json::from_value(dynamic::oracle("warehouse-capacity", &w)).unwrap();
    assert_eq!(derive_storage_capacities(&w).unwrap(), expected);
}
#[test]
fn rgo_expansion_technology_and_state_raw_warehouse_use_installed_capacity() {
    let mut w = fixture();
    w.production_data.factories.clear();
    w.production_data.resource_operations.push(serde_json::from_value(json!({"id":"raw","owner_id":"o","province_id":1,"method_id":"rgo:grain","resource_id":"grain","facility_id":"wh","initial_multiplier":1,"assignments":[],"technology":[{"id":"better-tools","status":"source","source":"author","places_delta":500}]})).unwrap());
    w.production_data.expansions.push(serde_json::from_value(json!({"id":"pasture","rgo_id":"raw","status":"completed","kind":"pasture","completed_levels":1,"places_per_level":2000})).unwrap());
    let mut m = w.production_data.catalog.as_ref().unwrap().methods[0].clone();
    m.method_id = "rgo:grain".into();
    m.kind = "rgo".into();
    m.output_goods_id = "grain".into();
    m.weekly_productivity = Some(0.5);
    m.inputs.clear();
    w.production_data.catalog.as_mut().unwrap().methods.push(m);
    w.market_data.territory[0]
        .extra
        .insert("owner_state_id".into(), json!("s"));
    let f = &mut w.production_data.storage_facilities[0];
    let mapping = f.goods["steel"].clone();
    f.goods.insert("grain".into(), mapping);
    let mut public = f.clone();
    public.id = "state-raw:s:1".into();
    public.owner_id = "treasury:s".into();
    public.extra.insert(
        "capacity_rule".into(),
        json!({"opening_space":0,"minimum_capacity":0,"buffer_weeks":12}),
    );
    w.production_data.storage_facilities.push(public);
    let derived = derive_storage_capacities(&w).unwrap();
    assert_eq!(derived[0].capacity, Some(255002.));
    assert_eq!(derived[1].capacity, Some(255000.));
    let expected: Vec<kingmaker_core::storage::StorageFacility> =
        serde_json::from_value(dynamic::oracle("warehouse-capacity", &w)).unwrap();
    assert_eq!(derived, expected);
}
#[test]
fn artisan_pending_method_drives_installed_input_and_output_storage() {
    let mut w = fixture();
    let f = w.production_data.factories.remove(0);
    let mut a: kingmaker_core::production::ArtisanEnterprise =
        serde_json::from_value(serde_json::to_value(f).unwrap()).unwrap();
    a.people_slots = Some(10.);
    a.base.method_id = "artisan:steel".into();
    let m = &mut w.production_data.catalog.as_mut().unwrap().methods[0];
    m.kind = "artisan".into();
    m.method_id = "artisan:steel".into();
    w.production_data.artisans.push(a);
    assert_eq!(
        derive_storage_capacities(&w).unwrap()[0].capacity,
        Some(302.)
    );
    let mut pending = w.production_data.catalog.as_ref().unwrap().methods[0].clone();
    pending.method_id = "artisan:efficient".into();
    pending.weekly_productivity = Some(2.5);
    pending.inputs.truncate(1);
    pending.inputs[0].amount = 0.1;
    w.production_data
        .catalog
        .as_mut()
        .unwrap()
        .methods
        .push(pending);
    w.production_data.artisans[0].pending_retooling =
        Some(json!({"method_id":"artisan:efficient"}));
    assert_eq!(
        derive_storage_capacities(&w).unwrap()[0].capacity,
        Some(332.)
    );
    let expected: Vec<kingmaker_core::storage::StorageFacility> =
        serde_json::from_value(dynamic::oracle("warehouse-capacity", &w)).unwrap();
    assert_eq!(derive_storage_capacities(&w).unwrap(), expected);
    w.production_data.catalog.as_mut().unwrap().methods[1].status = "draft".into();
    assert_eq!(derive_storage_capacities(&w).unwrap()[0].capacity, Some(2.));
}
