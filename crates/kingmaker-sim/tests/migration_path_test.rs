use kingmaker_core::{market::MarketTerritory, production::ProductionPolicies, ProvinceId};
use kingmaker_sim::migration_path::WalkingPaths;
use serde_json::json;
fn map() -> Vec<MarketTerritory> {
    serde_json::from_value(json!([{"province_id":1,"is_floating":false,"landmass_id":"land","owner_state_id":"A","neighbors":[{"neighbor_id":2,"contact_type":"land"}]},{"province_id":2,"is_floating":false,"landmass_id":"land","owner_state_id":"A","neighbor_ids":"1,3;4"},{"province_id":3,"is_floating":false,"landmass_id":"land","owner_state_id":"B","neighbors":[2]},{"province_id":4,"is_floating":false,"landmass_id":"other","owner_state_id":"A","neighbors":[2]}])).unwrap()
}
#[test]
fn land_neighbors_and_cached_multiple_destinations_preserve_source_order() {
    let rows = map();
    let p = ProductionPolicies::default();
    let mut paths = WalkingPaths::new(&rows, &p).unwrap();
    assert_eq!(
        paths.path(ProvinceId(1), ProvinceId(2)).unwrap(),
        Some(vec![ProvinceId(1), ProvinceId(2)])
    );
    assert_eq!(paths.path(ProvinceId(1), ProvinceId(4)).unwrap(), None);
    assert_eq!(paths.path(ProvinceId(1), ProvinceId(3)).unwrap(), None);
}
#[test]
fn foreign_permission_is_directed_and_never_implies_reverse_border() {
    let rows = map();
    let mut p = ProductionPolicies::default();
    p.extra.insert(
        "migration_borders".into(),
        json!([{"from_state_id":"A","to_state_id":"B","allowed":true}]),
    );
    let mut paths = WalkingPaths::new(&rows, &p).unwrap();
    assert_eq!(
        paths.path(ProvinceId(1), ProvinceId(3)).unwrap(),
        Some(vec![ProvinceId(1), ProvinceId(2), ProvinceId(3)])
    );
    assert_eq!(paths.path(ProvinceId(3), ProvinceId(1)).unwrap(), None);
}
#[test]
fn sea_contacts_floating_provinces_and_unknown_landmass_do_not_allow_walk() {
    for case in 0..3 {
        let mut rows = map();
        match case {
            0 => {
                rows[0].extra.insert(
                    "neighbors".into(),
                    json!([{"neighbor_id":2,"contact_type":"sea"}]),
                );
            }
            1 => {
                rows[1].extra.insert("is_floating".into(), json!(true));
            }
            _ => {
                rows[1].extra.remove("landmass_id");
            }
        }
        let p = ProductionPolicies::default();
        let mut paths = WalkingPaths::new(&rows, &p).unwrap();
        assert_eq!(paths.path(ProvinceId(1), ProvinceId(2)).unwrap(), None);
    }
}
