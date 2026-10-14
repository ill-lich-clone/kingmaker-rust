//! Closed evidence justifies a read-only dividend plan; only income_cycle publishes it.
use kingmaker_core::world_read::WorldRead;
use crate::{
    budget_funding::error,
    income_basis::IncomeOpeningBasis,
    income_recognition::{recognize_producer_income, required_income_operations},
    money_math::multiply_money,
    needs::{js_string, js_truthy},
    operation_session::{OpResult, OperationSession},
    producer_reserves::producer_reserves,
    profit_rights::{integer, mixed_producer_rights, validate_shares},
    transport_validation::actor,
};
use kingmaker_core::{
    income::IncomeRequest, operations::EconomicOperation, production::ProductionPolicies,
    ExternalId, Money,
};
use num_bigint::BigInt;
use num_traits::{Signed, ToPrimitive, Zero};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize)]
pub struct ProducerIncomePlan {
    pub operations: Vec<EconomicOperation>,
    pub history: Vec<Value>,
    pub diagnostics: Vec<Value>,
}
pub(crate) fn effective_income_policies(w: &dyn WorldRead, r: &IncomeRequest) -> ProductionPolicies {
    let mut p = w.production_policies().clone();
    p.wages.extend(r.policies.wages.clone());
    p.taxes.extend(r.policies.taxes.clone());
    p.extra.extend(r.policies.extra.clone());
    if !r.policies.priorities.is_empty() {
        p.priorities = r.policies.priorities.clone()
    }
    p
}
fn project_rights(
    w: &dyn WorldRead,
    owner: &ExternalId,
    currency: &ExternalId,
) -> OpResult<Option<Value>> {
    let subjects = w.resource_operations().values()
        .map(|e| &e.base)
        .chain(w.factories().values().map(|e| &e.base));
    let Some(e) = subjects
        .filter(|e| e.owner_id.as_ref() == Some(owner))
        .find(|e| {
            e.extra
                .get("shares")
                .and_then(Value::as_array)
                .is_some_and(|s| !s.is_empty())
        })
    else {
        return Ok(None);
    };
    let investors = w.capitalist_entities().values()
        .filter_map(|i| actor(&i["id"]).ok().map(|id| (id, i)))
        .collect::<HashMap<_, _>>();
    let pops = w.pops().values()
        .map(|p| (&p.external_id, p))
        .collect::<HashMap<_, _>>();
    let mut beneficiaries = vec![];
    for share in e.extra["shares"].as_array().unwrap() {
        let id = actor(&share["investor_id"])?;
        let investor = investors
            .get(&id)
            .ok_or_else(|| error("Factory investor beneficiary identity unresolved"))?;
        let popid = actor(
            investor
                .get("pop_id")
                .filter(|p| js_truthy(p))
                .unwrap_or(&investor["owner_id"]),
        )?;
        let pop = pops
            .get(&popid)
            .ok_or_else(|| error("Factory investor POP missing"))?;
        if investor["owner_id"] != share["owner_id"]
            || pop.pop_type_id.as_str() != Some("capitalists")
            || pop.currency_id.as_ref() != Some(currency)
        {
            return Err(error("Factory investor beneficiary identity unresolved"));
        }
        beneficiaries.push(json!({"owner_id":share["owner_id"],"purpose":investor["account_purpose"],"pop_id":pop.external_id,"state_id":pop.state_id,"numerator":share["numerator"],"denominator":share["denominator"]}));
    }
    Ok(Some(
        json!({"source":format!("actual project investments:{}",js_string(&e.id.json())),"beneficiaries":beneficiaries}),
    ))
}
pub fn plan_producer_income(
    w: &dyn WorldRead,
    r: &IncomeRequest,
    basis: &IncomeOpeningBasis,
) -> OpResult<ProducerIncomePlan> {
    let policies = effective_income_policies(w, r);
    let mut rows = recognize_producer_income(w, r, basis)?;
    let operations = required_income_operations(w, r)?;
    let reserve = producer_reserves(w, &r.period, &policies, &mut rows)?;
    let mixed = mixed_producer_rights(w, basis, &operations)?;
    let session = OperationSession::new(w)?;
    let mut result = ProducerIncomePlan {
        operations: vec![],
        history: vec![],
        diagnostics: reserve.diagnostics,
    };
    for row in rows {
        let key = (row.owner_id.clone(), row.currency_id.clone());
        let prior = w.producer_income().values().find(|h| {
            h["owner_id"] == row.owner_id.json() && h["currency_id"] == row.currency_id.json()
        });
        let prior_loss = prior
            .and_then(|h| h.get("loss_carry_minor"))
            .map(integer)
            .transpose()?
            .unwrap_or_else(BigInt::zero);
        if prior_loss.is_negative() {
            return Err(error("Negative producer loss carry"));
        }
        let operating = reserve
            .values
            .get(&key)
            .cloned()
            .unwrap_or_else(BigInt::zero);
        let cash = BigInt::from(
            session
                .cash(
                    &row.owner_id,
                    &row.currency_id,
                    "employer_operating_capital",
                )
                .0,
        );
        let net = row
            .recognized_cost_minor
            .map(|cost| BigInt::from(row.revenue_minor.0) - BigInt::from(cost.0));
        let carry = net
            .as_ref()
            .map(|net| (&prior_loss - net).max(BigInt::zero()))
            .unwrap_or_else(|| prior_loss.clone());
        let taxable = net
            .as_ref()
            .map(|net| (net - &prior_loss).max(BigInt::zero()))
            .unwrap_or_else(BigInt::zero);
        let unresolved = net.is_none() || reserve.unresolved.contains(&key);
        let budget = if unresolved {
            BigInt::zero()
        } else {
            taxable.min((cash - &operating).max(BigInt::zero()))
        };
        let mut rights = mixed.get(&key).cloned().or_else(|| {
            w.production_beneficiaries().values()
                .find(|v| {
                    v["owner_id"] == row.owner_id.json()
                        && v["currency_id"] == row.currency_id.json()
                })
                .cloned()
        });
        if rights.is_none() && !budget.is_zero() {
            rights = project_rights(w, &row.owner_id, &row.currency_id)?
        }
        let mut distributed = BigInt::zero();
        result.diagnostics.extend(row.diagnostics);
        if !budget.is_zero() {
            if let Some(rights) = rights.filter(|v| js_truthy(&v["source"])) {
                if let Some(shares) = rights["beneficiaries"].as_array().filter(|s| !s.is_empty()) {
                    validate_shares(shares)?;
                    let amount = budget
                        .to_i64()
                        .map(Money)
                        .ok_or_else(|| error("Dividend overflow"))?;
                    let mut credits = vec![];
                    let mut incomes = vec![];
                    let mut used = BigInt::zero();
                    for (i, share) in shares.iter().enumerate() {
                        let gross = if i + 1 == shares.len() {
                            &budget - &used
                        } else {
                            &budget * integer(&share["numerator"])?
                                / integer(&share["denominator"])?
                        };
                        used += &gross;
                        if gross.is_zero() {
                            continue;
                        }
                        let gross = gross
                            .to_i64()
                            .map(Money)
                            .ok_or_else(|| error("Beneficiary amount overflow"))?;
                        let pop = share.get("pop_id").filter(|p| js_truthy(p));
                        if let Some(popid) = pop {
                            let id = actor(popid)?;
                            let p = w.pops().values()
                                .find(|p| p.external_id == id)
                                .ok_or_else(|| error("Production beneficiary POP missing"))?;
                            if p.currency_id.as_ref() != Some(&row.currency_id) {
                                return Err(error("Production beneficiary currency mismatch"));
                            }
                        }
                        let tax_rate = if pop.is_some() {
                            share
                                .get("state_id")
                                .map(js_string)
                                .and_then(|s| policies.taxes.get(&s))
                                .copied()
                                .unwrap_or(0.)
                        } else {
                            0.
                        };
                        if !tax_rate.is_finite() || !(0. ..=1.).contains(&tax_rate) {
                            return Err(error("Invalid producer income tax"));
                        }
                        let tax = multiply_money(gross, tax_rate)?;
                        credits.push(json!({"owner_id":share["owner_id"],"currency_id":row.currency_id,"purpose":share["purpose"],"amount_minor":Money(gross.0-tax.0)}));
                        if let Some(pop) = pop {
                            incomes.push(json!({"pop_id":pop,"gross_minor":gross,"tax_minor":tax}));
                        }
                        if tax.0 > 0 {
                            let state = share
                                .get("state_id")
                                .filter(|v| !v.is_null())
                                .ok_or_else(|| error("Producer tax state missing"))?;
                            credits.push(json!({"owner_id":format!("treasury:{}",js_string(state)),"currency_id":row.currency_id,"purpose":"operating_capital","amount_minor":tax}));
                        }
                    }
                    result.operations.push(serde_json::from_value(json!({"event_id":format!("producer-dividend:{}:{}",r.period,json!([row.owner_id,row.currency_id])),"type":"enterprise_dividend","owner_id":row.owner_id,"currency_id":row.currency_id,"income_recognized":true,"income_by_pop":incomes,"debits":[{"owner_id":row.owner_id,"currency_id":row.currency_id,"purpose":"employer_operating_capital","amount_minor":amount}],"credits":credits,"rights_source":rights["source"]}))?);
                    distributed = budget;
                }
            }
        }
        result.history.push(json!({"owner_id":row.owner_id,"currency_id":row.currency_id,"period":r.period,"revenue_minor":row.revenue_minor,"opening_stock_revenue_minor":row.opening_stock_revenue_minor,"recognized_cost_minor":row.recognized_cost_minor,"net_income_minor":net.map(|n|n.to_string()),"loss_carry_minor":carry.to_string(),"operating_reserve_minor":operating.to_string(),"distributed_minor":distributed.to_string(),"source":"actual sales of produced stocks minus paid wages, maintenance and consumed inputs; two weekly wage reserve"}));
    }
    Ok(result)
}
