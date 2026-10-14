//! Derived finite capacity uses authored rules and installed output, never current employment.
use kingmaker_core::world_read::WorldRead;
use crate::{
    budget_funding::error,
    needs::{js_string, js_truthy},
    operation_session::OpResult,
    production_activities::{province_factory_effect, rgo_capacity},
};
use kingmaker_core::{storage::StorageFacility, *};
use serde_json::Value;
use std::collections::HashMap;
fn number(v: Option<&Value>) -> OpResult<f64> {
    v.and_then(Value::as_f64)
        .filter(|n| n.is_finite() && *n >= 0.)
        .ok_or_else(|| error("Invalid derived warehouse capacity rule"))
}
pub fn derive_storage_capacities(w: &dyn WorldRead) -> OpResult<Vec<StorageFacility>> {
    if !w.storage_facilities().values()
        .any(|f| f.extra.contains_key("capacity_rule"))
    {
        return Ok(w.storage_facilities().values().cloned().collect::<Vec<_>>());
    }
    let indexing=kingmaker_core::diagnostics::scope("storage.capacity.index",kingmaker_core::diagnostics::Visibility::Detail);
    let facilities = w.storage_facilities().values()
        .map(|f| (&f.id, f))
        .collect::<HashMap<_, _>>();
    if facilities.len() != w.storage_facilities().len() {
        return Err(error("Duplicate warehouse"));
    }
    let methods = w.production_catalog()
        .as_ref()
        .into_iter()
        .flat_map(|c| &c.methods)
        .map(|m| (&m.method_id, m))
        .collect::<HashMap<_, _>>();
    indexing.finish(kingmaker_core::diagnostics::Outcome::Ok);
    let installed=kingmaker_core::diagnostics::scope("storage.capacity.installed",kingmaker_core::diagnostics::Visibility::Detail);
    let mut weekly = HashMap::<ExternalId, f64>::new();
    let mut add = |id: Option<&ExternalId>, good: &ExternalId, q: f64| -> OpResult<()> {
        if !q.is_finite() || q < 0. {
            return Err(error("Invalid installed weekly storage quantity"));
        }
        if let Some((id, m)) = id.and_then(|id| {
            facilities
                .get(id)
                .and_then(|f| f.goods.get(&js_string(&good.json())).map(|m| (id, m)))
        }) {
            if !m.space_per_unit.is_finite() || m.space_per_unit <= 0. {
                return Err(error("Invalid warehouse unit mapping"));
            }
            *weekly.entry(id.clone()).or_default() += q * m.space_per_unit;
        }
        Ok(())
    };
    let mut enterprise = |e: &kingmaker_core::production::Producer,
                          method: &kingmaker_core::production::ProductionMethod,
                          output: f64,
                          raw: bool|
     -> OpResult<()> {
        let facility = e.facility_id.as_ref().or_else(|| {
            w.storage_facilities().values()
                .find(|f| {
                    e.owner_id.as_ref() == Some(&f.owner_id) && e.province_id == f.province_id
                })
                .map(|f| &f.id)
        });
        add(facility, &method.output_goods_id, output)?;
        for i in &method.inputs {
            let input = e
                .extra
                .get("input_facility_ids")
                .and_then(|v| v.get(js_string(&i.goods_id.json())))
                .filter(|v| js_truthy(v))
                .map(|v| ExternalId::from_json(v.clone()).map_err(|m| error(&m)))
                .transpose()?;
            add(
                input.as_ref().or(e.facility_id.as_ref()),
                &i.goods_id,
                output * i.amount,
            )?;
        }
        if raw {
            if let Some(state) = w.territory().values()
                .find(|p| p.province_id == e.province_id)
                .and_then(|p| p.extra.get("owner_state_id"))
                .filter(|v| js_truthy(v))
            {
                let id: ExternalId =
                    format!("state-raw:{}:{}", js_string(state), e.province_id.0).into();
                add(Some(&id), &method.output_goods_id, output)?;
            }
        }
        Ok(())
    };
    for r in w.resource_operations().values() {
        if let Some(m) = methods
            .get(&r.base.method_id)
            .filter(|m| m.status == "ready")
        {
            enterprise(
                &r.base,
                m,
                rgo_capacity(r, &w.expansions())?
                    * m.weekly_productivity.unwrap_or(0.),
                true,
            )?;
        }
    }
    for f in w.factories().values() {
        if let Some(m) = methods
            .get(&f.base.method_id)
            .filter(|m| m.status == "ready")
        {
            enterprise(
                &f.base,
                m,
                f.equipment_capacity_weekly.unwrap_or(0.)
                    * province_factory_effect(w, f.base.province_id)?,
                false,
            )?;
        }
    }
    for a in w.artisans().values() {
        let pending = a
            .pending_retooling
            .as_ref()
            .and_then(|p| p.get("method_id"))
            .filter(|v| js_truthy(v))
            .map(|v| ExternalId::from_json(v.clone()).map_err(|m| error(&m)))
            .transpose()?;
        if let Some(m) = methods
            .get(pending.as_ref().unwrap_or(&a.base.method_id))
            .filter(|m| m.status == "ready")
        {
            enterprise(
                &a.base,
                m,
                a.people_slots.unwrap_or(0.) * m.weekly_productivity.unwrap_or(0.),
                false,
            )?;
        }
    }
    installed.finish(kingmaker_core::diagnostics::Outcome::Ok);
    let needs=kingmaker_core::diagnostics::scope("storage.capacity.needs",kingmaker_core::diagnostics::Visibility::Detail);
    if w.needs().is_some() {
        let built = crate::needs::build_need_orders(w, &Default::default())?;
        for o in crate::food_conversion::food_orders(&built.orders)?
            .iter()
            .filter(|o| o.need_id.as_ref().and_then(ExternalId::as_str) == Some("food"))
        {
            let id = w.consumer_facility_by_province()
                .as_ref()
                .and_then(|m| m.get(o.province_id.0.to_string()))
                .filter(|v| js_truthy(v))
                .map(|v| ExternalId::from_json(v.clone()).map_err(|m| error(&m)))
                .transpose()?;
            add(id.as_ref(), &o.goods_id, o.quantity)?;
        }
    }
    needs.finish(kingmaker_core::diagnostics::Outcome::Ok);
    let occupancy=kingmaker_core::diagnostics::scope("storage.capacity.occupancy",kingmaker_core::diagnostics::Visibility::Detail);
    let mut occupied = HashMap::<ExternalId, f64>::new();
    for (id, good, unit, q) in w.stockpiles().values()
        .filter_map(|s| {
            s.facility_id
                .as_ref()
                .map(|id| (id, s.goods.external_id(), s.unit.as_ref(), s.quantity))
        })
        .chain(
            w.storage_reservations().values()
                .filter(|r| r.status == "held")
                .map(|r| {
                    (
                        &r.facility_id,
                        r.goods_id.clone(),
                        Some(&r.unit),
                        r.quantity,
                    )
                }),
        )
    {
        let mapping = facilities
            .get(id)
            .and_then(|f| f.goods.get(&js_string(&good.json())))
            .ok_or_else(|| error("Warehouse occupancy has unresolved facility/goods"))?;
        if unit != Some(&mapping.unit)
            || !q.is_finite()
            || q < 0.
            || !mapping.space_per_unit.is_finite()
            || mapping.space_per_unit <= 0.
        {
            return Err(error("Invalid warehouse occupancy/unit"));
        }
        *occupied.entry(id.clone()).or_default() += q * mapping.space_per_unit;
    }
    occupancy.finish(kingmaker_core::diagnostics::Outcome::Ok);
    kingmaker_core::diagnostics::measure("storage.capacity.finalize",kingmaker_core::diagnostics::Visibility::Detail,|| w.storage_facilities().values()
        .map(|f| {
            let mut next = f.clone();
            if let Some(rule) = f.extra.get("capacity_rule") {
                let capacity = occupied
                    .get(&f.id)
                    .copied()
                    .unwrap_or(0.)
                    .max(number(rule.get("minimum_capacity"))?)
                    .max(
                        number(rule.get("opening_space"))?
                            + number(rule.get("buffer_weeks"))?
                                * weekly.get(&f.id).copied().unwrap_or(0.),
                    );
                if !capacity.is_finite() {
                    return Err(error("Derived warehouse capacity overflow"));
                }
                next.capacity = Some(capacity);
            }
            Ok(next)
        })
        .collect())
}
