use kingmaker_core::world_read::WorldRead;
use crate::{
    budget_funding::*,
    money_math::{allocate_budget, multiply_money, rate_pay, BudgetRequest as AllocationRequest},
    operation_session::*,
};
use kingmaker_core::{budget::*, operations::*, production::*, *};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

fn service_rows<'a>(s: &OperationSession<'a>) -> OpResult<Vec<&'a Employment>> {
    let excluded = production_employment(s);
    let active = s.active_labor_ids()?;
    Ok(s.base.employment().values()
        .filter(|e| {
            e.fte > 0.
                && active.contains(&e.employment_id)
                && !excluded.contains(&e.employment_id)
                && e.extra.get("direct_output_labor") != Some(&json!(true))
        })
        .collect())
}
pub(crate) fn validate_service_labor(s: &OperationSession) -> OpResult<()> {
    let mut ids = HashSet::new();
    for e in s.base.employment().values() {
        if !ids.insert(&e.employment_id) {
            return Err(error("Duplicate employment identity"));
        }
    }
    let mut used = HashMap::<ExternalId, f64>::new();
    let mut groups = HashMap::<String, (f64, f64)>::new();
    let mut check = |pop: &ExternalId,
                     workers: f64,
                     time: f64,
                     fte: f64,
                     group: Option<&Value>|
     -> OpResult<()> {
        let p = s
            .pop(pop)
            .ok_or_else(|| error("Invalid labor POP reference"))?;
        if !fte.is_finite()
            || fte < 0.
            || !workers.is_finite()
            || workers.fract() != 0.
            || workers < 0.
            || workers > p.workforce
            || !time.is_finite()
            || !(0. ..=1.).contains(&time)
            || (workers * time - fte).abs() > 1e-7
        {
            return Err(error("Invalid labor workers/FTE/time"));
        }
        *used.entry(pop.clone()).or_default() += fte;
        if let Some(group) = group.filter(|g| !g.is_null() && **g != json!("")) {
            let key = json!([pop, group]).to_string();
            let entry = groups.entry(key).or_insert((workers, 0.));
            if entry.0 != workers {
                return Err(error("Worker group size changed"));
            }
            entry.1 += time;
            if entry.1 > 1. + 1e-7 {
                return Err(error("Worker group time exceeds one week"));
            }
        }
        Ok(())
    };
    let producers = s
        .base.resource_operations().values()
        .map(|e| &e.base)
        .chain(s.base.factories().values().map(|e| &e.base))
        .chain(s.base.artisans().values().map(|e| &e.base));
    for a in producers.flat_map(|p| &p.assignments).chain(
        s.base.construction_projects().values()
            .filter(|p| !crate::labor_roster::INACTIVE_PROJECTS.contains(&p.status.as_str()))
            .flat_map(|p| &p.assignments),
    ) {
        let typed_group = a.worker_group_id.as_ref().map(ExternalId::json);
        check(
            &a.pop_id,
            a.workers_count,
            a.time_fraction,
            a.workers_count * a.time_fraction,
            typed_group
                .as_ref()
                .or_else(|| a.extra.get("worker_group_id")),
        )?;
    }
    let mut jobs = HashMap::new();
    for j in s.base.jobs().values() {
        if let Some(id) = optional_id(j.get("job_id"))?.or(optional_id(j.get("id"))?) {
            if jobs.insert(id, j).is_some() {
                return Err(error("Duplicate job identity"));
            }
        }
    }
    let mut pools = HashMap::<ExternalId, f64>::new();
    for e in service_rows(s)? {
        let c = s
            .contract(&e.contract_id)
            .ok_or_else(|| error("Invalid labor contract reference"))?;
        let job_id = optional_id(e.extra.get("job_id"))?
            .or(optional_id(c.extra.get("job_id"))?)
            .ok_or_else(|| error("Actual service job unresolved"))?;
        let job = jobs
            .get(&job_id)
            .ok_or_else(|| error("Actual service job missing"))?;
        let contract_type = optional_id(c.extra.get("pop_type_id"))?;
        // A shared capacity pool labels its primary workforce, not every role.
        let job_type = if job.get("profile_kind") == Some(&json!("capacity_pool"))
            && contract_type.is_some()
        {
            None
        } else {
            optional_id(job.get("pop_type_id"))?
        };
        for expected in [contract_type, job_type].into_iter().flatten() {
            if s.pop(&e.pop_id).is_none_or(|p| p.pop_type_id != expected) {
                return Err(OperationError(format!("Service profession differs from authored job/contract: employment={} pop={} actual={} expected={} job={} contract={}",external_text(&e.employment_id),external_text(&e.pop_id),s.pop(&e.pop_id).map(|p|external_text(&p.pop_type_id)).unwrap_or_default(),external_text(&expected),external_text(&job_id),external_text(&e.contract_id))));
            }
        }
        let workers = labor_number(e.extra.get("workers_count"))?.unwrap_or(e.fte.ceil());
        let time = labor_number(e.extra.get("time_fraction"))?.unwrap_or(if e.fte > 0. {
            e.fte / workers
        } else {
            0.
        });
        check(
            &e.pop_id,
            workers,
            time,
            e.fte,
            e.extra.get("worker_group_id"),
        )?;
        if let Some(pool) =
            optional_id(c.extra.get("capacity_pool_id"))?.or(optional_id(e.extra.get("job_id"))?)
        {
            *pools.entry(pool).or_default() += e.fte;
        }
    }
    for (id, fte) in used {
        if fte > s.pop(&id).unwrap().workforce + 1e-7 {
            return Err(error("Production/service labor exceeds workforce"));
        }
    }
    for (id, fte) in pools {
        if let Some(capacity) = jobs
            .get(&id)
            .and_then(|j| j.get("slots_total"))
            .filter(|v| !v.is_null())
        {
            let n = capacity
                .as_f64()
                .ok_or_else(|| error("Invalid job capacity"))?;
            if !n.is_finite() || n < 0. || fte > n + 1e-7 {
                return Err(error("Labor exceeds capacity"));
            }
        }
    }
    Ok(())
}

fn labor_number(value: Option<&Value>) -> OpResult<Option<f64>> {
    value
        .filter(|v| !v.is_null() && !crate::needs::undefined(v))
        .map(|v| {
            v.as_f64()
                .ok_or_else(|| error("Invalid supplied labor number"))
        })
        .transpose()
}

pub(crate) fn plan_service_payroll(
    s: &mut OperationSession,
    r: &BudgetRequest,
) -> OpResult<Vec<EconomicOperation>> {
    #[derive(Clone)]
    struct Row {
        e: Employment,
        c: LaborContract,
        pop: PopRecord,
        event: ExternalId,
        gross: Money,
        purpose: String,
    }
    let mut grouped: Vec<Vec<Row>> = vec![];
    let mut group_index = HashMap::new();
    let mut ops = vec![];
    let typed = s
        .base.employment().values()
        .any(|e| matches!(e.employment_id, ExternalId::Number(_)));
    for e in service_rows(s)? {
        let p = s.pop(&e.pop_id).unwrap();
        let c = s.contract(&e.contract_id).unwrap();
        let event = crate::production_validation::financial_event_id(
            s,
            "payroll",
            &r.period,
            &e.employment_id,
            typed,
        );
        let initial_plan = s.base.budget_profile_id().as_deref() == Some("strict")
            && e.extra.get("building_id").is_some_and(|id| {
                s.base.institutions().values().any(|b| {
                    b.get("building_id").or_else(|| b.get("id")) == Some(id)
                        && b.get("operational_status") == Some(&json!("initial_plan"))
                })
            });
        let reason = if initial_plan {
            Some("initial_plan_unconfirmed")
        } else if p.pop_type_id.as_str() == Some("slaves") {
            Some("slave_maintenance_required")
        } else if c.currency_id.is_none() || c.employer_entity_id.is_none() {
            Some("unknown_currency_or_employer")
        } else if c.weekly_wage_rate.is_none() && c.wage_weekly_per_worker.is_none() {
            Some("unknown_wage")
        } else {
            None
        };
        if let Some(reason) = reason {
            ops.push(serde_json::from_value::<EconomicOperation>(json!({"event_id":event,"type":"blocked","reason":reason,"employment_id":e.employment_id,"pop_id":p.external_id}))?);
            continue;
        }
        let purpose = payroll_purpose(c)?;
        let gross = rate_pay(&contract_rate(c)?, e.fte, wage_multiplier(s, c)?)?;
        let key = json!([c.employer_entity_id, c.currency_id, purpose]).to_string();
        let n = if let Some(n) = group_index.get(&key) {
            *n
        } else {
            let n = grouped.len();
            grouped.push(vec![]);
            group_index.insert(key, n);
            n
        };
        grouped[n].push(Row {
            e: e.clone(),
            c: c.clone(),
            pop: p.clone(),
            event,
            gross,
            purpose,
        });
    }
    for group in grouped {
        let by_id: HashMap<_, _> = group
            .iter()
            .map(|row| (&row.e.employment_id, row))
            .collect();
        let first = &group[0];
        let cash = s.cash(
            first.c.employer_entity_id.as_ref().unwrap(),
            first.c.currency_id.as_ref().unwrap(),
            &first.purpose,
        );
        for a in allocate_budget(
            &group
                .iter()
                .map(|v| AllocationRequest {
                    id: v.e.employment_id.clone(),
                    amount: v.gross,
                })
                .collect::<Vec<_>>(),
            cash,
        )? {
            let row = by_id[&a.id];
            let p = &row.pop;
            let c = &row.c;
            let e = &row.e;
            let currency = c.currency_id.as_ref().unwrap();
            let employer = c.employer_entity_id.as_ref().unwrap();
            let tax_rate = p
                .state_id
                .as_ref()
                .map(|state| {
                    s.policies
                        .taxes
                        .get(&external_text(state))
                        .copied()
                        .or_else(|| {
                            c.expected_payroll
                                .as_ref()
                                .and_then(|v| v.get("tax_rate"))
                                .and_then(Value::as_f64)
                        })
                        .unwrap_or(0.)
                })
                .unwrap_or(0.);
            if !tax_rate.is_finite() || !(0. ..=1.).contains(&tax_rate) {
                return Err(error("Invalid tax rate"));
            }
            let tax = multiply_money(a.paid, tax_rate)?;
            let net = Money(a.paid.0 - tax.0);
            let treasury_id = if tax_rate > 0. {
                p.state_id.as_ref().map(treasury)
            } else {
                None
            };
            let mut op: EconomicOperation = serde_json::from_value(
                json!({"event_id":row.event,"type":"payroll","period":r.period,"idle_guarantee_minor":Money::ZERO,"employment_id":e.employment_id,"pop_id":p.external_id,"employer_id":employer,"currency_id":currency,"gross_minor":row.gross,"paid_gross_minor":a.paid,"tax_minor":tax,"net_minor":net,"actual_fte":if row.gross.0>0{e.fte*(a.paid.0 as f64)/(row.gross.0 as f64)}else{0.},"planned_fte":e.fte,"tax_rate":tax_rate,"tax_owner_id":treasury_id,"causes":["actual employment FTE","weekly contract rate","finite employer cash"]}),
            )?;
            for key in ["budget_state_id", "budget_sector"] {
                if let Some(v) = c.extra.get(key) {
                    op.meta.insert(key.into(), v.clone());
                }
            }
            if a.paid.0 > 0 {
                op.debits.push(MoneyLeg {
                    owner_id: employer.clone(),
                    currency_id: currency.clone(),
                    purpose: row.purpose.clone(),
                    amount_minor: a.paid,
                });
            }
            if net.0 > 0 {
                op.credits.push(MoneyLeg {
                    owner_id: p.external_id.clone(),
                    currency_id: currency.clone(),
                    purpose: "personal_savings".into(),
                    amount_minor: net,
                });
            }
            if tax.0 > 0 {
                op.credits.push(MoneyLeg {
                    owner_id: treasury_id.clone().unwrap(),
                    currency_id: currency.clone(),
                    purpose: "operating_capital".into(),
                    amount_minor: tax,
                });
            }
            if row.gross > a.paid {
                let mut debt: Obligation = serde_json::from_value(
                    json!({"obligation_id":format!("debt:{}",external_text(&row.event)),"amount_minor":Money(row.gross.0-a.paid.0),"payer_id":employer,"payee_id":p.external_id,"currency_id":currency,"purpose":row.purpose,"tax_rate":tax_rate,"tax_owner_id":treasury_id,"pop_id":p.external_id,"employment_id":e.employment_id,"created_period":r.period}),
                )?;
                for key in ["budget_state_id", "budget_sector"] {
                    if let Some(v) = c.extra.get(key) {
                        debt.extra.insert(key.into(), v.clone());
                    }
                }
                op.obligation = Some(debt);
            }
            s.stage_budget(&op)?;
            ops.push(op);
        }
    }
    for op in &ops {
        if op.kind == OperationKind::Blocked {
            s.stage_budget(op)?;
        }
    }
    Ok(ops)
}
