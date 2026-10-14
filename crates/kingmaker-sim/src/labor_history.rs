//! End-of-period ages use assigned labor and actual paid gross, never promised wages.
use kingmaker_core::world_read::WorldRead;
use crate::{
    budget_funding::error, cohort_assets::text, labor_roster::build_labor_roster, needs::js_truthy,
    operation_session::OpResult,
};
use kingmaker_core::{operations::*, *};
use serde_json::{json, Map, Value};
use std::collections::HashMap;

fn number(n: f64) -> Value {
    if n.fract() == 0. && n >= 0. && n <= 9_007_199_254_740_991. {
        json!(n as u64)
    } else {
        json!(n)
    }
}
/// Pure planner. The closed weekly cycle passes its actual native payroll receipts.
pub fn plan_unemployment_history(
    w: &dyn WorldRead,
    period: &str,
    payroll: &[EconomicOperation],
) -> OpResult<Value> {
    if period.is_empty() {
        return Err(error("Empty labor history period"));
    }
    let old = w.pop_mobility()
        .clone()
        .unwrap_or_else(|| json!({}));
    if !old.is_object() {
        return Err(error("Invalid mobility registry"));
    }
    if old["last_completed_period"].as_str() == Some(period) {
        return Ok(old);
    }
    for key in ["unemployment", "unpaid_weeks"] {
        if old
            .get(key)
            .is_some_and(|v| !v.is_object() && !v.is_null() && !crate::needs::undefined(v))
        {
            return Err(error("Invalid labor history map"));
        }
    }
    let roster = build_labor_roster(w)?;
    let mut unemployment = Map::new();
    for pop in w.pops().values() {
        let key = text(&pop.external_id);
        let previous = old["unemployment"].get(&key);
        let mut cohorts = match previous {
            None | Some(Value::Null) => vec![],
            Some(Value::Array(rows)) => rows.clone(),
            _ => return Err(error("Invalid unemployment cohorts")),
        };
        for c in &cohorts {
            if !c["workers"]
                .as_f64()
                .is_some_and(|n| n.is_finite() && n >= 0.)
                || c["weeks"].as_u64().is_none()
            {
                return Err(error("Invalid unemployment cohort"));
            }
        }
        if pop.extra.get("mobility_status").and_then(Value::as_str) == Some("in_transit") {
            unemployment.insert(key, json!(cohorts));
            continue;
        }
        cohorts.sort_by_key(|c| std::cmp::Reverse(c["weeks"].as_u64().unwrap()));
        let mut free = (pop.workforce
            - roster
                .by_pop
                .get(&pop.external_id)
                .map(|a| a.occupied)
                .unwrap_or(0.))
        .max(0.);
        let mut rows = vec![];
        for c in cohorts {
            let workers = free.min(c["workers"].as_f64().unwrap());
            if workers > 0. {
                let weeks = c["weeks"]
                    .as_u64()
                    .unwrap()
                    .checked_add(1)
                    .ok_or_else(|| error("Unemployment age overflow"))?;
                rows.push(json!({"workers":number(workers),"weeks":weeks}));
            }
            free -= workers;
        }
        if free > 0. {
            rows.push(json!({"workers":number(free),"weeks":1}));
        }
        unemployment.insert(key, json!(rows));
    }
    let mut paid = HashMap::<ExternalId, i128>::new();
    for op in payroll
        .iter()
        .filter(|op| op.kind == OperationKind::Payroll)
    {
        let Some(id) = op.meta.get("employment_id").filter(|id| js_truthy(id)) else {
            continue;
        };
        let id = ExternalId::from_json(id.clone()).map_err(|s| error(&s))?;
        let gross = op
            .meta
            .get("paid_gross_minor")
            .filter(|v| js_truthy(v))
            .map(|v| serde_json::from_value::<Money>(v.clone()))
            .transpose()?
            .unwrap_or(Money::ZERO);
        if gross.0 < 0 {
            return Err(error("Negative paid payroll in history"));
        }
        let value = paid.entry(id).or_default();
        *value = value
            .checked_add(i128::from(gross.0))
            .ok_or_else(|| error("Paid payroll total overflow"))?;
    }
    let mut unpaid = Map::new();
    for e in roster.by_employment.values() {
        if e.contract_id
            .as_ref()
            .is_none_or(|id| !js_truthy(&id.json()))
            && !paid.contains_key(&e.employment_id)
        {
            continue;
        }
        let key = text(&e.employment_id);
        let previous = old["unpaid_weeks"]
            .get(&key)
            .filter(|v| js_truthy(v))
            .map(|v| v.as_u64().ok_or_else(|| error("Invalid unpaid weeks")))
            .transpose()?
            .unwrap_or(0);
        let weeks = if paid.get(&e.employment_id).copied().unwrap_or(0) > 0 {
            0
        } else {
            previous
                .checked_add(1)
                .ok_or_else(|| error("Unpaid age overflow"))?
        };
        unpaid.insert(key, json!(weeks));
    }
    let mut next = old;
    next["last_completed_period"] = json!(period);
    next["unemployment"] = Value::Object(unemployment);
    next["unpaid_weeks"] = Value::Object(unpaid);
    Ok(next)
}
