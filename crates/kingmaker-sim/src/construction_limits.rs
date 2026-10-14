use kingmaker_core::world_read::WorldRead;
use crate::{budget_funding::error, needs::js_string, operation_session::OpResult};
use kingmaker_core::ProvinceId;
use serde_json::Value;
use std::collections::HashSet;
fn live(status: &str) -> bool {
    !matches!(status, "cancelled" | "completed" | "proposed")
}
pub fn factory_places(w: &dyn WorldRead, region: &Value) -> OpResult<usize> {
    let factories: Vec<_> = w.factories().values()
        .filter(|f| {
            f.status.as_deref() != Some("demolished")
                && f.region_id
                    .as_ref()
                    .is_some_and(|r| js_string(&r.json()) == js_string(region))
        })
        .collect();
    let ids: HashSet<_> = factories.iter().map(|f| &f.base.id).collect();
    if ids.len() != factories.len() {
        return Err(error("Duplicate factory identity"));
    }
    let mut reserved = HashSet::new();
    for p in w.construction_projects().values() {
        if p.extra.get("kind").and_then(Value::as_str) != Some("factory")
            || p.extra
                .get("target_factory_id")
                .is_some_and(|v| !v.is_null())
            || js_string(&p.extra.get("region_id").cloned().unwrap_or(Value::Null))
                != js_string(region)
        {
            continue;
        }
        if p.status == "completed" {
            if !p
                .extra
                .get("factory_id")
                .is_some_and(|id| ids.iter().any(|i| i.json() == *id))
            {
                return Err(error("Completed project missing factory asset"));
            }
        } else if live(&p.status)
            && p.extra.get("place_reserved") == Some(&Value::Bool(true))
            && !reserved.insert(&p.id)
        {
            return Err(error("Duplicate factory place reservation"));
        }
    }
    Ok(8usize.saturating_sub(factories.len() + reserved.len()))
}
pub(crate) fn new_factory_limit(
    w: &dyn WorldRead,
    province: ProvinceId,
) -> OpResult<Option<&'static str>> {
    place_limit(w, province, None, false)
}
fn place_limit(
    w: &dyn WorldRead,
    province: ProvinceId,
    exclude: Option<&kingmaker_core::ExternalId>,
    expansion: bool,
) -> OpResult<Option<&'static str>> {
    let mut built = HashSet::new();
    for f in w.factories().values() {
        if f.base.province_id == province && f.status.as_deref() != Some("demolished") {
            built.insert(f.base.id.json().to_string());
        }
    }
    for b in w.completed_construction().values() {
        let p = b
            .province_id
            .as_ref()
            .map(|v| v.json())
            .or_else(|| b.asset.as_ref().and_then(|a| a.get("province_id").cloned()));
        if p.as_ref()
            .is_some_and(|p| js_string(p) == province.0.to_string())
        {
            if let Some(id) = b.extra.get("id") {
                built.insert(id.to_string());
            }
        }
    }
    for b in w.institutions().values() {
        if b.get("province_id")
            .is_some_and(|p| js_string(p) == province.0.to_string())
            && matches!(
                b["operational_status"].as_str(),
                Some("operating" | "active" | "completed")
            )
        {
            if let Some(id) = b.get("id").or_else(|| b.get("building_id")) {
                built.insert(id.to_string());
            }
        }
    }
    let pending = w.construction_projects().values()
        .filter(|p| {
            Some(&p.id) != exclude
                && live(&p.status)
                && p.extra.get("target_factory_id").is_none_or(Value::is_null)
                && p.extra
                    .get("province_id")
                    .is_some_and(|v| js_string(v) == province.0.to_string())
        })
        .count();
    Ok((built.len() + pending + usize::from(!expansion) > 12).then_some("province_building_limit"))
}

pub(crate) fn building_limit_reason(
    w: &dyn WorldRead,
    p: &kingmaker_core::production::ConstructionProjectLabor,
) -> OpResult<Option<&'static str>> {
    let place =
        crate::transport_validation::province(p.extra.get("province_id").unwrap_or(&Value::Null))?;
    let target = p
        .extra
        .get("target_factory_id")
        .filter(|v| crate::needs::js_truthy(v));
    if let Some(reason) = place_limit(w, place, Some(&p.id), target.is_some())? {
        return Ok(Some(reason));
    }
    let kind = p.extra.get("kind").and_then(Value::as_str).unwrap_or("");
    if kind == "factory" {
        if let Some(target) = target {
            let id = crate::transport_validation::actor(target)?;
            let f = w.factories().values().find(|f| {
                f.base.id == id
                    && f.base.province_id == place
                    && f.status.as_deref() != Some("demolished")
            });
            if f.is_none_or(|f| {
                f.base.method_id.json() != p.extra.get("method_id").cloned().unwrap_or(Value::Null)
                    || f.base
                        .extra
                        .get("level")
                        .and_then(Value::as_f64)
                        .unwrap_or(1.)
                        >= 20.
            }) {
                return Ok(Some("factory_expansion_limit"));
            }
        }
        return Ok(None);
    }
    if matches!(kind, "mine" | "pasture") {
        let id = crate::transport_validation::actor(
            p.extra.get("target_rgo_id").unwrap_or(&Value::Null),
        )?;
        let Some(raw) = w.resource_operations().values()
            .find(|r| r.base.id == id && r.base.province_id == place)
        else {
            return Ok(Some("rgo_expansion_target_unresolved"));
        };
        let allowed: &[&str] = if kind == "mine" {
            &["coal", "iron", "crystals"]
        } else {
            &["wood", "fibers", "grain", "livestock"]
        };
        if !allowed.contains(&js_string(&raw.resource_id.json()).as_str()) {
            return Ok(Some("rgo_expansion_resource_unavailable"));
        }
        let levels = w.expansions().values()
            .filter(|e| e.rgo_id == id && e.kind == kind && e.status == "completed")
            .map(|e| e.completed_levels)
            .sum::<f64>();
        let pending = w.construction_projects().values()
            .filter(|e| {
                e.id != p.id
                    && live(&e.status)
                    && e.extra.get("kind").and_then(Value::as_str) == Some(kind)
                    && e.extra.get("target_rgo_id") == Some(&id.json())
            })
            .count();
        if levels + pending as f64 + 1. > if kind == "mine" { 2. } else { 5. } {
            return Ok(Some("rgo_expansion_level_limit"));
        }
    }
    if let Some(rule) = w.construction_construction_catalog().values()
        .find(|c| c["id"].as_str() == Some(kind))
    {
        let built =
            w.completed_construction().values()
                .filter(|b| {
                    b.extra.get("kind").and_then(Value::as_str) == Some(kind)
                        && (b.province_id.as_ref().map(|v| v.json()).or_else(|| {
                            b.asset.as_ref().and_then(|a| a.get("province_id").cloned())
                        }))
                        .as_ref()
                        .is_some_and(|v| js_string(v) == place.0.to_string())
                })
                .map(|b| b.level.unwrap_or(1.))
                .sum::<f64>();
        let pending = w.construction_projects().values()
            .filter(|e| {
                e.id != p.id
                    && live(&e.status)
                    && e.extra.get("kind").and_then(Value::as_str) == Some(kind)
                    && e.extra
                        .get("province_id")
                        .is_some_and(|v| js_string(v) == place.0.to_string())
            })
            .count();
        if rule["unique"] == true && built + pending as f64 > 0. {
            return Ok(Some("unique_building_exists"));
        }
        let max = rule["max_level"].as_f64().unwrap_or(0.);
        if max > 0. && built + pending as f64 + 1. > max {
            return Ok(Some("building_level_limit"));
        }
    }
    Ok(None)
}
