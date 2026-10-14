use kingmaker_core::row_store::ReadSequence;
use kingmaker_core::world_read::WorldRead;
use crate::operation_session::*;
use crate::storage::{approved, truthy};
use kingmaker_core::production::*;
use kingmaker_core::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
fn err(s: &str) -> OperationError {
    OperationError(s.into())
}
fn safe_integer(n: f64) -> bool {
    n.is_finite() && n >= 0.0 && n.fract() == 0.0 && n <= 9007199254740991.0
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductionActivity {
    pub activity_id: String,
    pub subject_id: ExternalId,
    pub owner_id: Option<ExternalId>,
    pub province_id: ProvinceId,
    pub method_id: ExternalId,
    pub output_goods_id: ExternalId,
    pub output_unit: Value,
    pub assignments: Vec<LaborAssignment>,
    pub output: f64,
    pub fte: f64,
    pub potential_output: Option<f64>,
    pub labor_output: Option<f64>,
    pub equipment_capacity_weekly: Option<f64>,
    pub inputs: Vec<RecipeInput>,
    pub blocked_reason: Option<String>,
    pub payroll_required: bool,
    pub facility_id: Option<ExternalId>,
    pub storage_reservation_id: Option<ExternalId>,
    pub feasible_fraction: f64,
}
pub fn method<'a>(w: &'a dyn WorldRead, id: &ExternalId) -> OpResult<&'a ProductionMethod> {
    w.production_catalog()
        .as_ref()
        .and_then(|c| c.methods.iter().find(|m| &m.method_id == id))
        .ok_or_else(|| err("Unknown authored method/catalog"))
}
pub fn producer<'a>(w: &'a dyn WorldRead, id: &ExternalId) -> OpResult<&'a Producer> {
    w.resource_operations().values()
        .map(|p| &p.base)
        .chain(w.factories().values().map(|p| &p.base))
        .chain(w.artisans().values().map(|p| &p.base))
        .find(|p| &p.id == id)
        .ok_or_else(|| err("Unknown producer"))
}
pub fn validate_production_assignments(
    pops: &(impl ReadSequence<PopRecord> + ?Sized),
    rows: &(impl ReadSequence<LaborAssignment> + ?Sized),
) -> OpResult<()> {
    let by_id: HashMap<_, _> = pops.iter().map(|p| (&p.external_id, p)).collect();
    let mut used: HashMap<&ExternalId, f64> = HashMap::new();
    let mut groups = HashMap::new();
    for a in rows.iter() {
        let p = by_id
            .get(&a.pop_id)
            .ok_or_else(|| err("Unknown assigned POP"))?;
        if !safe_integer(a.workers_count)
            || !p.workforce.is_finite()
            || a.workers_count > p.workforce
            || !a.time_fraction.is_finite()
            || !(0.0..=1.0).contains(&a.time_fraction)
        {
            return Err(err("Invalid production workers/time"));
        }
        let fte = a.workers_count * a.time_fraction;
        if a.fte
            .is_some_and(|n| !n.is_finite() || (n - fte).abs() > 1e-7)
        {
            return Err(err("Inconsistent production FTE"));
        }
        *used.entry(&a.pop_id).or_default() += fte;
        if let Some(g) = &a.worker_group_id {
            let group = groups
                .entry((&a.pop_id, g))
                .or_insert((a.workers_count, 0.0));
            if group.0 != a.workers_count {
                return Err(err("Worker group size changed"));
            }
            group.1 += a.time_fraction;
            if group.1 > 1.0 + 1e-7 {
                return Err(err("Worker group time exceeds week"));
            }
        }
    }
    for (id, fte) in used {
        if fte > by_id[id].workforce + 1e-7 {
            return Err(OperationError(format!("Labor exceeds workforce {id:?}")));
        }
    }
    Ok(())
}
pub fn rgo_capacity(r: &ResourceOperation, expansions: &(impl ReadSequence<RgoExpansion> + ?Sized)) -> OpResult<f64> {
    let mult = r.initial_multiplier.unwrap_or(1.0);
    if ![1.0, 3.0, 4.0, 6.0].contains(&mult) {
        return Err(err("Unapproved initial RGO multiplier"));
    }
    let mut total = 40000.0 * mult;
    let mut seen = HashSet::new();
    let mut levels: HashMap<&str, f64> = HashMap::new();
    for e in expansions.iter().filter(|e| {
        external_text(&e.rgo_id) == external_text(&r.base.id) && e.status == "completed"
    }) {
        let maximum = if e.kind == "pasture" { 5.0 } else { 2.0 };
        let mapped = if e.kind == "mine" {
            ["coal", "iron", "crystals"].contains(&external_text(&r.resource_id).as_str())
        } else if e.kind == "pasture" {
            ["wood", "fibers", "grain", "livestock"]
                .contains(&external_text(&r.resource_id).as_str())
        } else {
            false
        };
        if !seen.insert(&e.id)
            || !safe_integer(e.completed_levels)
            || e.completed_levels < 1.0
            || e.completed_levels > maximum
            || e.places_per_level != 2000.0
            || !mapped
        {
            return Err(err("Invalid completed RGO expansion"));
        }
        let sum = levels.entry(&e.kind).or_default();
        *sum += e.completed_levels;
        if *sum > maximum {
            return Err(err("Cumulative RGO expansion limit"));
        }
        total += e.completed_levels * 2000.0;
    }
    let mut technologies = HashSet::new();
    for t in &r.technology {
        if !technologies.insert(&t.id)
            || !approved(Some(&t.status), Some(&t.source), t.approval_ref.as_ref())
            || !safe_integer(t.places_delta)
        {
            return Err(err("Invalid RGO technology"));
        }
        total += t.places_delta;
    }
    if !safe_integer(total) {
        return Err(err("Invalid RGO capacity"));
    }
    Ok(total)
}
fn productivity(m: &ProductionMethod) -> OpResult<f64> {
    let p = m.weekly_productivity.unwrap_or(0.0);
    if !p.is_finite() || p < 0.0 || (m.status == "ready" && p <= 0.0) {
        return Err(err("Invalid authored productivity"));
    }
    Ok(p)
}
fn activity(
    base: &Producer,
    m: &ProductionMethod,
    id: String,
    assignments: Vec<LaborAssignment>,
    output: f64,
) -> ProductionActivity {
    ProductionActivity {
        activity_id: id,
        subject_id: base.id.clone(),
        owner_id: base.owner_id.clone(),
        province_id: base.province_id,
        method_id: m.method_id.clone(),
        output_goods_id: m.output_goods_id.clone(),
        output_unit: m.output_unit.clone(),
        fte: assignments
            .iter()
            .map(|a| a.workers_count * a.time_fraction)
            .sum(),
        assignments,
        output,
        potential_output: None,
        labor_output: None,
        equipment_capacity_weekly: None,
        inputs: m
            .inputs
            .iter()
            .cloned()
            .map(|mut i| {
                i.amount *= output;
                i
            })
            .collect(),
        blocked_reason: None,
        payroll_required: true,
        facility_id: base.facility_id.clone(),
        storage_reservation_id: None,
        feasible_fraction: 1.0,
    }
}
pub(crate) fn province_factory_effect(w: &dyn WorldRead, p: ProvinceId) -> OpResult<f64> {
    let mut factor = 1.0;
    for b in w.completed_construction().values() {
        let bp = b.province_id.clone().or_else(|| {
            b.asset
                .as_ref()
                .and_then(|a| a.get("province_id"))
                .cloned()
                .and_then(|v| ExternalId::from_json(v).ok())
        });
        if b.status != "completed" || bp.and_then(|v| ProvinceId::from_external(&v).ok()) != Some(p)
        {
            continue;
        }
        let level = b.level.filter(|v| *v != 0.0).unwrap_or(1.0);
        for (key, value) in &b.effects {
            if key.ends_with("_multiplier") {
                let n = value
                    .as_f64()
                    .filter(|n| n.is_finite() && *n >= 1.0)
                    .ok_or_else(|| err("Invalid infrastructure multiplier"))?;
                if key == "factory_output_multiplier" {
                    factor *= n.powf(level);
                }
            }
        }
    }
    if !factor.is_finite() {
        return Err(err("Infrastructure output overflow"));
    }
    Ok(factor)
}
pub fn plan_production_activities(
    w: &dyn WorldRead,
    request: &ProductionRequest,
) -> OpResult<Vec<ProductionActivity>> {
    let pops: HashMap<_, _> = w.pops().values().map(|p| (&p.external_id, p)).collect();
    let mut result = vec![];
    let mut provinces = HashSet::new();
    let mut subjects = HashSet::new();
    for p in w.resource_operations().values()
        .map(|r| &r.base)
        .chain(w.factories().values().map(|r| &r.base))
        .chain(w.artisans().values().map(|r| &r.base))
    {
        if !subjects.insert(&p.id) {
            return Err(err("Duplicate producer identity"));
        }
    }
    for r in w.resource_operations().values() {
        if !provinces.insert(r.base.province_id) {
            return Err(err("Duplicate province RGO"));
        }
        if r.base.assignments.is_empty() {
            continue;
        }
        if r.resource_status.as_deref() != Some("approved")
            || !r.approval_ref.as_ref().is_some_and(truthy)
            || r.land_access != Some(true)
        {
            return Err(err("Unconfirmed RGO resource/land"));
        }
        let m = method(w, &r.base.method_id)?;
        if m.kind != "rgo"
            || m.status != "ready"
            || m.output_goods_id != r.resource_id
            || !m.inputs.is_empty()
        {
            return Err(err("Invalid raw production method"));
        }
        if w.production_catalog()
            .as_ref()
            .unwrap()
            .goods
            .iter()
            .find(|g| g.goods_id == r.resource_id)
            .is_none_or(|g| g.kind != "raw_resource")
        {
            return Err(err("RGO output is not basic resource"));
        }
        let capacity = rgo_capacity(r, &w.expansions())?;
        if r.base
            .assignments
            .iter()
            .map(|a| a.workers_count)
            .sum::<f64>()
            > capacity
        {
            return Err(err("RGO capacity exceeded"));
        }
        for a in &r.base.assignments {
            let p = pops.get(&a.pop_id).ok_or_else(|| err("Missing RGO POP"))?;
            if p.province_id != r.base.province_id || !m.staffing.contains(&p.pop_type_id) {
                return Err(err("Invalid RGO staff"));
            }
            let unknown = a.productivity_status.as_deref() == Some("unknown");
            let coeff = if unknown {
                0.0
            } else {
                a.productivity_multiplier.unwrap_or(1.0)
            };
            if !coeff.is_finite() || coeff < 0.0 {
                return Err(err("Invalid RGO labor coefficient"));
            }
            let id = json!([
                request.period,
                "rgo",
                r.base.id,
                a.pop_id,
                a.employment_id.as_ref().or(a.worker_group_id.as_ref())
            ])
            .to_string();
            let mut row = activity(
                &r.base,
                m,
                id,
                vec![a.clone()],
                a.workers_count * a.time_fraction * coeff * productivity(m)?,
            );
            if unknown {
                row.blocked_reason = Some("species_labor_coefficient_unknown".into());
            }
            result.push(row);
        }
    }
    for f in w.factories().values() {
        if f.status.as_deref() == Some("operating") {
            result.push(factory_activity(w, f, &pops, &request.period)?);
        }
    }
    let mut available: HashMap<_, f64> = HashMap::new();
    for s in w.stockpiles().values() {
        if s.purpose != "food_reserve" {
            *available
                .entry((s.owner_id.clone(), s.province_id, s.goods.external_id()))
                .or_default() += s.quantity;
        }
    }
    for e in w.artisans().values() {
        let m = method(w, &e.base.method_id)?;
        if m.kind != "artisan" {
            return Err(err("Artisan method mismatch"));
        }
        let mut fte = 0.0;
        let mut workers = 0.0;
        for a in &e.base.assignments {
            let p = pops
                .get(&a.pop_id)
                .ok_or_else(|| err("Missing artisan POP"))?;
            if p.pop_type_id.as_str() != Some("artisans") || p.province_id != e.base.province_id {
                return Err(err("Invalid artisan worker"));
            }
            fte += a.workers_count * a.time_fraction;
            workers += a.workers_count;
        }
        let owner = e
            .base
            .owner_id
            .as_ref()
            .ok_or_else(|| err("Unknown artisan owner"))?;
        if e.people_slots
            .is_none_or(|n| !safe_integer(n) || workers > n)
        {
            return Err(err("Invalid artisan capacity"));
        }
        let blocked = if m.status != "ready" {
            Some("artisan method unresolved")
        } else if e.pending_retooling.as_ref().is_some_and(truthy) {
            Some("retooling in progress")
        } else if m
            .technology
            .iter()
            .any(|t| !e.implemented_technologies.contains(t))
        {
            Some("technology not implemented")
        } else {
            None
        };
        let potential = if blocked.is_some() {
            0.0
        } else {
            fte * productivity(m)?
        };
        let mut out = potential;
        for i in &m.inputs {
            if !i.amount.is_finite() || i.amount <= 0.0 {
                return Err(err("Invalid artisan recipe"));
            }
            out = out.min(
                available
                    .get(&(owner.clone(), e.base.province_id, i.goods_id.clone()))
                    .copied()
                    .unwrap_or(0.0)
                    / i.amount,
            );
        }
        for i in &m.inputs {
            *available
                .entry((owner.clone(), e.base.province_id, i.goods_id.clone()))
                .or_default() -= i.amount * out;
        }
        let mut a = activity(
            &e.base,
            m,
            json!([request.period, "artisan", e.base.id]).to_string(),
            e.base.assignments.clone(),
            out,
        );
        a.potential_output = Some(potential);
        a.fte = if potential > 0.0 {
            fte * out / potential
        } else {
            0.0
        };
        a.payroll_required = false;
        a.blocked_reason = blocked
            .or(if out == 0.0 && potential > 0.0 {
                Some("owned inputs unavailable")
            } else {
                None
            })
            .map(str::to_owned);
        result.push(a);
    }
    let mut rows: Vec<_> = result.iter().flat_map(|a| a.assignments.clone()).collect();
    rows.extend(
        w.construction_projects().values()
            .filter(|p| !crate::labor_roster::INACTIVE_PROJECTS.contains(&p.status.as_str()))
            .flat_map(|p| p.assignments.clone()),
    );
    validate_production_assignments(&w.pops(), &rows)?;
    let mut ids = HashSet::new();
    for a in &result {
        if !a.output.is_finite() || a.output < 0.0 || !ids.insert(&a.activity_id) {
            return Err(err("Invalid or duplicate output activity"));
        }
    }
    Ok(result)
}
pub fn update_factory_tenure(
    factory: &FactoryEnterprise,
    worked_fte: f64,
) -> OpResult<FactoryEnterprise> {
    let staff = factory
        .staff_count
        .ok_or_else(|| err("Unknown factory tenure staff"))?;
    let tenure = factory
        .tenure_days
        .ok_or_else(|| err("Unknown factory tenure days"))?;
    if !safe_integer(staff)
        || !tenure.is_finite()
        || tenure < 0.0
        || !worked_fte.is_finite()
        || worked_fte < 0.0
    {
        return Err(err("Invalid factory tenure change"));
    }
    let days = if staff > 0.0 {
        (7.0 * worked_fte / staff).min(7.0)
    } else {
        0.0
    };
    let mut f = factory.clone();
    f.tenure_days = Some(if staff > 0.0 {
        (tenure * staff + days * staff) / staff
    } else {
        0.0
    });
    Ok(f)
}

pub(crate) fn factory_activity(
    w: &dyn WorldRead,
    f: &FactoryEnterprise,
    pops: &HashMap<&ExternalId, &PopRecord>,
    period: &str,
) -> OpResult<ProductionActivity> {
    let m = method(w, &f.base.method_id)?;
    if m.kind != "factory" {
        return Err(err("Factory method kind mismatch"));
    }
    let (mut direct, mut support, mut workers) = (0.0, 0.0, 0.0);
    for a in &f.base.assignments {
        let p = pops
            .get(&a.pop_id)
            .ok_or_else(|| err("Missing factory POP"))?;
        if p.province_id != f.base.province_id || !m.staffing.contains(&p.pop_type_id) {
            return Err(err("Invalid factory staff"));
        }
        workers += a.workers_count;
        if p.pop_type_id.as_str() == Some("craftsmen") {
            direct += a.workers_count * a.time_fraction;
        } else {
            support += a.workers_count * a.time_fraction;
        }
    }
    if f.people_slots.is_none_or(|v| !safe_integer(v) || v <= 0.0)
        || !f.capacity_source.as_ref().is_some_and(truthy)
        || workers > f.people_slots.unwrap()
    {
        return Err(err("Invalid factory people capacity"));
    }
    let mut blocked = if m.status != "ready" {
        Some("method unresolved")
    } else {
        None
    };
    let mut mult = 1.0;
    if f.equipment_capacity_weekly
        .is_none_or(|v| !v.is_finite() || v <= 0.0)
        || !f.equipment_source.as_ref().is_some_and(truthy)
    {
        blocked = Some("equipment parameters unresolved");
    }
    if m.inputs.is_empty()
        || m.inputs
            .iter()
            .any(|i| !i.amount.is_finite() || i.amount <= 0.0)
    {
        blocked = Some("factory recipe unresolved");
    }
    if m.technology
        .iter()
        .any(|id| !f.implemented_technologies.contains(id))
    {
        blocked = Some("technology not implemented");
    }
    let tenure = f.tenure_days.unwrap_or(0.0);
    if support > 0.0 || tenure > 0.0 {
        if let Some(p) = f.parameters.as_ref().filter(|p| {
            approved(
                p.status.as_deref(),
                p.source.as_ref(),
                p.approval_ref.as_ref(),
            )
        }) {
            if support > 0.0 {
                if let (Some(b), Some(r)) = (
                    p.clerk_bmax.filter(|n| n.is_finite() && *n >= 0.0),
                    p.clerk_ratio.filter(|n| n.is_finite() && *n > 0.0),
                ) {
                    mult += b
                        * (if direct > 0.0 {
                            support / (direct * r)
                        } else {
                            0.0
                        })
                        .min(1.0);
                } else {
                    blocked = Some("clerk parameters unresolved");
                }
            }
            if tenure > 0.0 {
                if let (Some(b), Some(t)) = (
                    p.tenure_bmax.filter(|n| n.is_finite() && *n >= 0.0),
                    p.tenure_tsat.filter(|n| n.is_finite() && *n > 0.0),
                ) {
                    mult += b * (tenure / t).min(1.0);
                } else {
                    blocked = Some("tenure parameters unresolved");
                }
            }
        } else {
            blocked = Some("productivity parameters unresolved");
        }
    }
    let effect = province_factory_effect(w, f.base.province_id)?;
    mult *= effect;
    let equipment = f.equipment_capacity_weekly.unwrap_or(0.0) * effect;
    let labor = if blocked.is_some() {
        0.0
    } else {
        direct * productivity(m)? * mult
    };
    let output = if blocked.is_some() {
        0.0
    } else {
        labor.min(equipment)
    };
    let mut a = activity(
        &f.base,
        m,
        json!([period, "factory", f.base.id]).to_string(),
        f.base.assignments.clone(),
        output,
    );
    a.labor_output = Some(labor);
    a.equipment_capacity_weekly = Some(equipment);
    a.blocked_reason = blocked.map(str::to_owned);
    Ok(a)
}
