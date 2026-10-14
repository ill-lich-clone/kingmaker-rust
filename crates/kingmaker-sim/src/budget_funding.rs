use kingmaker_core::world_read::WorldRead;
use crate::{
    money_math::{allocate_budget, multiply_money, rate_pay, BudgetRequest as AllocationRequest},
    operation_session::*,
};
use kingmaker_core::{operations::*, production::*, *};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
pub(crate) fn error(s: &str) -> OperationError {
    OperationError(s.into())
}
pub(crate) const SECTORS: [&str; 4] = ["education", "medicine", "administration", "army"];
pub(crate) fn optional_id(v: Option<&Value>) -> OpResult<Option<ExternalId>> {
    v.filter(|v| {
        !v.is_null()
            && !v
                .get("__kingmaker_codec_v2__")
                .is_some_and(|x| x == "undefined")
    })
    .map(|v| ExternalId::from_json(v.clone()).map_err(OperationError))
    .transpose()
}
pub(crate) fn public_state(c: &LaborContract) -> OpResult<Option<ExternalId>> {
    let state = optional_id(c.extra.get("budget_state_id"))?;
    Ok(state
        .filter(|s| {
            c.employer_entity_id.as_ref() == Some(s)
                || c.employer_entity_id.as_ref() == Some(&treasury(s))
        })
        .filter(|_| {
            c.extra
                .get("budget_sector")
                .and_then(Value::as_str)
                .is_some_and(|s| SECTORS.contains(&s))
        }))
}
pub(crate) fn treasury(state: &ExternalId) -> ExternalId {
    format!("treasury:{}", external_text(state)).into()
}
pub(crate) fn funding(s: &OperationSession, state: &ExternalId, sector: &str) -> OpResult<f64> {
    let key = external_text(state);
    let value = s
        .policies
        .extra
        .get("state_budgets")
        .and_then(|v| v.get(&key))
        .or_else(|| {
            s.base.production_policies()
                .extra
                .get("state_budgets")
                .and_then(|v| v.get(&key))
        })
        .and_then(|v| v.get(sector))
        .filter(|v| !v.is_null());
    let n = value
        .map(|v| v.as_f64().ok_or_else(|| error("Invalid budget funding")))
        .transpose()?
        .unwrap_or(100.);
    if !n.is_finite() || !(0. ..=100.).contains(&n) {
        return Err(error("Invalid budget funding"));
    }
    Ok(n)
}
pub(crate) fn wage_multiplier(s: &OperationSession, c: &LaborContract) -> OpResult<f64> {
    let owner = c
        .employer_entity_id
        .as_ref()
        .ok_or_else(|| error("Missing employer"))?;
    let mut n = s
        .policies
        .wages
        .get(&external_text(owner))
        .copied()
        .unwrap_or(1.);
    if let Some(state) = public_state(c)? {
        n *= funding(s, &state, c.extra["budget_sector"].as_str().unwrap())? / 100.;
    }
    if !n.is_finite() || n < 0. {
        return Err(error("Invalid wage multiplier"));
    }
    Ok(n)
}
pub(crate) fn payroll_purpose(c: &LaborContract) -> OpResult<String> {
    Ok(c.payment_purpose
        .clone()
        .unwrap_or(if public_state(c)?.is_some() {
            format!(
                "state_service:{}",
                c.extra["budget_sector"].as_str().unwrap()
            )
        } else {
            "employer_operating_capital".into()
        }))
}
pub(crate) fn add_money(a: Money, b: Money) -> OpResult<Money> {
    Ok(Money(
        a.0.checked_add(b.0)
            .ok_or_else(|| error("Money overflow"))?,
    ))
}
fn twice(a: Money) -> OpResult<Money> {
    add_money(a, a)
}
fn ordered_groups<T>(rows: impl IntoIterator<Item = (String, T)>) -> Vec<Vec<T>> {
    let mut index = HashMap::new();
    let mut groups: Vec<Vec<T>> = vec![];
    for (key, row) in rows {
        let n = if let Some(n) = index.get(&key) {
            *n
        } else {
            let n = groups.len();
            groups.push(vec![]);
            index.insert(key, n);
            n
        };
        groups[n].push(row);
    }
    groups
}
#[derive(Clone)]
pub(crate) struct Requirement {
    id: ExternalId,
    pub(crate) state: ExternalId,
    pub(crate) owner: ExternalId,
    pub(crate) currency: ExternalId,
    sector: String,
    pub(crate) purpose: String,
    pub(crate) amount: Money,
    weekly: Money,
    full: Money,
}
fn append_requirement(
    rows: &mut Vec<Requirement>,
    index: &mut HashMap<String, usize>,
    mut row: Requirement,
) -> OpResult<()> {
    let key = json!([row.owner, row.currency, row.purpose]).to_string();
    row.id = key.clone().into();
    if let Some(n) = index.get(&key) {
        let prior = &mut rows[*n];
        prior.amount = add_money(prior.amount, row.amount)?;
        prior.weekly = add_money(prior.weekly, row.weekly)?;
        prior.full = add_money(prior.full, row.full)?;
    } else {
        index.insert(key, rows.len());
        rows.push(row);
    }
    Ok(())
}
pub(crate) fn plan_adventurers(
    s: &mut OperationSession,
    period: &str,
) -> OpResult<Vec<EconomicOperation>> {
    let mut rates = HashMap::new();
    for rate in s.base.adventurer_income_rates().values() {
        if rates.insert(&rate.pop_id, rate).is_some() {
            return Err(error("Duplicate adventurer rate"));
        }
    }
    let mut ops = vec![];
    for p in s.base.pops().values() {
        if p.pop_type_id.as_str() != Some("adventurers") || p.size == 0 {
            continue;
        }
        let rate = rates
            .get(&p.external_id)
            .ok_or_else(|| error("Adventurer income rate unresolved"))?;
        if p.currency_id.as_ref() != Some(&rate.currency_id) {
            return Err(error("Adventurer rate currency mismatch"));
        }
        let gross = rate_pay(&rate.rate, f64::from(p.size), 1.)?;
        let tax_rate = p
            .state_id
            .as_ref()
            .map(|id| {
                s.policies
                    .taxes
                    .get(&external_text(id))
                    .copied()
                    .unwrap_or(0.)
            })
            .unwrap_or(0.);
        if !tax_rate.is_finite() || !(0. ..=1.).contains(&tax_rate) {
            return Err(error("Invalid issuance tax"));
        }
        let tax = multiply_money(gross, tax_rate)?;
        let net = Money(gross.0 - tax.0);
        let event = format!(
            "adventurer-income:{}",
            json!([s.base.campaign().campaign_id, period, p.external_id])
        );
        let mut op: EconomicOperation = serde_json::from_value(
            json!({"event_id":event,"type":"adventurer_income","source":"temporary_adventurer_issuance","period":period,"pop_id":p.external_id,"province_id":p.province_id.0,"state_id":p.state_id,"currency_id":rate.currency_id,"income_recognized":true,"issuance_minor":gross,"paid_gross_minor":gross,"gross_minor":gross,"tax_rate":tax_rate,"tax_minor":tax,"net_minor":net,"credits":[{"owner_id":p.external_id,"currency_id":rate.currency_id,"purpose":"personal_savings","amount_minor":net}]}),
        )?;
        if tax.0 > 0 {
            op.credits.push(MoneyLeg {
                owner_id: treasury(p.state_id.as_ref().unwrap()),
                currency_id: rate.currency_id.clone(),
                purpose: "operating_capital".into(),
                amount_minor: tax,
            });
        }
        s.stage_budget(&op)?;
        ops.push(op);
    }
    Ok(ops)
}
pub(crate) fn state_requirements(s: &OperationSession) -> OpResult<Vec<Requirement>> {
    let mut rows = vec![];
    let mut index = HashMap::new();
    for e in s.base.employment().values() {
        if !s.active_labor_ids()?.contains(&e.employment_id) {
            continue;
        }
        let c = s
            .contract(&e.contract_id)
            .ok_or_else(|| error("Missing actual employment contract"))?;
        let Some(state) = public_state(c)? else {
            continue;
        };
        if e.fte <= 0. {
            continue;
        }
        let rate = c
            .weekly_wage_rate
            .as_ref()
            .ok_or_else(|| error("Public wage unresolved"))?;
        let currency = c
            .currency_id
            .as_ref()
            .ok_or_else(|| error("Public wage currency unresolved"))?;
        let owner = c.employer_entity_id.as_ref().unwrap();
        let sector = c.extra["budget_sector"].as_str().unwrap();
        let weekly = rate_pay(rate, e.fte, wage_multiplier(s, c)?)?;
        let full = rate_pay(
            rate,
            e.fte,
            s.policies
                .wages
                .get(&external_text(owner))
                .copied()
                .unwrap_or(1.),
        )?;
        append_requirement(
            &mut rows,
            &mut index,
            Requirement {
                id: "".into(),
                state,
                owner: owner.clone(),
                currency: currency.clone(),
                sector: sector.into(),
                purpose: payroll_purpose(c)?,
                amount: twice(weekly)?,
                weekly,
                full,
            },
        )?;
    }
    for d in s.base.obligations().values() {
        let Some(sector) = d
            .purpose
            .strip_prefix("state_service:")
            .filter(|s| SECTORS.contains(s))
        else {
            continue;
        };
        if d.amount_minor.0 <= 0 {
            continue;
        }
        let state = optional_id(d.extra.get("budget_state_id"))?.unwrap_or_else(|| {
            d.payer_id
                .as_str()
                .and_then(|s| s.strip_prefix("treasury:"))
                .map(ExternalId::from)
                .unwrap_or(d.payer_id.clone())
        });
        append_requirement(
            &mut rows,
            &mut index,
            Requirement {
                id: "".into(),
                state,
                owner: d.payer_id.clone(),
                currency: d.currency_id.clone(),
                sector: sector.into(),
                purpose: d.purpose.clone(),
                amount: d.amount_minor,
                weekly: Money::ZERO,
                full: Money::ZERO,
            },
        )?;
    }
    Ok(rows)
}
pub(crate) fn plan_state_funding(
    s: &mut OperationSession,
    period: &str,
) -> OpResult<(Vec<EconomicOperation>, Vec<Value>)> {
    let rows = state_requirements(s)?;
    let requirements=rows.iter().map(|r|json!({"id":r.id,"state_id":r.state,"owner_id":r.owner,"currency_id":r.currency,"sector":r.sector,"purpose":r.purpose,"amount_minor":r.amount,"weekly_request_minor":r.weekly,"full_weekly_request_minor":r.full})).collect();
    let mut shortages = vec![];
    for mut r in rows {
        r.amount = Money((r.amount.0 - s.cash(&r.owner, &r.currency, &r.purpose).0).max(0));
        if r.amount.0 > 0 {
            shortages.push((json!([r.state, r.currency]).to_string(), r));
        }
    }
    let mut ops = vec![];
    for group in ordered_groups(shortages) {
        let first = &group[0];
        let source = treasury(&first.state);
        let cash = s.cash(&source, &first.currency, "operating_capital");
        let requests = group
            .iter()
            .map(|r| AllocationRequest {
                id: r.id.clone(),
                amount: r.amount,
            })
            .collect::<Vec<_>>();
        for a in allocate_budget(&requests, cash)? {
            if a.paid.0 == 0 {
                continue;
            }
            let r = group.iter().find(|r| r.id == a.id).unwrap();
            let op: EconomicOperation = serde_json::from_value(
                json!({"event_id":format!("state-budget-funding:{}",json!([s.base.campaign().campaign_id,period,r.state,r.currency,r.sector,r.owner])),"type":"state_budget_funding","state_id":r.state,"budget_sector":r.sector,"currency_id":r.currency,"income_recognized":false,"debits":[{"owner_id":source,"currency_id":r.currency,"purpose":"operating_capital","amount_minor":a.paid}],"credits":[{"owner_id":r.owner,"currency_id":r.currency,"purpose":r.purpose,"amount_minor":a.paid}]}),
            )?;
            s.stage_budget(&op)?;
            ops.push(op);
        }
    }
    Ok((ops, requirements))
}
#[derive(Clone)]
struct PrivateRequirement {
    owner: ExternalId,
    currency: ExternalId,
    purpose: String,
    amount: Money,
}
pub(crate) fn plan_private_funding(
    s: &mut OperationSession,
    period: &str,
) -> OpResult<Vec<EconomicOperation>> {
    let Some(policy) = &s.base.foundation_budget_policy() else {
        return Ok(vec![]);
    };
    let Some(source) = policy
        .get("source")
        .filter(|v| !v.is_null() && v.as_str() != Some(""))
    else {
        return Ok(vec![]);
    };
    if policy.get("reserve_weeks") != Some(&json!(2)) {
        return Err(error("Invalid approved budget reserve"));
    }
    let source = source.clone();
    let mut rows: Vec<PrivateRequirement> = vec![];
    let mut index = HashMap::new();
    let mut add =
        |owner: ExternalId, currency: ExternalId, purpose: String, amount: Money| -> OpResult<()> {
            let key = json!([owner, currency, purpose]).to_string();
            if let Some(i) = index.get(&key) {
                let r: &mut PrivateRequirement = &mut rows[*i];
                r.amount = add_money(r.amount, amount)?;
            } else {
                index.insert(key, rows.len());
                rows.push(PrivateRequirement {
                    owner,
                    currency,
                    purpose,
                    amount,
                });
            }
            Ok(())
        };
    for e in s.base.employment().values() {
        if !s.active_labor_ids()?.contains(&e.employment_id) {
            continue;
        }
        let c = s
            .contract(&e.contract_id)
            .ok_or_else(|| error("Missing employment contract"))?;
        if public_state(c)?.is_some() || e.fte <= 0. {
            continue;
        }
        if let Some(rate) = &c.weekly_wage_rate {
            let owner = c
                .employer_entity_id
                .as_ref()
                .ok_or_else(|| error("Private employer unresolved"))?;
            let currency = c
                .currency_id
                .as_ref()
                .ok_or_else(|| error("Private currency unresolved"))?;
            add(
                owner.clone(),
                currency.clone(),
                c.payment_purpose
                    .clone()
                    .unwrap_or("employer_operating_capital".into()),
                twice(rate_pay(
                    rate,
                    e.fte,
                    s.policies
                        .wages
                        .get(&external_text(owner))
                        .copied()
                        .unwrap_or(1.),
                )?)?,
            )?;
        }
    }
    for m in s.base.maintenance_agreements().values() {
        if let Some(v) = m.extra.get("weekly_amount_minor") {
            let amount: Money = serde_json::from_value(v.clone())?;
            add(
                m.payer_id.clone(),
                m.currency_id.clone(),
                "sponsored_maintenance_reserve".into(),
                twice(amount)?,
            )?;
        }
    }
    let mut shortages = vec![];
    for r in &rows {
        let amount = Money((r.amount.0 - s.cash(&r.owner, &r.currency, &r.purpose).0).max(0));
        if amount.0 > 0 {
            shortages.push((
                json!([r.owner, r.currency]).to_string(),
                PrivateRequirement {
                    amount,
                    ..r.clone()
                },
            ));
        }
    }
    let mut ops = vec![];
    for group in ordered_groups(shortages) {
        let first = &group[0];
        let wage_reserve = rows
            .iter()
            .find(|r| {
                r.owner == first.owner
                    && r.currency == first.currency
                    && r.purpose == "employer_operating_capital"
            })
            .map(|r| r.amount)
            .unwrap_or(Money::ZERO);
        let mut paid_by_purpose = HashMap::new();
        let cash = s.cash(&first.owner, &first.currency, "operating_capital");
        let allocations = allocate_budget(
            &group
                .iter()
                .map(|r| AllocationRequest {
                    id: r.purpose.clone().into(),
                    amount: r.amount,
                })
                .collect::<Vec<_>>(),
            cash,
        )?;
        for a in allocations {
            let purpose = a.id.as_str().unwrap();
            if a.paid.0 == 0 || purpose == "operating_capital" {
                continue;
            }
            stage_private(
                s,
                period,
                first,
                purpose,
                "operating_capital",
                "budget-funding",
                &source,
                a.paid,
                &mut ops,
            )?;
            paid_by_purpose.insert(purpose.to_string(), a.paid);
        }
        let employer_cash = s.cash(&first.owner, &first.currency, "employer_operating_capital");
        let excess = Money((employer_cash.0 - wage_reserve.0).max(0));
        let remaining = group
            .iter()
            .filter(|r| r.purpose != "employer_operating_capital")
            .map(|r| AllocationRequest {
                id: r.purpose.clone().into(),
                amount: Money(
                    r.amount.0 - paid_by_purpose.get(&r.purpose).map(|m| m.0).unwrap_or(0),
                ),
            })
            .filter(|r| r.amount.0 > 0)
            .collect::<Vec<_>>();
        for a in allocate_budget(&remaining, excess)? {
            if a.paid.0 > 0 {
                stage_private(
                    s,
                    period,
                    first,
                    a.id.as_str().unwrap(),
                    "employer_operating_capital",
                    "budget-sales-funding",
                    &source,
                    a.paid,
                    &mut ops,
                )?;
            }
        }
    }
    Ok(ops)
}
fn stage_private(
    s: &mut OperationSession,
    period: &str,
    r: &PrivateRequirement,
    purpose: &str,
    from: &str,
    prefix: &str,
    source: &Value,
    amount: Money,
    ops: &mut Vec<EconomicOperation>,
) -> OpResult<()> {
    let op: EconomicOperation = serde_json::from_value(
        json!({"event_id":format!("{prefix}:{period}:{}",json!([r.owner,r.currency,purpose])),"type":"budget_funding","income_recognized":false,"source":source,"debits":[{"owner_id":r.owner,"currency_id":r.currency,"purpose":from,"amount_minor":amount}],"credits":[{"owner_id":r.owner,"currency_id":r.currency,"purpose":purpose,"amount_minor":amount}]}),
    )?;
    s.stage_budget(&op)?;
    ops.push(op);
    Ok(())
}

pub(crate) fn production_employment(s: &OperationSession) -> HashSet<ExternalId> {
    s.base.resource_operations().values()
        .map(|e| &e.base)
        .chain(s.base.factories().values().map(|e| &e.base))
        .chain(s.base.artisans().values().map(|e| &e.base))
        .flat_map(|e| &e.assignments)
        .chain(
            s.base.construction_projects().values()
                .filter(|p| !crate::labor_roster::INACTIVE_PROJECTS.contains(&p.status.as_str()))
                .flat_map(|e| &e.assignments),
        )
        .filter_map(|a| a.employment_id.clone())
        .collect()
}
