use kingmaker_core::world_read::WorldRead;
use crate::{
    budget_funding::{error, optional_id},
    needs::{js_truthy, undefined},
    operation_session::OpResult,
};
use kingmaker_core::{budget::Employment, production::*, *};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct LaborAvailability {
    pub occupied: f64,
    pub reserved: f64,
    pub free: f64,
}
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LaborPool {
    pub occupied: f64,
    pub capacity: Option<f64>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct LaborEntry {
    pub employment_id: ExternalId,
    pub pop_id: ExternalId,
    pub contract_id: Option<ExternalId>,
    pub worker_group_id: Option<ExternalId>,
    pub job_id: Option<ExternalId>,
    pub capacity_pool_id: Option<ExternalId>,
    pub building_id: Option<ExternalId>,
    pub workers_count: f64,
    pub time_fraction: f64,
    pub fte: f64,
    pub reserved: bool,
    pub source_id: Option<ExternalId>,
    pub extra: Extra,
}
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LaborRoster {
    pub by_pop: HashMap<ExternalId, LaborAvailability>,
    pub by_employment: HashMap<ExternalId, LaborEntry>,
    pub by_job: HashMap<ExternalId, f64>,
    pub by_pool: HashMap<ExternalId, LaborPool>,
}
pub(crate) const INACTIVE_PROJECTS: [&str; 5] = [
    "completed",
    "cancelled",
    "proposed",
    "awaiting_funding",
    "awaiting_owner_consent",
];
fn number(v: Option<&Value>) -> OpResult<Option<f64>> {
    v.filter(|v| !v.is_null() && !undefined(v))
        .map(|v| {
            v.as_f64()
                .filter(|n| n.is_finite())
                .ok_or_else(|| error("Invalid authored labor number"))
        })
        .transpose()
}
pub(crate) fn active_entries(world: &dyn WorldRead) -> OpResult<Vec<LaborEntry>> {
    let mut employment = HashMap::<&ExternalId, &Employment>::new();
    for e in world.employment().values() {
        if employment.insert(&e.employment_id, e).is_some() {
            return Err(error("Duplicate employment row"));
        }
    }
    let contracts = world.contracts().values()
        .map(|c| (&c.contract_id, c))
        .collect::<HashMap<_, _>>();
    if contracts.len() != world.contracts().len() {
        return Err(error("Duplicate labor contract"));
    }
    let mut jobs = HashMap::new();
    for j in world.jobs().values() {
        let id = optional_id(j.get("job_id"))?
            .or(optional_id(j.get("id"))?)
            .ok_or_else(|| error("Missing labor job identity"))?;
        if jobs.insert(id, j).is_some() {
            return Err(error("Duplicate labor job"));
        }
    }
    let mut ids = HashSet::new();
    let mut entries = vec![];
    let mut add = |id: ExternalId,
                   pop: ExternalId,
                   contract: Option<ExternalId>,
                   workers: f64,
                   time: f64,
                   supplied_fte: Option<f64>,
                   group: Option<ExternalId>,
                   source: Option<ExternalId>,
                   reserved: bool,
                   extra: Extra|
     -> OpResult<()> {
        if id.as_str() == Some("") || !ids.insert(id.clone()) {
            return Err(error("Missing/duplicate production employment"));
        }
        let mirror = employment.get(&id).copied();
        let fte = workers * time;
        if supplied_fte.is_some_and(|n| !n.is_finite() || (n - fte).abs() > 1e-7) {
            return Err(error(&format!("Inconsistent typed assignment FTE: {id:?}")));
        }
        if mirror
            .is_some_and(|e| e.pop_id != pop || !e.fte.is_finite() || (e.fte - fte).abs() > 1e-7)
        {
            return Err(error("Inconsistent mirrored employment assignment"));
        }
        if contract
            .as_ref()
            .zip(mirror.map(|e| &e.contract_id))
            .is_some_and(|(a, b)| a != b)
        {
            return Err(error(&format!(
                "Conflicting assignment/mirror contracts: {id:?}"
            )));
        }
        let worker_group_id = group.or(optional_id(
            mirror.and_then(|e| e.extra.get("worker_group_id")),
        )?);
        let contract_id = contract.or_else(|| mirror.map(|e| e.contract_id.clone()));
        let c = contract_id
            .as_ref()
            .map(|id| {
                contracts
                    .get(id)
                    .copied()
                    .ok_or_else(|| error("Missing employment contract"))
            })
            .transpose()?;
        let job_id = optional_id(extra.get("job_id"))?
            .or(optional_id(mirror.and_then(|e| e.extra.get("job_id")))?)
            .or(optional_id(c.and_then(|c| c.extra.get("job_id")))?);
        let job = job_id.as_ref().and_then(|id| jobs.get(id).copied());
        let pool = optional_id(job.and_then(|j| j.get("capacity_pool_id")))?
            .or(optional_id(
                c.and_then(|c| c.extra.get("capacity_pool_id")),
            )?)
            .or(job_id.clone());
        let building = optional_id(extra.get("building_id"))?
            .or(optional_id(
                mirror.and_then(|e| e.extra.get("building_id")),
            )?)
            .or(optional_id(c.and_then(|c| c.extra.get("building_id")))?);
        let mut merged = mirror.map(|e| e.extra.clone()).unwrap_or_default();
        merged.extend(extra);
        entries.push(LaborEntry {
            employment_id: id,
            pop_id: pop,
            contract_id,
            worker_group_id,
            job_id,
            capacity_pool_id: pool,
            building_id: building,
            workers_count: workers,
            time_fraction: time,
            fte,
            reserved,
            source_id: source,
            extra: merged,
        });
        Ok(())
    };
    for p in world.resource_operations().values()
        .map(|e| &e.base)
        .chain(world.factories().values().map(|e| &e.base))
        .chain(world.artisans().values().map(|e| &e.base))
    {
        for a in &p.assignments {
            let id = a
                .employment_id
                .clone()
                .ok_or_else(|| error("Missing production employment identity"))?;
            add(
                id,
                a.pop_id.clone(),
                a.contract_id.clone(),
                a.workers_count,
                a.time_fraction,
                a.fte,
                a.worker_group_id.clone(),
                Some(p.id.clone()),
                false,
                a.extra.clone(),
            )?;
        }
    }
    let mut inactive = HashSet::new();
    for p in world.construction_projects().values() {
        if INACTIVE_PROJECTS.contains(&p.status.as_str()) {
            inactive.extend(p.assignments.iter().filter_map(|a| a.employment_id.clone()));
        } else {
            for a in &p.assignments {
                add(
                    a.employment_id
                        .clone()
                        .ok_or_else(|| error("Missing builder employment identity"))?,
                    a.pop_id.clone(),
                    a.contract_id.clone(),
                    a.workers_count,
                    a.time_fraction,
                    a.fte,
                    a.worker_group_id.clone(),
                    Some(p.id.clone()),
                    true,
                    a.extra.clone(),
                )?;
            }
        }
    }
    // Drop the closure's mutable borrow before looking up collected identities.
    drop(add);
    for e in world.employment().values() {
        if ids.contains(&e.employment_id)
            || inactive.contains(&e.employment_id)
            || e.extra.get("direct_output_labor").is_some_and(js_truthy)
            || e.extra
                .get("status")
                .and_then(Value::as_str)
                .is_some_and(|s| ["ended", "cancelled"].contains(&s))
        {
            continue;
        }
        let workers = number(e.extra.get("workers_count"))?.unwrap_or(e.fte.ceil());
        let time = number(e.extra.get("time_fraction"))?.unwrap_or(if workers > 0. {
            e.fte / workers
        } else {
            0.
        });
        if !e.fte.is_finite() || (workers * time - e.fte).abs() > 1e-7 {
            return Err(error("Supplied service FTE differs from workers/time"));
        }
        let c = contracts
            .get(&e.contract_id)
            .copied()
            .ok_or_else(|| error("Missing employment contract"))?;
        let job_id = optional_id(e.extra.get("job_id"))?.or(optional_id(c.extra.get("job_id"))?);
        let job = job_id.as_ref().and_then(|id| jobs.get(id).copied());
        entries.push(LaborEntry {
            employment_id: e.employment_id.clone(),
            pop_id: e.pop_id.clone(),
            contract_id: Some(e.contract_id.clone()),
            worker_group_id: optional_id(e.extra.get("worker_group_id"))?,
            job_id: job_id.clone(),
            capacity_pool_id: optional_id(job.and_then(|j| j.get("capacity_pool_id")))?
                .or(optional_id(c.extra.get("capacity_pool_id"))?)
                .or(job_id),
            building_id: optional_id(e.extra.get("building_id"))?
                .or(optional_id(c.extra.get("building_id"))?),
            workers_count: workers,
            time_fraction: time,
            fte: workers * time,
            reserved: false,
            source_id: None,
            extra: e.extra.clone(),
        });
    }
    Ok(entries)
}
pub fn build_labor_roster(world: &dyn WorldRead) -> OpResult<LaborRoster> {
    let pops = world.pops().values()
        .map(|p| (&p.external_id, p))
        .collect::<HashMap<_, _>>();
    if pops.len() != world.pops().len() {
        return Err(error("Duplicate labor POP"));
    }
    let mut jobs = HashMap::new();
    for j in world.jobs().values() {
        let id = optional_id(j.get("job_id"))?
            .or(optional_id(j.get("id"))?)
            .ok_or_else(|| error("Missing labor job identity"))?;
        if jobs.insert(id, j).is_some() {
            return Err(error("Duplicate labor job"));
        }
    }
    let mut roster = LaborRoster {
        by_pop: world.pops().values()
            .map(|p| {
                (
                    p.external_id.clone(),
                    LaborAvailability {
                        free: p.workforce,
                        ..Default::default()
                    },
                )
            })
            .collect(),
        ..Default::default()
    };
    let mut groups = HashMap::<String, (f64, f64)>::new();
    for e in active_entries(world)? {
        let p = pops
            .get(&e.pop_id)
            .ok_or_else(|| error("Unknown labor POP"))?;
        if !e.workers_count.is_finite()
            || e.workers_count < 0.
            || e.workers_count.fract() != 0.
            || e.workers_count > p.workforce
            || !e.time_fraction.is_finite()
            || !(0. ..=1.).contains(&e.time_fraction)
            || !e.fte.is_finite()
            || e.fte < 0.
        {
            return Err(error("Invalid labor workers/time/FTE"));
        }
        if p.extra.get("mobility_status").and_then(Value::as_str) == Some("in_transit")
            && e.fte > 0.
        {
            return Err(error("In-transit population cannot be employed"));
        }
        if e.job_id.as_ref().is_some_and(|id| !jobs.contains_key(id)) {
            return Err(error("Missing employment job"));
        }
        if let Some(id) = &e.worker_group_id {
            let group = groups
                .entry(json!([e.pop_id, id]).to_string())
                .or_insert((e.workers_count, 0.));
            if group.0 != e.workers_count {
                return Err(error("Worker group size changed"));
            }
            group.1 += e.time_fraction;
            if group.1 > 1. + 1e-7 {
                return Err(error("Worker group time exceeds one week"));
            }
        }
        let availability = roster.by_pop.get_mut(&e.pop_id).unwrap();
        availability.occupied += e.fte;
        availability.free -= e.fte;
        if e.reserved {
            availability.reserved += e.fte;
        }
        if availability.free < -1e-7 {
            return Err(error("Labor exceeds workforce"));
        }
        if let Some(id) = &e.job_id {
            *roster.by_job.entry(id.clone()).or_default() += e.fte;
        }
        if let Some(id) = &e.capacity_pool_id {
            roster.by_pool.entry(id.clone()).or_default().occupied += e.fte;
        }
        if roster
            .by_employment
            .insert(e.employment_id.clone(), e)
            .is_some()
        {
            return Err(error("Duplicate actual employment"));
        }
    }
    for (id, fte) in &roster.by_job {
        let cap = number(jobs[id].get("slots_total"))?;
        if cap.is_some_and(|cap| cap < 0. || *fte > cap + 1e-7) {
            return Err(error("Job capacity exceeded"));
        }
    }
    for (id, j) in jobs {
        let pool = optional_id(j.get("capacity_pool_id"))?.unwrap_or(id.clone());
        let Some(entry) = roster.by_pool.get_mut(&pool) else {
            continue;
        };
        let capacity = number(j.get("capacity_pool_total"))?.or(if pool == id {
            number(j.get("slots_total"))?
        } else {
            None
        });
        if let Some(capacity) = capacity {
            if capacity < 0.
                || entry.capacity.is_some_and(|old| old != capacity)
                || entry.occupied > capacity + 1e-7
            {
                return Err(error("Shared capacity exceeded/inconsistent"));
            }
            entry.capacity = Some(capacity);
        }
    }
    Ok(roster)
}
pub fn reconcile_labor(world:&mut WorldState,roster:&LaborRoster)->OpResult<()> {
    crate::world_transaction::execute_atomic(world,|world|reconcile_labor_in(world,roster))
}
pub(crate) fn reconcile_labor_in(world:&mut crate::world_transaction::WorldTransaction<'_>,roster:&LaborRoster)->OpResult<()> {
    if roster.by_pop.len()!=world.pops().len() || world.pops().iter().any(|p|!roster.by_pop.contains_key(&p.external_id)) {
        return Err(error("Roster does not match population"));
    }
    let mut pops=vec![];
    for p in world.pops().iter() {
        let occupied=roster.by_pop[&p.external_id].occupied;
        if !occupied.is_finite() || occupied<0. || occupied>p.workforce+1e-7 {return Err(error("Invalid reconciled employment"));}
        let free=(p.workforce-occupied).max(0.);
        if p.employed.to_bits()!=occupied.to_bits() || p.unemployed.to_bits()!=free.to_bits() {
            let mut p=p.clone();p.employed=occupied;p.unemployed=free;pops.push(p);
        }
    }
    let mut jobs=vec![];
    for (key,j) in world.jobs().keyed() {
        let id=optional_id(j.get("job_id"))?.or(optional_id(j.get("id"))?).ok_or_else(||error("Missing reconciliation job"))?;
        let filled=roster.by_job.get(&id).copied().unwrap_or(0.);
        let total=j.get("slots_total").and_then(Value::as_f64).unwrap_or(0.);
        if !j.is_object() {return Err(error("Job is not an object"));}
        let status=if filled>0. {"staffed"} else if total>0. {"available"} else {"no_capacity"};
        if j.get("slots_filled")!=Some(&json!(filled)) || j.get("employment_status")!=Some(&json!(status)) {
            let mut j=j.clone();let row=j.as_object_mut().unwrap();
            row.insert("slots_filled".into(),json!(filled));row.insert("employment_status".into(),json!(status));jobs.push((key,j));
        }
    }
    for p in pops {world.put_pops(p.external_id.clone(),p)?;}
    for (key,j) in jobs {world.put_jobs(key,j)?;}
    Ok(())
}
