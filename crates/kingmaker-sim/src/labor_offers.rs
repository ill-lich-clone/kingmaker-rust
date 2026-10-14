use kingmaker_core::world_read::WorldRead;
use crate::{
    budget_funding::{error, optional_id},
    labor_roster::LaborRoster,
    operation_session::OpResult,
    production_activities::{method, rgo_capacity},
};
use kingmaker_core::*;
use serde_json::{json, Value};
use std::collections::HashSet;
#[derive(Debug, Clone)]
pub struct JobOffer {
    pub id: ExternalId,
    pub kind: String,
    pub entity_id: ExternalId,
    pub province_id: ProvinceId,
    pub owner_id: ExternalId,
    pub method_id: Option<ExternalId>,
    pub job: Option<Value>,
    pub sector: Option<String>,
    pub capacity: f64,
    pub places: u64,
    pub staffing: Vec<ExternalId>,
    pub purpose: String,
}
#[derive(Debug, Clone, Default)]
pub struct OfferResult {
    pub offers: Vec<JobOffer>,
    pub blocked: Vec<Value>,
}
pub fn build_job_offers(w: &dyn WorldRead, roster: &LaborRoster) -> OpResult<OfferResult> {
    let mut result = OfferResult::default();
    let catalog = w.production_catalog().as_ref();
    for rgo in w.resource_operations().values() {
        let m = catalog.and_then(|c| c.methods.iter().find(|m| m.method_id == rgo.base.method_id));
        if rgo.base.owner_id.is_none()
            || rgo.land_access != Some(true)
            || rgo.resource_status.as_deref() != Some("approved")
            || m.is_none_or(|m| m.kind != "rgo" || m.status != "ready")
        {
            result
                .blocked
                .push(json!({"entity_id":rgo.base.id,"reason":"rgo_not_available"}));
            continue;
        }
        let m = m.unwrap();
        let capacity = rgo_capacity(rgo, &w.expansions())?;
        let heads: f64 = rgo.base.assignments.iter().map(|a| a.workers_count).sum();
        if capacity > heads {
            result.offers.push(JobOffer {
                id: rgo.base.id.clone(),
                kind: "rgo".into(),
                entity_id: rgo.base.id.clone(),
                province_id: rgo.base.province_id,
                owner_id: rgo.base.owner_id.clone().unwrap(),
                method_id: Some(m.method_id.clone()),
                job: None,
                sector: None,
                capacity,
                places: (capacity - heads).floor() as u64,
                staffing: m
                    .staffing
                    .iter()
                    .filter(|p| p.as_str() != Some("slaves"))
                    .cloned()
                    .collect(),
                purpose: "employer_operating_capital".into(),
            });
        }
    }
    for f in w.factories().values() {
        let m = match method(w, &f.base.method_id) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if !f
            .status
            .as_deref()
            .is_some_and(|s| ["operating", "awaiting_staff"].contains(&s))
            || m.status != "ready"
        {
            continue;
        }
        if f.equipment_capacity_weekly
            .is_none_or(|n| !n.is_finite() || n <= 0.)
            || f.equipment_source
                .as_ref()
                .is_none_or(|v| !crate::needs::js_truthy(v))
            || m.technology
                .iter()
                .any(|t| !f.implemented_technologies.contains(t))
        {
            result.blocked.push(json!({"entity_id":f.base.id,"reason":"factory_equipment_or_technology_unavailable"}));
            continue;
        }
        let capacity = f
            .people_slots
            .ok_or_else(|| error("Factory hiring capacity unresolved"))?;
        let heads: f64 = f.base.assignments.iter().map(|a| a.workers_count).sum();
        let places = (capacity - heads).floor();
        if !places.is_finite() {
            return Err(error("Invalid factory hiring places"));
        }
        if places > 0. {
            result.offers.push(JobOffer {
                id: f.base.id.clone(),
                kind: "factory".into(),
                entity_id: f.base.id.clone(),
                province_id: f.base.province_id,
                owner_id: f
                    .base
                    .owner_id
                    .clone()
                    .ok_or_else(|| error("Factory owner unresolved"))?,
                method_id: Some(m.method_id.clone()),
                job: None,
                sector: None,
                capacity,
                places: places as u64,
                staffing: m.staffing.clone(),
                purpose: "employer_operating_capital".into(),
            });
        } else {
            result.blocked.push(
                json!({"entity_id":f.base.id,"reason":"factory_capacity_or_inputs_unavailable"}),
            );
        }
    }
    let states = w.profiles().values()
        .map(|p| optional_id(p.get("state_id")))
        .collect::<OpResult<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect::<HashSet<_>>();
    let buildings = w.institutions().values()
        .map(|b| {
            Ok((
                optional_id(b.get("building_id"))?
                    .or(optional_id(b.get("id"))?)
                    .ok_or_else(|| error("Missing hiring building ID"))?,
                b,
            ))
        })
        .collect::<OpResult<std::collections::HashMap<_, _>>>()?;
    for j in w.jobs().values() {
        let Some(building) =
            optional_id(j.get("building_id"))?.and_then(|id| buildings.get(&id).copied())
        else {
            continue;
        };
        let sector = match j.get("role_id").and_then(Value::as_str) {
            Some("teacher") => "education",
            Some("medic") => "medicine",
            Some("bureaucrat" | "official") => "administration",
            Some("soldier" | "officer") => "army",
            _ => continue,
        };
        let Some(owner) = optional_id(building.get("owner_entity_id"))? else {
            continue;
        };
        if !states.contains(&owner)
            || !building
                .get("operational_status")
                .and_then(Value::as_str)
                .is_some_and(|s| {
                    [
                        "existing",
                        "initial_scenario",
                        "active",
                        "operating",
                        "completed",
                    ]
                    .contains(&s)
                })
        {
            continue;
        }
        let id = optional_id(j.get("job_id"))?.ok_or_else(|| error("Missing service job ID"))?;
        let capacity = j
            .get("slots_total")
            .and_then(Value::as_f64)
            .ok_or_else(|| error("Service capacity unresolved"))?;
        let places = (capacity - roster.by_job.get(&id).copied().unwrap_or(0.)).floor();
        if places > 0. {
            let bid = optional_id(building.get("building_id"))?
                .or(optional_id(building.get("id"))?)
                .unwrap();
            let province = ProvinceId::from_external(
                &optional_id(building.get("province_id"))?
                    .ok_or_else(|| error("Service province unresolved"))?,
            )
            .map_err(crate::operation_session::OperationError)?;
            result.offers.push(JobOffer {
                id: crate::labor_identity::service_offer_id(&id),
                kind: "service".into(),
                entity_id: bid,
                province_id: province,
                owner_id: owner,
                method_id: None,
                job: Some(j.clone()),
                sector: Some(sector.into()),
                capacity,
                places: places as u64,
                staffing: vec![optional_id(j.get("pop_type_id"))?
                    .ok_or_else(|| error("Service profession unresolved"))?],
                purpose: format!("state_service:{sector}"),
            });
        }
    }
    Ok(result)
}
