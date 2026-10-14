//! Source procurement and land-only lookup; neither construction nor transport executes here.
use kingmaker_core::world_read::WorldRead;
use crate::{
    needs::{finite_or_null, js_string, js_truthy},
    operation_session::*,
    production_activities::method,
};
use kingmaker_core::{market::*, production::*, *};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
fn error(s: &str) -> OperationError {
    OperationError(s.into())
}
fn id(v: &Value) -> OpResult<ExternalId> {
    ExternalId::from_json(v.clone()).map_err(OperationError)
}
fn province(v: &Value) -> OpResult<ProvinceId> {
    ProvinceId::from_external(&id(v)?).map_err(OperationError)
}
fn number(v: Option<&Value>, default: f64) -> f64 {
    finite_or_null(v).unwrap_or(default)
}
fn same(v: &Value, p: ProvinceId) -> bool {
    v.get("province_id")
        .filter(|x| !x.is_null())
        .or_else(|| v.get("asset").and_then(|x| x.get("province_id")))
        .is_some_and(|v| js_string(v) == p.0.to_string())
}
pub fn building_limit_reason(
    w: &dyn WorldRead,
    p: &ConstructionProjectLabor,
) -> OpResult<Option<&'static str>> {
    let value = serde_json::to_value(p)?;
    let prov = province(
        value
            .get("province_id")
            .ok_or_else(|| error("Project province missing"))?,
    )?;
    let active = w.construction_projects().values()
        .filter(|p| !matches!(p.status.as_str(), "cancelled" | "completed" | "proposed"))
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()?;
    let finished = w.completed_construction().values()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|v| same(v, prov))
        .collect::<Vec<_>>();
    let factories = w.factories().values()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|v| same(v, prov) && v.get("status").and_then(Value::as_str) != Some("demolished"))
        .collect::<Vec<_>>();
    let institutions = w.institutions().values().filter(|v| {
        same(v, prov)
            && matches!(
                v.get("operational_status").and_then(Value::as_str),
                Some("operating" | "active" | "completed")
            )
    });
    let built = finished
        .iter()
        .chain(factories.iter())
        .chain(institutions)
        .map(|v| {
            v.get("id")
                .filter(|x| js_truthy(x))
                .or_else(|| v.get("building_id"))
                .cloned()
                .unwrap_or(Value::Null)
                .to_string()
        })
        .collect::<HashSet<_>>();
    let own_id = json!(p.id);
    let target = value.get("target_factory_id").filter(|v| js_truthy(v));
    if built.len()
        + active
            .iter()
            .filter(|p| {
                same(p, prov)
                    && p.get("id") != Some(&own_id)
                    && !p.get("target_factory_id").is_some_and(js_truthy)
            })
            .count()
        + usize::from(target.is_none())
        > 12
    {
        return Ok(Some("province_building_limit"));
    }
    let kind = value.get("kind").and_then(Value::as_str).unwrap_or("");
    if kind == "factory" {
        if let Some(target) = target {
            let f = factories.iter().find(|f| f.get("id") == Some(target));
            if f.is_none_or(|f| {
                f.get("method_id") != value.get("method_id")
                    || number(f.get("level").filter(|v| js_truthy(v)), 1.0) >= 20.0
            }) {
                return Ok(Some("factory_expansion_limit"));
            }
        }
        return Ok(None);
    }
    if matches!(kind, "mine" | "pasture") {
        let raw = w.resource_operations().values().find(|r| {
            Some(&r.base.id.json()) == value.get("target_rgo_id") && r.base.province_id == prov
        });
        let Some(raw) = raw else {
            return Ok(Some("rgo_expansion_target_unresolved"));
        };
        let allowed = if kind == "mine" {
            vec!["coal", "iron", "crystals"]
        } else {
            vec!["wood", "fibers", "grain", "livestock"]
        };
        if !allowed.contains(&raw.resource_id.as_str().unwrap_or("")) {
            return Ok(Some("rgo_expansion_resource_unavailable"));
        }
        let levels = w.expansions().values()
            .filter(|e| e.rgo_id == raw.base.id && e.kind == kind && e.status == "completed")
            .map(|e| e.completed_levels)
            .sum::<f64>();
        let pending = active
            .iter()
            .filter(|p| {
                p.get("id") != Some(&own_id)
                    && p.get("kind").and_then(Value::as_str) == Some(kind)
                    && p.get("target_rgo_id") == Some(&raw.base.id.json())
            })
            .count();
        if levels + pending as f64 + 1.0 > if kind == "mine" { 2.0 } else { 5.0 } {
            return Ok(Some("rgo_expansion_level_limit"));
        }
    }
    if let Some(rule) = w.market_construction_catalog().values()
        .find(|r| r.get("id").and_then(Value::as_str) == Some(kind))
    {
        let built = finished
            .iter()
            .filter(|r| r.get("kind").and_then(Value::as_str) == Some(kind))
            .map(|r| number(r.get("level").filter(|v| js_truthy(v)), 1.0))
            .sum::<f64>();
        let pending = active
            .iter()
            .filter(|p| {
                p.get("id") != Some(&own_id)
                    && same(p, prov)
                    && p.get("kind").and_then(Value::as_str) == Some(kind)
            })
            .count();
        if rule.get("unique").is_some_and(js_truthy) && built + pending as f64 > 0.0 {
            return Ok(Some("unique_building_exists"));
        }
        let max = number(rule.get("max_level"), 0.0);
        if max > 0.0 && built + pending as f64 + 1.0 > max {
            return Ok(Some("building_level_limit"));
        }
    }
    Ok(None)
}
pub fn build_foundation_input_orders(
    w: &dyn WorldRead,
    production: &ProductionPhaseReceipt,
) -> OpResult<Vec<MarketOrder>> {
    if production.campaign_id != w.campaign().campaign_id
        || production.request.expected_revision != w.campaign().revision
        || !w.production_phase_receipts().contains(production)
    {
        return Err(error(
            "Procurement requires authenticated native production receipt",
        ));
    }
    let mut held = HashMap::new();
    for r in w.goods_reservations().values() {
        if r.status == "held" {
            *held.entry(r.stock_id.clone()).or_insert(0.0) += r.quantity;
        }
    }
    let mut owned = HashMap::new();
    let mut raw = HashMap::new();
    for s in w.stockpiles().values() {
        if s.purpose == "food_reserve" {
            continue;
        }
        let key = (s.owner_id.clone(), s.province_id, s.goods.external_id());
        *raw.entry(key.clone()).or_insert(0.0) += s.quantity;
        *owned.entry(key).or_insert(0.0) +=
            s.quantity - held.get(&s.external_id).copied().unwrap_or(0.0);
    }
    let entities = w.factories().values()
        .map(|f| &f.base)
        .chain(w.artisans().values().map(|a| &a.base))
        .map(|e| (&e.id, e))
        .collect::<HashMap<_, _>>();
    let currencies = w.accounts().values()
        .map(|a| {
            (
                (a.owner_id.clone(), a.purpose.clone()),
                a.currency_id.clone(),
            )
        })
        .collect::<HashMap<_, _>>();
    let mut orders = vec![];
    let artisans = w.artisans().values()
        .map(|a| (&a.base.id, a))
        .collect::<HashMap<_, _>>();
    for a in &production.procurement_activities {
        let Some(e) = entities.get(&a.subject_id) else {
            continue;
        };
        let artisan = artisans.get(&a.subject_id).copied();
        let pending = artisan
            .and_then(|e| e.pending_retooling.as_ref())
            .or_else(|| e.extra.get("pending_retooling"))
            .filter(|v| js_truthy(v));
        let mid = pending
            .and_then(|p| p.get("method_id"))
            .filter(|v| js_truthy(v))
            .map(id)
            .transpose()?
            .unwrap_or_else(|| a.method_id.clone());
        let m = method(w, &mid)?;
        let goal = if pending.is_some() {
            e.assignments
                .iter()
                .map(|a| a.workers_count * a.time_fraction)
                .sum::<f64>()
                * m.weekly_productivity
                    .ok_or_else(|| error("Pending method productivity missing"))?
        } else {
            a.potential_output.or(a.labor_output).unwrap_or(a.output)
        };
        let owner = e
            .owner_id
            .as_ref()
            .ok_or_else(|| error("Procurement owner missing"))?;
        let purpose = e.payment_purpose.clone().unwrap_or_else(|| {
            if m.kind == "artisan" {
                "operating_capital".into()
            } else {
                "employer_operating_capital".into()
            }
        });
        let capital = artisan
            .and_then(|a| a.working_capital.as_ref())
            .or_else(|| e.extra.get("working_capital"));
        let currency = currencies
            .get(&(owner.clone(), purpose.clone()))
            .cloned()
            .or_else(|| {
                capital
                    .and_then(|c| c.get("currency_id"))
                    .and_then(|v| id(v).ok())
            });
        for input in &m.inputs {
            let q = (goal * input.amount
                - owned
                    .get(&(owner.clone(), e.province_id, input.goods_id.clone()))
                    .copied()
                    .unwrap_or(0.0))
            .max(0.0);
            if !q.is_finite() {
                return Err(error("Nonfinite procurement demand"));
            }
            if q > 1e-10 {
                let facility = e
                    .extra
                    .get("input_facility_ids")
                    .and_then(|x| x.get(js_string(&input.goods_id.json())))
                    .filter(|v| js_truthy(v))
                    .cloned()
                    .or_else(|| e.facility_id.as_ref().map(ExternalId::json));
                orders.push(serde_json::from_value(json!({"id":format!("input:{}:{}",js_string(&e.id.json()),js_string(&input.goods_id.json())),"owner_id":owner,"province_id":e.province_id.0,"currency_id":currency,"purpose":purpose,"goods_id":input.goods_id,"quantity":q,"priority":"production_inputs","consume":false,"facility_id":facility}))?);
            }
        }
    }
    for p in w.construction_projects().values() {
        if !matches!(p.status.as_str(), "funded" | "building" | "awaiting_labor")
            || p.extra.get("materials_consumed").is_some_and(js_truthy)
            || p.extra
                .get("construction")
                .and_then(|c| c.get("status"))
                .and_then(Value::as_str)
                != Some("ready")
            || building_limit_reason(w, p)?.is_some()
        {
            continue;
        }
        let owner = id(p
            .extra
            .get("owner_id")
            .ok_or_else(|| error("Project owner missing"))?)?;
        let prov = province(
            p.extra
                .get("province_id")
                .ok_or_else(|| error("Project province missing"))?,
        )?;
        let currency = p.extra.get("terms").and_then(|t| t.get("currency_id"));
        let materials = p.extra["construction"]["materials"]
            .as_array()
            .ok_or_else(|| error("Ready project materials missing"))?;
        for m in materials {
            let good = id(m
                .get("goods_id")
                .ok_or_else(|| error("Project material goods missing"))?)?;
            let amount = m
                .get("amount")
                .and_then(|v| finite_or_null(Some(v)))
                .ok_or_else(|| error("Project material amount missing"))?;
            let q = (amount
                - raw
                    .get(&(owner.clone(), prov, good.clone()))
                    .copied()
                    .unwrap_or(0.0))
            .max(0.0);
            if !q.is_finite() {
                return Err(error("Nonfinite project demand"));
            }
            if q > 1e-9 {
                orders.push(serde_json::from_value(json!({"id":format!("construction-input:{}:{}",js_string(&p.id.json()),js_string(&good.json())),"owner_id":owner,"province_id":prov.0,"currency_id":currency,"purpose":"construction_escrow","goods_id":good,"quantity":q,"priority":"production_inputs","consume":false,"facility_id":p.extra.get("material_facility_ids").and_then(|x|x.get(js_string(&good.json())))}))?);
            }
        }
    }
    Ok(orders)
}
pub fn find_land_path(
    data: &MarketData,
    from: ProvinceId,
    to: ProvinceId,
) -> OpResult<Option<Vec<ProvinceId>>> {
    crate::land_paths::LandPathIndex::new(data, 0)?
        .path(from, to).map(|p| p.map(|p| p.to_vec()))
}
