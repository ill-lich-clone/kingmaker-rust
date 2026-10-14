use kingmaker_core::world_read::WorldRead;
use crate::{
    budget_funding::{error, optional_id},
    labor_roster::*,
    operation_session::{external_text, OpResult},
};
use kingmaker_core::{budget::Employment, production::*, *};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

#[derive(Clone)]
pub(crate) struct LaborRows {
    pub pops: Vec<PopRecord>,
    pub jobs: Vec<Value>,
    pub employment: Vec<Employment>,
    pub contracts: Vec<LaborContract>,
    pub resources: Vec<ResourceOperation>,
    pub factories: Vec<FactoryEnterprise>,
    pub artisans: Vec<ArtisanEnterprise>,
}
impl LaborRows {
    pub(crate) fn capture(w: &dyn WorldRead) -> Self {
        Self {
            pops: w.pops().values().cloned().collect::<Vec<_>>(),
            jobs: w.jobs().values().cloned().collect::<Vec<_>>(),
            employment: w.employment().values().cloned().collect::<Vec<_>>(),
            contracts: w.contracts().values().cloned().collect::<Vec<_>>(),
            resources: w.resource_operations().values().cloned().collect::<Vec<_>>(),
            factories: w.factories().values().cloned().collect::<Vec<_>>(),
            artisans: w.artisans().values().cloned().collect::<Vec<_>>(),
        }
    }
    fn unchanged(&self, w: &dyn WorldRead) -> bool {
        self.pops == w.pops()
            && self.jobs == w.jobs()
            && self.employment == w.employment()
            && self.contracts == w.contracts()
            && self.resources == w.resource_operations()
            && self.factories == w.factories()
            && self.artisans == w.artisans()
    }
    pub(crate) fn install(self, w: &mut WorldState) {
        w.pops = self.pops;
        w.budget_data.jobs = self.jobs;
        w.budget_data.employment = self.employment;
        w.production_data.contracts = self.contracts;
        w.production_data.resource_operations = self.resources;
        w.production_data.factories = self.factories;
        w.production_data.artisans = self.artisans;
    }
    pub(crate) fn install_in(self,w:&mut crate::world_transaction::WorldTransaction<'_>)->OpResult<()> {
        w.replace_pops(self.pops)?;w.replace_jobs(self.jobs)?;w.replace_employment(self.employment)?;
        w.replace_contracts(self.contracts)?;w.replace_resource_operations(self.resources)?;
        w.replace_factories(self.factories)?;w.replace_artisans(self.artisans)
    }

}
pub struct LaborRowChanges {
    pub released_fte: f64,
    pub released: Vec<Value>,
    pub blocked: Vec<Value>,
    delta:crate::world_transaction::PreparedWorldDelta,
    reads:crate::read_guard::Fingerprint,
}
impl LaborRowChanges {
    pub fn apply(self,w:&mut WorldState)->OpResult<()> {
        crate::world_transaction::execute_atomic(w,|w|self.apply_in(w))
    }
    pub(crate) fn apply_in(self,w:&mut crate::world_transaction::WorldTransaction<'_>)->OpResult<()> {
        if release_reads(w)?!=self.reads {return Err(error("Prepared labor release consulted rows changed"));}
        w.apply_delta(self.delta)
    }
}
fn release_reads(w:&dyn WorldRead)->OpResult<crate::read_guard::Fingerprint> {
    crate::read_guard::fingerprint(&(w.pops(),w.jobs(),w.employment(),w.contracts(),w.resource_operations(),w.factories(),w.artisans(),w.campaign(),w.pop_mobility(),w.institutions(),w.expansions(),w.construction_projects()))
}
pub fn apply_staff_change(
    factory: &FactoryEnterprise,
    departed: u64,
    newcomers: u64,
) -> OpResult<FactoryEnterprise> {
    let staff = factory
        .staff_count
        .ok_or_else(|| error("Unknown factory staff count"))?;
    let tenure = factory
        .tenure_days
        .ok_or_else(|| error("Unknown factory tenure"))?;
    if !staff.is_finite()
        || staff < 0.
        || staff.fract() != 0.
        || staff > 9_007_199_254_740_991.
        || departed as f64 > staff
        || !tenure.is_finite()
        || tenure < 0.
    {
        return Err(error("Invalid factory staff change"));
    }
    let remaining = staff - departed as f64;
    let total = remaining + newcomers as f64;
    if total > 9_007_199_254_740_991. {
        return Err(error("Factory staff overflow"));
    }
    let mut next = factory.clone();
    next.staff_count = Some(total);
    next.tenure_days = Some(if total > 0. {
        tenure * remaining / total
    } else {
        0.
    });
    Ok(next)
}
fn resize_assignment(a: &LaborAssignment, keep: f64) -> LaborAssignment {
    let workers = if a.time_fraction > 0. {
        (keep / a.time_fraction).ceil()
    } else {
        keep.ceil()
    };
    let mut next = a.clone();
    next.workers_count = workers;
    next.time_fraction = if workers > 0. { keep / workers } else { 0. };
    if next.fte.is_some() {
        next.fte = Some(keep);
    }
    if next.extra.contains_key("fte") {
        next.extra.insert("fte".into(), json!(keep));
    }
    next
}
fn resize_assignments(
    rows: &[LaborAssignment],
    retained: &HashMap<ExternalId, f64>,
) -> Vec<LaborAssignment> {
    rows.iter()
        .filter_map(|a| {
            let keep = a.employment_id.as_ref().and_then(|id| retained.get(id));
            match keep {
                Some(keep) if *keep == 0. && a.workers_count * a.time_fraction > 0. => None,
                Some(keep) if *keep != a.workers_count * a.time_fraction => {
                    Some(resize_assignment(a, *keep))
                }
                _ => Some(a.clone()),
            }
        })
        .collect()
}
pub fn release_employment(w: &dyn WorldRead, period: &str) -> OpResult<LaborRowChanges> {
    if period.is_empty() {
        return Err(error("Labor release period required"));
    }
    let mut projected=crate::world_transaction::WorldTransaction::new(w);
    let entries = active_entries(w)?;
    let jobs = w.jobs().values()
        .map(|j| {
            let id = optional_id(j.get("job_id"))?
                .or(optional_id(j.get("id"))?)
                .ok_or_else(|| error("Missing release job ID"))?;
            Ok((id, j))
        })
        .collect::<OpResult<HashMap<_, _>>>()?;
    let contracts = w.contracts().values()
        .map(|c| (&c.contract_id, c))
        .collect::<HashMap<_, _>>();
    let pops = w.pops().values()
        .map(|p| (&p.external_id, p))
        .collect::<HashMap<_, _>>();
    let buildings = w.institutions().values()
        .map(|b| {
            let id = optional_id(b.get("building_id"))?
                .or(optional_id(b.get("id"))?)
                .ok_or_else(|| error("Missing institution ID"))?;
            Ok((id, b))
        })
        .collect::<OpResult<HashMap<_, _>>>()?;
    let mut assets = HashMap::<ExternalId, (ExternalId, Option<String>, bool)>::new();
    let mut heads = HashMap::<ExternalId, f64>::new();
    let mut actual_workplaces = HashSet::new();
    for rgo in w.resource_operations().values() {
        let id = rgo.base.id.clone();
        actual_workplaces.insert(id.clone());
        heads.insert(
            id.clone(),
            crate::production_activities::rgo_capacity(rgo, &w.expansions())?,
        );
        for a in &rgo.base.assignments {
            if let Some(employment) = &a.employment_id {
                assets.insert(
                    employment.clone(),
                    (
                        id.clone(),
                        rgo.base
                            .extra
                            .get("status")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                        rgo.land_access == Some(true),
                    ),
                );
            }
        }
    }
    for factory in w.factories().values() {
        let id = factory.base.id.clone();
        actual_workplaces.insert(id.clone());
        if let Some(cap) = factory.people_slots {
            heads.insert(id.clone(), cap);
        }
        for a in &factory.base.assignments {
            if let Some(e) = &a.employment_id {
                assets.insert(e.clone(), (id.clone(), factory.status.clone(), true));
            }
        }
    }
    for artisan in w.artisans().values() {
        let id = artisan.base.id.clone();
        actual_workplaces.insert(id.clone());
        if let Some(cap) = artisan.people_slots {
            heads.insert(id.clone(), cap);
        }
        for a in &artisan.base.assignments {
            if let Some(e) = &a.employment_id {
                assets.insert(
                    e.clone(),
                    (
                        id.clone(),
                        artisan
                            .base
                            .extra
                            .get("status")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                        true,
                    ),
                );
            }
        }
    }
    for (id, b) in &buildings {
        if b.get("building_type_id")
            .and_then(Value::as_str)
            .is_some_and(|s| ["school", "hospital", "administration", "military_unit"].contains(&s))
        {
            actual_workplaces.insert(id.clone());
        }
    }
    let mut retained = HashMap::new();
    let mut remaining = HashMap::<ExternalId, f64>::new();
    let mut individual = HashMap::<ExternalId, f64>::new();
    let mut released = vec![];
    let mut blocked = vec![];
    for e in &entries {
        let p = pops
            .get(&e.pop_id)
            .ok_or_else(|| error("Missing release POP"))?;
        let c = e
            .contract_id
            .as_ref()
            .and_then(|id| contracts.get(id).copied());
        let job = e.job_id.as_ref().and_then(|id| jobs.get(id).copied());
        let building = e
            .building_id
            .as_ref()
            .and_then(|id| buildings.get(id).copied());
        let asset = assets.get(&e.employment_id);
        if !e.fte.is_finite()
            || e.fte < 0.
            || !e.workers_count.is_finite()
            || e.workers_count < 0.
            || e.workers_count.fract() != 0.
            || !e.time_fraction.is_finite()
            || !(0. ..=1.).contains(&e.time_fraction)
        {
            return Err(error("Invalid employment release labor"));
        }
        let unpaid = w.pop_mobility()
            .as_ref()
            .and_then(|m| m.get("unpaid_weeks"))
            .and_then(|m| m.get(external_text(&e.employment_id)))
            .and_then(Value::as_f64)
            .unwrap_or(0.);
        if e.reserved {
            retained.insert(e.employment_id.clone(), e.fte);
            if unpaid >= 2. {
                blocked.push(
                    json!({"employment_id":e.employment_id,"reason":"protected_construction"}),
                );
            }
            continue;
        }
        let mut keep = e.fte;
        let mut reason = None;
        if unpaid >= 2. {
            keep = 0.;
            reason = Some("two_unpaid_weeks");
        } else if building
            .and_then(|b| b.get("operational_status"))
            .and_then(Value::as_str)
            .is_some_and(|s| ["closed", "demolished", "inactive"].contains(&s))
            || asset
                .and_then(|(_, status, _)| status.as_deref())
                .is_some_and(|s| ["closed", "demolished", "inactive"].contains(&s))
        {
            keep = 0.;
            reason = Some("workplace_closed");
        } else if optional_id(c.and_then(|c| c.extra.get("pop_type_id")))?
            .is_some_and(|expected| expected != p.pop_type_id)
        {
            keep = 0.;
            reason = Some("profession_ineligible");
        }
        if asset.is_some_and(|(_, _, land)| !*land) {
            keep = 0.;
            reason = Some("land_access_unavailable");
        }
        if e.source_id.is_none() {
            let contract_building = optional_id(c.and_then(|c| c.extra.get("building_id")))?;
            if contract_building
                .as_ref()
                .is_some_and(|id| !actual_workplaces.contains(id))
            {
                keep = 0.;
                reason = Some("stale_workplace_reference");
            }
        }
        if let Some((id, _, _)) = asset {
            if e.time_fraction > 0. {
                if let Some(left) = heads.get(id) {
                    if keep > *left * e.time_fraction {
                        keep = (*left * e.time_fraction).max(0.);
                        reason = Some("physical_capacity_reduced");
                    }
                }
            }
        }
        if let Some(job) = job {
            let id = e.job_id.as_ref().unwrap();
            let pool = optional_id(job.get("capacity_pool_id"))?.unwrap_or(id.clone());
            let cap = job
                .get("capacity_pool_total")
                .filter(|v| !v.is_null())
                .or_else(|| job.get("slots_total"))
                .and_then(Value::as_f64);
            if let Some(cap) = cap {
                if !cap.is_finite() || cap < 0. {
                    return Err(error("Invalid release pool capacity"));
                }
                let left = remaining.entry(pool.clone()).or_insert(cap);
                if keep > *left {
                    keep = left.max(0.);
                    reason = Some("capacity_reduced");
                }
                *left -= keep;
            }
            if let Some(cap) = job.get("slots_total").and_then(Value::as_f64) {
                let left = individual.entry(id.clone()).or_insert(cap);
                if keep > *left {
                    let excess = keep - left.max(0.);
                    keep -= excess;
                    if let Some(n) = remaining.get_mut(&pool) {
                        *n += excess;
                    }
                    reason = Some("capacity_reduced");
                }
                *left -= keep;
            }
        }
        if let Some((id, _, _)) = asset {
            if e.time_fraction > 0. {
                if let Some(left) = heads.get_mut(id) {
                    *left -= (keep / e.time_fraction).ceil();
                }
            }
        }
        retained.insert(e.employment_id.clone(), keep);
        if keep < e.fte {
            released.push(json!({"employment_id":e.employment_id,"pop_id":e.pop_id,"fte":e.fte-keep,"reason":reason}));
        }
    }
    for e in w.employment().iter() {
        (|| -> OpResult<()> {
            let Some(keep) = retained.get(&e.employment_id) else {
                return Ok(());
            };
            if *keep == e.fte {
                return Ok(());
            };
            if *keep == 0. {
                projected.remove_employment(e.employment_id.clone())?;return Ok(());
            }
            let mut e=e.clone();
            let time = e
                .extra
                .get("time_fraction")
                .and_then(Value::as_f64)
                .unwrap_or(0.);
            let workers = if time > 0. {
                (keep / time).ceil()
            } else {
                keep.ceil()
            };
            e.fte = *keep;
            e.extra.insert("workers_count".into(), json!(workers));
            e.extra
                .insert("time_fraction".into(), json!(keep / workers));
            if e.extra.contains_key("effective_labor") {
                let mult = e
                    .extra
                    .get("productivity_multiplier")
                    .and_then(Value::as_f64)
                    .unwrap_or(1.);
                e.extra.insert("effective_labor".into(), json!(keep * mult));
            }
            projected.put_employment(e.employment_id.clone(),e)?;
            Ok(())

        })()?;
    }
    for r in w.resource_operations().iter() {
        let assignments=resize_assignments(&r.base.assignments,&retained);
        if assignments!=r.base.assignments {let mut r=r.clone();r.base.assignments=assignments;projected.put_resource_operations(r.base.id.clone(),r)?;}
    }
    for a in w.artisans().iter() {
        let assignments=resize_assignments(&a.base.assignments,&retained);
        if assignments!=a.base.assignments {let mut a=a.clone();a.base.assignments=assignments;projected.put_artisans(a.base.id.clone(),a)?;}
    }
    for f in w.factories().iter() {
        let before:f64=f.base.assignments.iter().map(|a|a.workers_count).sum();
        let assignments=resize_assignments(&f.base.assignments,&retained);
        let after:f64=assignments.iter().map(|a|a.workers_count).sum();
        if assignments!=f.base.assignments || before>after {
            let mut next=if before>after {apply_staff_change(f,(before-after) as u64,0)?} else {f.clone()};
            next.base.assignments=assignments;projected.put_factories(next.base.id.clone(),next)?;
        }
    }
    let roster=build_labor_roster(&projected)?;
    reconcile_labor_in(&mut projected,&roster)?;
    Ok(LaborRowChanges {
        released_fte: released.iter().map(|r| r["fte"].as_f64().unwrap()).sum(),
        released,
        blocked,
        delta:projected.finish()?,
        reads:release_reads(w)?,
    })
}
