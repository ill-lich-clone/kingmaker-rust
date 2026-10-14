//! Read-only initial-state diagnosis. It never seeds, normalizes or repairs the world.
use kingmaker_core::world_read::WorldRead;
use kingmaker_core::{production::Producer, *};
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
#[derive(Debug, Clone, Serialize)]
pub struct AuditIssue {
    pub code: String,
    pub id: Option<ExternalId>,
    pub province_id: Option<ProvinceId>,
    pub detail: String,
}
#[derive(Debug, Clone, Default, Serialize)]
pub struct InitialAudit {
    pub issues: Vec<AuditIssue>,
    pub not_measured: Vec<String>,
    pub observations: Vec<Value>,
}
impl InitialAudit {
    pub fn can_run(&self) -> bool {
        self.issues.is_empty()
    }
    fn issue(
        &mut self,
        code: &str,
        id: Option<&ExternalId>,
        province_id: Option<ProvinceId>,
        detail: impl ToString,
    ) {
        self.issues.push(AuditIssue {
            code: code.into(),
            id: id.cloned(),
            province_id,
            detail: detail.to_string(),
        });
    }
}
pub fn audit_initial_world(w: &dyn WorldRead) -> InitialAudit {
    let mut out = InitialAudit::default();
    for error in w.validate_links().errors {
        out.issue("native_link", None, None, error);
    }
    if let Err(error) = crate::production_validation::validate_production_catalog(w) {
        out.issue("production_catalog", None, None, error);
    }
    if let Err(error) = crate::labor_roster::build_labor_roster(w) {
        out.issue("labor_roster", None, None, error);
    }
    let provinces = w.provinces().values()
        .map(|p| (p.id, p))
        .collect::<HashMap<_, _>>();
    let pops = w.pops().values()
        .map(|p| (&p.external_id, p))
        .collect::<HashMap<_, _>>();
    let methods = w.production_catalog()
        .as_ref()
        .map(|c| {
            c.methods
                .iter()
                .map(|m| (&m.method_id, m))
                .collect::<HashMap<_, _>>()
        })
        .unwrap_or_default();
    let goods = w.production_catalog()
        .as_ref()
        .map(|c| {
            c.goods
                .iter()
                .map(|g| (&g.goods_id, g))
                .collect::<HashMap<_, _>>()
        })
        .unwrap_or_default();
    let mut owners = w.accounts().values()
        .map(|a| a.owner_id.clone())
        .chain(w.pops().values().map(|p| p.external_id.clone()))
        .collect::<HashSet<_>>();
    for p in w.provinces().values() {
        if let Some(state) = &p.owner_state_id {
            owners.insert(format!("treasury:{}", crate::needs::js_string(&state.json())).into());
        }
    }
    for row in w.capitalist_entities().values() {
        for key in ["id", "owner_id"] {
            if let Some(id) = row
                .get(key)
                .and_then(|v| ExternalId::from_json(v.clone()).ok())
            {
                owners.insert(id);
            }
        }
        if let Some(id) = row
            .get("pop_id")
            .filter(|v| !v.is_null())
            .and_then(|v| ExternalId::from_json(v.clone()).ok())
        {
            if !pops.contains_key(&id) {
                out.issue(
                    "capitalist_pop_reference",
                    Some(&id),
                    None,
                    "Capitalist entity refers to absent POP",
                );
            }
        }
    }
    for row in w.production_beneficiaries().values()
        .chain(&w.factory_beneficiaries())
    {
        let id = row
            .get("owner_id")
            .and_then(|v| ExternalId::from_json(v.clone()).ok());
        if let Some(id) = &id {
            owners.insert(id.clone());
        }
        if let Some(shares) = row.get("beneficiaries").and_then(Value::as_array) {
            if let Err(error) = crate::profit_rights::validate_shares(shares) {
                out.issue("beneficial_shares", id.as_ref(), None, error);
            }
            for share in shares {
                if let Some(pop) = share
                    .get("pop_id")
                    .filter(|v| !v.is_null())
                    .and_then(|v| ExternalId::from_json(v.clone()).ok())
                {
                    if !pops.contains_key(&pop) {
                        out.issue(
                            "beneficiary_pop_reference",
                            Some(&pop),
                            None,
                            "Beneficial claim refers to absent POP",
                        );
                    }
                }
            }
        } else {
            out.issue(
                "beneficial_shares_missing",
                id.as_ref(),
                None,
                "Beneficial ownership row lacks its actual shares",
            );
        }
    }
    let mut addresses = HashSet::new();
    for a in w.accounts().values() {
        if !addresses.insert(a.address()) {
            out.issue(
                "duplicate_cash_address",
                Some(&a.external_id),
                None,
                format!("{:?}", a.address()),
            );
        }
        if a.balance.0 < 0 {
            out.issue(
                "negative_cash",
                Some(&a.external_id),
                None,
                "Account cash cannot be negative; permitted debt is recorded separately",
            );
        }
    }
    let mut by_province = HashMap::<ProvinceId, Vec<_>>::new();
    let populated = w.pops().values()
        .filter(|p| p.size > 0)
        .map(|p| p.province_id)
        .collect::<HashSet<_>>();
    let mut kinds = HashMap::<ProvinceId, String>::new();
    let mut conflicting = HashSet::new();
    for (province, kind) in w.territory().values()
        .filter_map(|p| {
            p.extra
                .get("kind")
                .and_then(Value::as_str)
                .map(|k| (p.province_id, k))
        })
        .chain(
            w.navigation_geography().values()
                .filter_map(|p| {
                    Some((
                        ProvinceId(u32::try_from(p.get("province_id")?.as_u64()?).ok()?),
                        p.get("kind")?.as_str()?,
                    ))
                }),
        )
    {
        if let Some(previous) = kinds.insert(province, kind.to_owned()) {
            if previous != kind && conflicting.insert(province) {
                out.issue(
                    "province_kind_conflict",
                    None,
                    Some(province),
                    format!("Author geography types disagree: {previous} / {kind}"),
                );
            }
        }
    }
    for r in w.resource_operations().values() {
        by_province.entry(r.base.province_id).or_default().push(r);
    }
    for p in w.provinces().values() {
        let rows = by_province.get(&p.id).map(Vec::as_slice).unwrap_or(&[]);
        if matches!(kinds.get(&p.id).map(String::as_str), Some("sea" | "lake"))
            && !conflicting.contains(&p.id)
            && rows.is_empty()
            && p.goods_id.is_none()
            && !populated.contains(&p.id)
        {
            continue;
        }
        if rows.len() != 1 {
            out.issue(
                "raw_province_count",
                None,
                Some(p.id),
                format!("Expected one DP, found {}", rows.len()),
            );
        }
        for r in rows {
            let raw = r.resource_id.clone();
            if p.goods_id.as_ref().map(Goods::external_id) != Some(raw.clone()) {
                out.issue(
                    "raw_author_resource_mismatch",
                    Some(&r.base.id),
                    Some(p.id),
                    format!("Province {:?}; DP {:?}", p.goods_id, raw),
                );
            }
            let method = methods.get(&r.base.method_id);
            if method
                .is_none_or(|m| m.kind != "rgo" || m.output_goods_id != raw || !m.inputs.is_empty())
                || goods.get(&raw).is_none_or(|g| g.kind != "raw_resource")
            {
                out.issue(
                    "raw_recipe",
                    Some(&r.base.id),
                    Some(p.id),
                    "DP must output its one basic resource with labor only",
                );
            }
        }
    }
    let producers = w.resource_operations().values()
        .map(|e| {
            (
                &e.base,
                crate::producer_reserves::is_unclaimed_empty_rgo(w, e),
            )
        })
        .chain(w.factories().values().map(|e| (&e.base, false)))
        .chain(w.artisans().values().map(|e| (&e.base, false)));
    let facilities = w.storage_facilities().values()
        .map(|f| (&f.id, f))
        .collect::<HashMap<_, _>>();
    let mut entity_ids = HashSet::new();
    for (e, dormant) in producers {
        if !entity_ids.insert(&e.id) {
            out.issue(
                "duplicate_producer",
                Some(&e.id),
                Some(e.province_id),
                "Producer identity reused",
            );
        }
        if !provinces.contains_key(&e.province_id) {
            out.issue(
                "producer_province_reference",
                Some(&e.id),
                Some(e.province_id),
                "Producer province absent",
            );
        }
        if !dormant && e.owner_id.as_ref().is_none_or(|id| !owners.contains(id)) {
            out.issue(
                "producer_owner_unresolved",
                Some(&e.id),
                Some(e.province_id),
                format!(
                    "Owner {:?} has no known actor/account/beneficial entity",
                    e.owner_id
                ),
            );
        }
        if !methods.contains_key(&e.method_id) {
            out.issue(
                "producer_method_reference",
                Some(&e.id),
                Some(e.province_id),
                format!("Unknown method {:?}", e.method_id),
            );
        }
        if let Some(id) = &e.facility_id {
            if !facilities.contains_key(id) {
                out.issue(
                    "producer_warehouse_reference",
                    Some(&e.id),
                    Some(e.province_id),
                    format!("Unknown warehouse {id:?}"),
                );
            }
        }
        check_assignments(&mut out, e, &pops);
    }
    let territory = w.territory().values()
        .map(|p| (p.province_id, p.region_id.as_ref()))
        .collect::<HashMap<_, _>>();
    let mut regions = HashMap::<ExternalId, Vec<ExternalId>>::new();
    for f in w.factories().values() {
        if f.status.as_deref() == Some("demolished") {
            continue;
        }
        let region = f
            .region_id
            .as_ref()
            .or_else(|| territory.get(&f.base.province_id).copied().flatten());
        if let Some(region) = region {
            regions
                .entry(region.clone())
                .or_default()
                .push(f.base.id.clone());
        } else {
            out.issue(
                "factory_region_reference",
                Some(&f.base.id),
                Some(f.base.province_id),
                "Factory region unresolved",
            );
        }
        for (name, value) in [
            ("people_slots", f.people_slots),
            ("equipment_capacity_weekly", f.equipment_capacity_weekly),
            ("staff_count", f.staff_count),
        ] {
            if value.is_some_and(|n| !n.is_finite() || n < 0.) {
                out.issue(
                    "invalid_factory_capacity",
                    Some(&f.base.id),
                    Some(f.base.province_id),
                    name,
                );
            }
        }
    }
    for (region, ids) in regions {
        if ids.len() > 8 {
            out.issue(
                "factory_region_limit",
                Some(&region),
                None,
                format!("{} factories: {ids:?}", ids.len()),
            );
        }
    }
    let mut stock_ids = HashSet::new();
    for (e, capacity) in w.factories().values()
        .map(|f| (&f.base, f.people_slots))
        .chain(
            w.artisans().values()
                .map(|a| (&a.base, a.people_slots)),
        )
    {
        if let Some(capacity) = capacity {
            let occupied = e
                .assignments
                .iter()
                .map(|a| a.workers_count * a.time_fraction)
                .sum::<f64>();
            if !capacity.is_finite() || capacity < 0. || occupied > capacity + 1e-7 {
                out.issue(
                    "producer_capacity",
                    Some(&e.id),
                    Some(e.province_id),
                    format!("capacity {capacity}; assigned FTE {occupied}"),
                );
            }
        } else {
            out.not_measured.push(format!(
                "producer {:?}: нет измерения declared people_slots",
                e.id
            ));
        }
    }
    let mut occupied = HashMap::<ExternalId, f64>::new();
    for s in w.stockpiles().values() {
        if !stock_ids.insert(&s.external_id) {
            out.issue(
                "duplicate_stock_identity",
                Some(&s.external_id),
                Some(s.province_id),
                "External stock identity reused",
            );
        }
        if !s.quantity.is_finite() || s.quantity < 0. {
            out.issue(
                "invalid_stock_quantity",
                Some(&s.external_id),
                Some(s.province_id),
                s.quantity,
            );
        }
        if let Some(id) = &s.facility_id {
            if let Some(f) = facilities.get(id) {
                if f.province_id != s.province_id {
                    out.issue(
                        "stock_warehouse_province",
                        Some(&s.external_id),
                        Some(s.province_id),
                        "Stock and warehouse locations differ",
                    );
                }
                if let Some(mapping) = f
                    .goods
                    .get(&crate::needs::js_string(&s.goods.external_id().json()))
                {
                    if s.unit.as_ref() != Some(&mapping.unit)
                        || !mapping.space_per_unit.is_finite()
                        || mapping.space_per_unit <= 0.
                    {
                        out.issue(
                            "stock_warehouse_unit",
                            Some(&s.external_id),
                            Some(s.province_id),
                            "Stock unit or space mapping invalid",
                        );
                    } else {
                        *occupied.entry(id.clone()).or_default() +=
                            s.quantity * mapping.space_per_unit;
                    }
                } else {
                    out.issue(
                        "stock_warehouse_goods",
                        Some(&s.external_id),
                        Some(s.province_id),
                        "Warehouse cannot store this good",
                    );
                }
            } else {
                out.issue(
                    "stock_warehouse_reference",
                    Some(&s.external_id),
                    Some(s.province_id),
                    format!("Unknown warehouse {id:?}"),
                );
            }
        }
    }
    for r in w.storage_reservations().values() {
        if r.status == "held" {
            if !r.quantity.is_finite() || r.quantity < 0. {
                out.issue(
                    "invalid_storage_reservation",
                    Some(&r.id),
                    None,
                    "Held quantity must be finite/nonnegative",
                );
                continue;
            }
            if let Some(f) = facilities.get(&r.facility_id) {
                if let Some(m) = f.goods.get(&crate::needs::js_string(&r.goods_id.json())) {
                    *occupied.entry(r.facility_id.clone()).or_default() +=
                        r.quantity * m.space_per_unit;
                } else {
                    out.issue(
                        "reservation_warehouse_goods",
                        Some(&r.id),
                        None,
                        "Held reservation commodity mapping absent",
                    );
                }
            } else {
                out.issue(
                    "reservation_warehouse_reference",
                    Some(&r.id),
                    None,
                    "Held reservation warehouse absent",
                );
            }
        }
    }
    for f in w.storage_facilities().values() {
        if let Some(capacity) = f.capacity {
            if !capacity.is_finite()
                || capacity < 0.
                || occupied
                    .get(&f.id)
                    .is_some_and(|q| *q > capacity + 1e-7 * capacity.max(1.))
            {
                out.issue(
                    "warehouse_capacity",
                    Some(&f.id),
                    Some(f.province_id),
                    format!("capacity {capacity}; occupied {:?}", occupied.get(&f.id)),
                );
            }
        } else {
            out.not_measured
                .push(format!("warehouse {:?}: нет измерения вместимости", f.id));
        }
    }
    for p in w.pops().values() {
        if p.account_id.is_none() {
            out.observations.push(serde_json::json!({"pop_id":p.external_id,"province_id":p.province_id.0,"code":"personal_account_absent","measurement":"нет измерения личного остатка"}));
        }
    }
    out.not_measured.extend(
        [
            "full_property_graph: нет измерения; operational references/shares checked",
            "school_medical_outputs: не реализовано",
            "hunger_deaths: не реализовано",
            "sea_passenger_migration: не реализовано",
        ]
        .map(str::to_owned),
    );
    out
}
fn check_assignments(
    out: &mut InitialAudit,
    e: &Producer,
    pops: &HashMap<&ExternalId, &PopRecord>,
) {
    for a in &e.assignments {
        if !pops.contains_key(&a.pop_id) {
            out.issue(
                "assignment_pop_reference",
                Some(&e.id),
                Some(e.province_id),
                format!("Unknown worker {:?}", a.pop_id),
            );
        }
        if !a.workers_count.is_finite()
            || a.workers_count < 0.
            || !a.time_fraction.is_finite()
            || !(0. ..=1.).contains(&a.time_fraction)
        {
            out.issue(
                "invalid_assignment",
                Some(&e.id),
                Some(e.province_id),
                format!("{:?}", a.pop_id),
            );
        }
    }
}
