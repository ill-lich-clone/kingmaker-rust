use kingmaker_core::world_read::WorldRead;
use crate::{money_math::*, operation_session::*, production_feasibility::*};
use kingmaker_core::operations::*;
use kingmaker_core::production::*;
use kingmaker_core::*;
use serde_json::json;
use std::collections::{BTreeMap, HashSet};
fn err(s: &str) -> OperationError {
    OperationError(s.into())
}
pub fn plan_prior_production_obligations(
    s: &mut OperationSession,
    period: &str,
) -> OpResult<Vec<EconomicOperation>> {
    let result = (|| {
        let mut groups: BTreeMap<String, Vec<Obligation>> = BTreeMap::new();
        for d in s.base.obligations().values() {
            let d = s.debt(&d.obligation_id).unwrap();
            if d.amount_minor.0 > 0 {
                groups
                    .entry(json!([d.payer_id, d.currency_id, d.purpose]).to_string())
                    .or_default()
                    .push(d.clone());
            }
        }
        let mut operations = vec![];
        let typed = s
            .base.obligations().values()
            .any(|d| matches!(d.obligation_id, ExternalId::Number(_)));
        let budgets: BTreeMap<_, _> = groups
            .iter()
            .map(|(key, rows)| {
                (
                    key.clone(),
                    s.cash(&rows[0].payer_id, &rows[0].currency_id, &rows[0].purpose),
                )
            })
            .collect();
        for (key, rows) in &groups {
            let allocations = allocate_budget(
                &rows
                    .iter()
                    .map(|d| BudgetRequest {
                        id: d.obligation_id.clone(),
                        amount: d.amount_minor,
                    })
                    .collect::<Vec<_>>(),
                budgets[key],
            )?;
            for allocation in allocations {
                if allocation.paid.0 == 0 {
                    continue;
                }
                let old = rows
                    .iter()
                    .find(|d| d.obligation_id == allocation.id)
                    .unwrap();
                let event = crate::production_validation::financial_event_id(
                    s,
                    "debtpay",
                    period,
                    &old.obligation_id,
                    typed,
                );
                if s.committed(&event).is_some() {
                    continue;
                }
                let tax = multiply_money(allocation.paid, old.tax_rate.unwrap_or(0.0))?;
                let mut op: EconomicOperation = serde_json::from_value(
                    json!({"event_id":event,"type":"debt_payment","paid_gross_minor":allocation.paid,"tax_minor":tax,"net_minor":Money(allocation.paid.0-tax.0),"paid_minor":allocation.paid,"debits":[{"owner_id":old.payer_id,"currency_id":old.currency_id,"purpose":old.purpose,"amount_minor":allocation.paid}],"credits":[]}),
                )?;
                if allocation.paid.0 > 0 {
                    op.credits.push(MoneyLeg {
                        owner_id: old.payee_id.clone(),
                        currency_id: old.currency_id.clone(),
                        purpose: old
                            .payee_purpose
                            .clone()
                            .unwrap_or("personal_savings".into()),
                        amount_minor: Money(allocation.paid.0 - tax.0),
                    });
                }
                for field in ["budget_state_id", "budget_sector"] {
                    if let Some(value) = old.extra.get(field) {
                        op.meta.insert(field.into(), value.clone());
                    }
                }
                if let Some(id) = &old.pop_id {
                    op.meta.insert("pop_id".into(), id.json());
                }
                op.meta.insert("currency_id".into(), old.currency_id.json());
                if tax.0 > 0 {
                    op.credits.push(MoneyLeg {
                        owner_id: old
                            .tax_owner_id
                            .clone()
                            .ok_or_else(|| err("Debt tax owner missing"))?,
                        currency_id: old.currency_id.clone(),
                        purpose: "operating_capital".into(),
                        amount_minor: tax,
                    });
                }
                let mut remaining = old.clone();
                remaining.amount_minor = Money(old.amount_minor.0 - allocation.paid.0);
                op.obligation = Some(remaining);
                s.stage(&op)?;
                operations.push(op);
            }
        }
        Ok(operations)
    })();
    if result.is_err() {
        s.poison();
    }
    result
}
pub fn pay_production_labor(
    s: &mut OperationSession,
    request: &ProductionRequest,
    plan: &FeasibleProductionPlan,
) -> OpResult<Vec<PayrollReceipt>> {
    pay_labor(s, request, &plan.payroll_requests, false)
}
pub(crate) fn pay_construction_labor(
    s: &mut OperationSession,
    request: &ProductionRequest,
    requests: &[PayrollRequest],
) -> OpResult<Vec<PayrollReceipt>> {
    pay_labor(s, request, requests, true)
}
fn pay_labor(
    s: &mut OperationSession,
    request: &ProductionRequest,
    requests: &[PayrollRequest],
    construction: bool,
) -> OpResult<Vec<PayrollReceipt>> {
    let result = (|| {
        let mut groups: BTreeMap<String, Vec<&PayrollRequest>> = BTreeMap::new();
        let mut ids = HashSet::new();
        for r in requests {
            if !ids.insert(&r.employment_id) {
                return Err(err("Duplicate payroll employment"));
            }
            groups
                .entry(json!([r.employer_id, r.currency_id, r.purpose]).to_string())
                .or_default()
                .push(r);
        }
        let mut receipts = vec![];
        let typed = requests
            .iter()
            .any(|r| matches!(r.employment_id, ExternalId::Number(_)));
        let budgets: BTreeMap<_, _> = groups
            .iter()
            .map(|(key, rows)| {
                (
                    key.clone(),
                    s.cash(&rows[0].employer_id, &rows[0].currency_id, &rows[0].purpose),
                )
            })
            .collect();
        for (key, rows) in &groups {
            let allocations = allocate_budget(
                &rows
                    .iter()
                    .map(|r| BudgetRequest {
                        id: r.employment_id.clone(),
                        amount: r.gross_minor,
                    })
                    .collect::<Vec<_>>(),
                budgets[key],
            )?;
            for allocation in allocations {
                let r = *rows
                    .iter()
                    .find(|r| r.employment_id == allocation.id)
                    .unwrap();
                let event = crate::production_validation::financial_event_id(
                    s,
                    "payroll",
                    &request.period,
                    &r.employment_id,
                    typed,
                );
                if let Some(old) = s.committed(&event) {
                    let receipt: PayrollReceipt = serde_json::from_value(old.into_owned())?;
                    if receipt.period != request.period
                        || receipt.employment_id != r.employment_id
                        || receipt.pop_id != r.pop_id
                        || receipt.gross_minor != r.gross_minor
                        || receipt.planned_fte != r.planned_fte
                    {
                        return Err(err("Conflicting imported payroll receipt"));
                    }
                    receipts.push(receipt);
                    continue;
                }
                let contract = s
                    .contract(&r.contract_id)
                    .ok_or_else(|| err("Missing payroll contract"))?;
                let pop = s.pop(&r.pop_id).ok_or_else(|| err("Missing payroll POP"))?;
                if construction
                    && (pop.currency_id.as_ref() != Some(&r.currency_id)
                        || contract.currency_id.as_ref() != Some(&r.currency_id)
                        || contract.employer_entity_id.as_ref() != Some(&r.employer_id))
                {
                    return Err(err("Construction payroll currency or employer mismatch"));
                }
                let rate = if let Some(state) = &pop.state_id {
                    s.policies
                        .taxes
                        .get(&external_text(state))
                        .copied()
                        .or_else(|| {
                            contract
                                .expected_payroll
                                .as_ref()
                                .and_then(|v| v.get("tax_rate"))
                                .and_then(|v| v.as_f64())
                        })
                        .unwrap_or(0.0)
                } else {
                    0.0
                };
                if !rate.is_finite() || !(0.0..=1.0).contains(&rate) {
                    return Err(err("Invalid payroll tax"));
                }
                let tax = multiply_money(allocation.paid, rate)?;
                let net = Money(allocation.paid.0 - tax.0);
                let treasury = if rate > 0.0 {
                    pop.state_id
                        .as_ref()
                        .map(|state| ExternalId::from(format!("treasury:{}", external_text(state))))
                } else {
                    None
                };
                let mut op: EconomicOperation = serde_json::from_value(
                    json!({"event_id":event,"type":"payroll","period":request.period,"employment_id":r.employment_id,"pop_id":r.pop_id,"employer_id":r.employer_id,"activity_id":r.activity_id,"currency_id":r.currency_id,"gross_minor":r.gross_minor,"paid_gross_minor":allocation.paid,"tax_minor":tax,"net_minor":net,"tax_rate":rate,"tax_owner_id":treasury,"planned_fte":r.planned_fte,"actual_fte":if r.gross_minor.0>0 {r.planned_fte*(allocation.paid.0 as f64)/(r.gross_minor.0 as f64)}else{r.planned_fte},"idle_guarantee_minor":r.idle_guarantee_minor,"debits":[],"credits":[]}),
                )?;
                if allocation.paid.0 > 0 {
                    op.debits.push(MoneyLeg {
                        owner_id: r.employer_id.clone(),
                        currency_id: r.currency_id.clone(),
                        purpose: r.purpose.clone(),
                        amount_minor: allocation.paid,
                    });
                }
                if net.0 > 0 {
                    op.credits.push(MoneyLeg {
                        owner_id: r.pop_id.clone(),
                        currency_id: r.currency_id.clone(),
                        purpose: "personal_savings".into(),
                        amount_minor: net,
                    });
                }
                if tax.0 > 0 {
                    op.credits.push(MoneyLeg {
                        owner_id: treasury.clone().unwrap(),
                        currency_id: r.currency_id.clone(),
                        purpose: "operating_capital".into(),
                        amount_minor: tax,
                    });
                }
                if r.gross_minor > allocation.paid {
                    op.obligation = Some(serde_json::from_value(
                        json!({"obligation_id":format!("debt:{}",external_text(&event)),"amount_minor":Money(r.gross_minor.0-allocation.paid.0),"payer_id":r.employer_id,"payee_id":r.pop_id,"currency_id":r.currency_id,"purpose":r.purpose,"tax_rate":rate,"tax_owner_id":treasury,"pop_id":r.pop_id,"employment_id":r.employment_id}),
                    )?);
                }
                if construction {
                    op.meta.insert("subject_id".into(), r.subject_id.json());
                    s.stage_construction(&op)?;
                } else {
                    s.stage(&op)?;
                }
                receipts.push(serde_json::from_value(serde_json::to_value(op)?)?);
            }
        }
        Ok(receipts)
    })();
    if result.is_err() {
        s.poison();
    }
    result
}
