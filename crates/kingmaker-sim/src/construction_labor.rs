//! Current project resources and contracts determine payable labor, never promised work.
use kingmaker_core::world_read::WorldRead;
use crate::{
    budget_funding::error,
    money_math::{allocate_budget, rate_pay, BudgetRequest},
    needs::js_string,
    operation_session::{OpResult, OperationSession},
    production_feasibility::PayrollRequest,
    transport_validation::{actor, province},
};
use kingmaker_core::{construction::ConstructionRequest, Money};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
#[derive(Debug, Clone, Serialize)]
pub struct ConstructionLaborPlan {
    pub requests: Vec<PayrollRequest>,
    pub blocked: Vec<Value>,
}
pub fn plan_construction_labor(
    w: &dyn WorldRead,
    r: &ConstructionRequest,
) -> OpResult<ConstructionLaborPlan> {
    let contracts: HashMap<_, _> = w.contracts().values()
        .map(|c| (&c.contract_id, c))
        .collect();
    let pops: HashMap<_, _> = w.pops().values().map(|p| (&p.external_id, p)).collect();
    let policies = crate::producer_income::effective_income_policies(
        w,
        &kingmaker_core::income::IncomeRequest {
            expected_revision: r.expected_revision,
            period: r.period.clone(),
            policies: r.policies.clone(),
        },
    );
    let mut blocked = vec![];
    let mut groups = BTreeMap::<String, Vec<PayrollRequest>>::new();
    let mut ids = HashSet::new();
    for p in w.construction_projects().values() {
        if !matches!(p.status.as_str(), "funded" | "building" | "awaiting_labor") {
            continue;
        }
        if let Some(reason) = crate::construction_limits::building_limit_reason(w, p)? {
            blocked.push(json!({"project_id":p.id,"reason":reason}));
            continue;
        }
        let c = p.extra.get("construction").unwrap_or(&Value::Null);
        if c["status"] != "ready"
            || !crate::needs::js_truthy(&c["source"])
            || c["currency_id"].is_null()
            || !c["materials"].is_array()
        {
            blocked.push(json!({"project_id":p.id,"reason":"construction_parameters_unresolved"}));
            continue;
        }
        let owner = actor(p.extra.get("owner_id").unwrap_or(&Value::Null))?;
        let currency = actor(&c["currency_id"])?;
        let place = province(p.extra.get("province_id").unwrap_or(&Value::Null))?;
        if p.extra.get("materials_consumed") != Some(&json!(true)) {
            let mut unavailable = false;
            for m in c["materials"].as_array().unwrap() {
                let goods = actor(&m["goods_id"])?;
                let quantity = m["amount"]
                    .as_f64()
                    .filter(|n| n.is_finite() && *n > 0.)
                    .ok_or_else(|| error("Invalid construction material requirement"))?;
                let stock = w.stockpiles().values()
                    .filter(|s| {
                        s.owner_id == owner
                            && s.province_id == place
                            && s.goods.external_id() == goods
                    })
                    .map(|s| s.quantity)
                    .sum::<f64>();
                if stock + 1e-9 < quantity {
                    unavailable = true;
                    break;
                }
            }
            if unavailable {
                blocked
                    .push(json!({"project_id":p.id,"reason":"construction_materials_unavailable"}));
                continue;
            }
        }
        let required = c["labor_required_fte_days"]
            .as_f64()
            .filter(|n| n.is_finite() && *n > 0.)
            .ok_or_else(|| error("Construction labor requirement unresolved"))?;
        let done = p
            .extra
            .get("labor_fte_days")
            .and_then(Value::as_f64)
            .unwrap_or(0.);
        if !done.is_finite() || done < 0. {
            return Err(error("Invalid accumulated construction labor"));
        }
        let remaining = (required - done).max(0.);
        let planned = p
            .assignments
            .iter()
            .map(|a| a.workers_count * a.time_fraction * 7.)
            .sum::<f64>();
        let fraction = if planned > 0. {
            (remaining / planned).min(1.)
        } else {
            0.
        };
        if fraction <= 1e-12 {
            continue;
        }
        for a in &p.assignments {
            let contract = a
                .contract_id
                .as_ref()
                .and_then(|id| contracts.get(id).copied());
            let pop = pops.get(&a.pop_id).copied();
            if contract.is_none_or(|c| {
                c.employer_entity_id.as_ref() != Some(&owner)
                    || c.currency_id.as_ref() != Some(&currency)
                    || c.weekly_wage_rate.is_none()
            }) || pop.is_none_or(|p| {
                !matches!(
                    js_string(&p.pop_type_id.json()).as_str(),
                    "farmers" | "laborers"
                ) || p.province_id != place
            }) {
                blocked.push(
                    json!({"project_id":p.id,"reason":"construction_labor_contract_unresolved"}),
                );
                continue;
            }
            let contract = contract.unwrap();
            let id = a
                .employment_id
                .clone()
                .ok_or_else(|| error("Construction employment identity required"))?;
            if !ids.insert(id.clone()) {
                return Err(error("Duplicate construction payroll employment"));
            }
            let fte = a.workers_count * a.time_fraction * fraction;
            let multiplier = policies
                .wages
                .get(&js_string(&owner.json()))
                .copied()
                .unwrap_or(1.);
            let gross = rate_pay(contract.weekly_wage_rate.as_ref().unwrap(), fte, multiplier)
                .map_err(|e| error(&e.to_string()))?;
            let purpose = contract
                .payment_purpose
                .clone()
                .unwrap_or("construction_escrow".into());
            let request = PayrollRequest {
                employment_id: id,
                pop_id: a.pop_id.clone(),
                contract_id: contract.contract_id.clone(),
                subject_id: p.id.clone(),
                activity_id: format!("construction:{}:{}", r.period, js_string(&p.id.json())),
                gross_minor: gross,
                working_gross_minor: gross,
                idle_guarantee_minor: Money::ZERO,
                planned_fte: fte,
                employer_id: owner.clone(),
                currency_id: currency.clone(),
                purpose: purpose.clone(),
            };
            groups
                .entry(json!([owner, currency, purpose]).to_string())
                .or_default()
                .push(request);
        }
    }
    let session = OperationSession::new(w)?;
    let mut requests = vec![];
    for rows in groups.values() {
        let first = &rows[0];
        let cash = session.cash(&first.employer_id, &first.currency_id, &first.purpose);
        let allocation = allocate_budget(
            &rows
                .iter()
                .map(|r| BudgetRequest {
                    id: r.employment_id.clone(),
                    amount: r.gross_minor,
                })
                .collect::<Vec<_>>(),
            cash,
        )
        .map_err(|e| error(&e.to_string()))?;
        for allocation in allocation {
            let mut r = rows
                .iter()
                .find(|r| r.employment_id == allocation.id)
                .unwrap()
                .clone();
            let fraction = if r.gross_minor.0 > 0 {
                allocation.paid.0 as f64 / r.gross_minor.0 as f64
            } else {
                1.
            };
            r.planned_fte *= fraction;
            r.gross_minor = allocation.paid;
            r.working_gross_minor = allocation.paid;
            requests.push(r);
        }
    }
    Ok(ConstructionLaborPlan { requests, blocked })
}
