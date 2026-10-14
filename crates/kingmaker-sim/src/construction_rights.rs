//! Rational rights derive from real contributed capital and recorded actual refunds.
use kingmaker_core::world_read::WorldRead;
use crate::{
    budget_funding::error,
    money_math::{allocate_budget, BudgetRequest},
    needs::js_string,
    operation_session::OpResult,
    transport_validation::actor,
};
use kingmaker_core::{operations::*, ExternalId, Money};
use num_bigint::BigInt;
use num_traits::{ToPrimitive, Zero};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
#[derive(Debug, Clone)]
struct Right {
    investor: ExternalId,
    owner: ExternalId,
    value: BigInt,
    purpose: String,
}
fn integer(v: &Value) -> OpResult<BigInt> {
    js_string(v)
        .parse()
        .map_err(|_| error("Invalid project capital integer"))
}
fn gcd(mut a: BigInt, mut b: BigInt) -> BigInt {
    while !b.is_zero() {
        let r = &a % &b;
        a = b;
        b = r;
    }
    a
}
/// Existing capital remains owned; only the net new contribution changes its weight.
pub(crate) fn complete_expansion(w: &dyn WorldRead, factory: &Value, p: &Value) -> OpResult<Value> {
    let base = &p["expansion_base"];
    let level = base["level"]
        .as_u64()
        .filter(|n| *n > 0 && *n < 20)
        .ok_or_else(|| error("Invalid factory expansion base level"))?;
    let base_shares = base["shares"]
        .as_array()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| error("Existing factory ownership unresolved"))?;
    let accepted = p["existing_owner_acceptances"]
        .as_array()
        .ok_or_else(|| error("Factory expansion owner consent missing"))?;
    if base_shares
        .iter()
        .any(|s| !accepted.contains(&s["investor_id"]))
    {
        return Err(error("Factory expansion owner consent missing"));
    }
    if factory["id"] != p["target_factory_id"]
        || integer(&factory["capital_value_minor"])? != integer(&base["capital_value_minor"])?
        || factory["level"].as_u64().unwrap_or(1) != level
        || factory["shares"] != base["shares"]
    {
        return Err(error("Expansion original capital/rights changed"));
    }
    let incoming = complete_project_ownership(w, p)?;
    let old = integer(&base["capital_value_minor"])?;
    let added = integer(&p["funded_minor"])?
        - p.get("refunded_minor")
            .map(integer)
            .transpose()?
            .unwrap_or_default();
    if old <= BigInt::zero() || added <= BigInt::zero() {
        return Err(error("Invalid expansion capital"));
    }
    let total = &old + &added;
    let mut combined = HashMap::<(ExternalId, ExternalId), (Value, BigInt, BigInt)>::new();
    for (shares, value) in [
        (base_shares, &old),
        (incoming["shares"].as_array().unwrap(), &added),
    ] {
        for s in shares {
            let n = value * integer(&s["numerator"])?;
            let d = &total * integer(&s["denominator"])?;
            if n < BigInt::zero() || d <= BigInt::zero() {
                return Err(error("Invalid expansion share"));
            }
            let entry = combined
                .entry((actor(&s["investor_id"])?, actor(&s["owner_id"])?))
                .or_insert((s.clone(), BigInt::zero(), BigInt::from(1)));
            entry.1 = &entry.1 * &d + n * &entry.2;
            entry.2 *= d;
            let factor = gcd(entry.1.clone(), entry.2.clone());
            entry.1 /= &factor;
            entry.2 /= factor;
        }
    }
    let mut rows = combined.into_values().collect::<Vec<_>>();
    let collator = crate::ordering::collator();
    rows.sort_by(|a, b| {
        collator.compare(
            &js_string(&a.0["investor_id"]),
            &js_string(&b.0["investor_id"]),
        )
    });
    let shares = rows
        .into_iter()
        .map(|(mut s, n, d)| {
            s["numerator"] = json!(n.to_string());
            s["denominator"] = json!(d.to_string());
            s
        })
        .collect::<Vec<_>>();
    let slots = factory["people_slots"]
        .as_f64()
        .filter(|n| n.is_finite() && *n >= 0.)
        .ok_or_else(|| error("Expansion slots unresolved"))?;
    let equipment = factory["equipment_capacity_weekly"]
        .as_f64()
        .filter(|n| n.is_finite() && *n > 0.)
        .ok_or_else(|| error("Expansion equipment unresolved"))?;
    let mut asset = factory.clone();
    asset["level"] = json!(level + 1);
    asset["people_slots"] = json!(slots + 40000.);
    asset["equipment_capacity_weekly"] = json!(equipment * (level + 1) as f64 / level as f64);
    asset["capital_value_minor"] = json!(total.to_string());
    asset["shares"] = json!(shares);
    let mut history = asset
        .get("expansion_history")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    history.push(json!({"project_id":p["id"],"old_capital_minor":old.to_string(),"contribution_basis_minor":added.to_string(),"source":p["construction"]["source"]}));
    asset["expansion_history"] = json!(history);
    Ok(asset)
}
fn rights(w: &dyn WorldRead, p: &Value) -> OpResult<(BigInt, Vec<Right>)> {
    let mut rows = HashMap::<(ExternalId, ExternalId), Right>::new();
    let mut total = BigInt::zero();
    for i in w.project_investments().values()
        .filter(|i| i["project_id"] == p["id"])
    {
        if i["currency_id"] != p["terms"]["currency_id"] {
            return Err(error("Investor contribution currency mismatch"));
        }
        let investor = actor(&i["investor_id"])?;
        let owner = actor(&i["owner_id"])?;
        let value = integer(&i["amount_minor"])?;
        if value <= BigInt::zero() {
            return Err(error("Invalid actual investor contribution"));
        }
        let purpose = i["source_account_purpose"]
            .as_str()
            .filter(|v| !v.is_empty())
            .ok_or_else(|| error("Original investor account purpose unresolved"))?;
        let row = rows
            .entry((investor.clone(), owner.clone()))
            .or_insert(Right {
                investor,
                owner,
                value: BigInt::zero(),
                purpose: purpose.into(),
            });
        if row.purpose != purpose {
            return Err(error(
                "Investor refund purpose requires separate tranche routing",
            ));
        }
        row.value += &value;
        total += value;
    }
    let mut rows: Vec<_> = rows.into_values().collect();
    let collator = crate::ordering::collator();
    rows.sort_by(|a, b| {
        collator.compare(
            &json!([a.investor, a.owner]).to_string(),
            &json!([b.investor, b.owner]).to_string(),
        )
    });
    Ok((total, rows))
}
pub fn complete_project_ownership(w: &dyn WorldRead, p: &Value) -> OpResult<Value> {
    let (total, rows) = rights(w, p)?;
    if total <= BigInt::zero()
        || total != integer(&p["funded_minor"])?
        || total != integer(&p["terms"]["budget_minor"])?
    {
        return Err(error("Completed project requires actual full financing"));
    }
    let refunded = p
        .get("refunded_minor")
        .map(integer)
        .transpose()?
        .unwrap_or_default();
    let mut refunds = HashMap::<(ExternalId, ExternalId), BigInt>::new();
    for r in p["investor_refunds"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
    {
        let value = integer(&r["amount_minor"])?;
        if value < BigInt::zero() {
            return Err(error("Negative investor refund"));
        }
        *refunds
            .entry((actor(&r["investor_id"])?, actor(&r["owner_id"])?))
            .or_default() += value;
    }
    let net = &total - &refunded;
    if refunded < BigInt::zero()
        || net <= BigInt::zero()
        || refunds.values().cloned().sum::<BigInt>() != refunded
    {
        return Err(error("Invalid net project capital"));
    }
    let mut shares = vec![];
    for row in rows {
        let value = &row.value
            - refunds
                .remove(&(row.investor.clone(), row.owner.clone()))
                .unwrap_or_default();
        if value < BigInt::zero() {
            return Err(error("Investor refund exceeds contribution"));
        }
        if value.is_zero() {
            continue;
        }
        let d = gcd(value.clone(), net.clone());
        shares.push(json!({"investor_id":row.investor,"owner_id":row.owner,"numerator":(&value/&d).to_string(),"denominator":(&net/&d).to_string()}));
    }
    if !refunds.is_empty() {
        return Err(error("Refund references missing investor right"));
    }
    let id = format!(
        "{}:{}",
        if p["kind"] == "factory" {
            "factory"
        } else {
            "asset"
        },
        js_string(&p["id"])
    );
    let owner = format!("enterprise:{}", js_string(&p["id"]));
    Ok(
        json!({"asset":{"id":id,"enterprise_id":owner,"owner_id":owner,"province_id":p["province_id"],"region_id":p["region_id"],"method_id":p["method_id"],"kind":p["kind"],"status":"closed","level":1,"shares":shares,"provenance":{"project_id":p["id"],"source":p["construction"]["source"]}},"shares":shares,"refundClaims":[]}),
    )
}
pub(crate) fn cancellation_shares(w: &dyn WorldRead, p: &Value) -> OpResult<Vec<Value>> {
    let (total, rows) = rights(w, p)?;
    if p.get("funded_minor")
        .map(integer)
        .transpose()?
        .unwrap_or_default()
        != total
    {
        return Err(error(
            "Project financing differs from actual investor contributions",
        ));
    }
    Ok(rows.into_iter().map(|r|{
        let d=gcd(r.value.clone(),total.clone());
        json!({"investor_id":r.investor,"owner_id":r.owner,"refund_purpose":r.purpose,"numerator":(&r.value/&d).to_string(),"denominator":(&total/&d).to_string()})
    }).collect())
}
#[derive(Debug, Clone, Serialize)]
pub struct RefundPlan {
    pub operations: Vec<EconomicOperation>,
    pub refunds: Vec<Value>,
}
pub fn plan_project_refunds(w: &dyn WorldRead, p: &Value, event: &str) -> OpResult<RefundPlan> {
    let (total, rows) = rights(w, p)?;
    let owner = actor(&p["owner_id"])?;
    let currency = actor(&p["terms"]["currency_id"])?;
    let refundable_total = w.accounts().values()
        .filter(|a| {
            a.owner_id == owner
                && a.currency_id == currency
                && matches!(
                    a.purpose.as_str(),
                    "construction_escrow"
                        | "employer_operating_capital"
                        | "factory_startup_reserve"
                )
                && a.balance.0 > 0
        })
        .map(|a| BigInt::from(a.balance.0))
        .sum::<BigInt>();
    if refundable_total > total {
        return Err(error("Unclassified excess across project accounts"));
    }
    let mut result = RefundPlan {
        operations: vec![],
        refunds: vec![],
    };
    let requests = rows
        .iter()
        .map(|r| {
            Ok(BudgetRequest {
                id: r.investor.clone(),
                amount: Money(
                    r.value
                        .to_i64()
                        .ok_or_else(|| error("Investor capital overflow"))?,
                ),
            })
        })
        .collect::<OpResult<Vec<_>>>()?;
    let refundable = Money(
        refundable_total
            .to_i64()
            .ok_or_else(|| error("Total refundable capital overflow"))?,
    );
    let mut remaining = allocate_budget(&requests, refundable)
        .map_err(|e| error(&e.to_string()))?
        .into_iter()
        .map(|a| BudgetRequest {
            id: a.id,
            amount: a.paid,
        })
        .collect::<Vec<_>>();
    for account in w.accounts().values().filter(|a| {
        a.owner_id == owner
            && a.currency_id == currency
            && matches!(
                a.purpose.as_str(),
                "construction_escrow" | "employer_operating_capital" | "factory_startup_reserve"
            )
            && a.balance.0 > 0
    }) {
        if total <= BigInt::zero() || BigInt::from(account.balance.0) > total {
            return Err(error(
                "Unclassified excess project funds or missing investor rights",
            ));
        }
        let allocations =
            allocate_budget(&remaining, account.balance).map_err(|e| error(&e.to_string()))?;
        let mut credits = vec![];
        for allocation in allocations.into_iter().filter(|a| a.paid.0 > 0) {
            let right = rows
                .iter()
                .find(|r| r.investor == allocation.id)
                .ok_or_else(|| error("Refund investor missing"))?;
            let entitlement = remaining
                .iter_mut()
                .find(|r| r.id == allocation.id)
                .ok_or_else(|| error("Refund entitlement missing"))?;
            entitlement.amount.0 -= allocation.paid.0;
            credits.push(MoneyLeg {
                owner_id: right.owner.clone(),
                currency_id: currency.clone(),
                purpose: right.purpose.clone(),
                amount_minor: allocation.paid,
            });
            result.refunds.push(json!({"investor_id":right.investor,"owner_id":right.owner,"amount_minor":allocation.paid.0.to_string()}));
        }
        let op: EconomicOperation = serde_json::from_value(
            json!({"event_id":format!("{event}:refund:{}",account.purpose),"type":"project_refund","project_id":p["id"],"debits":[{"owner_id":owner,"currency_id":currency,"purpose":account.purpose,"amount_minor":account.balance}],"credits":credits,"income_recognized":false}),
        )?;
        result.operations.push(op);
    }
    Ok(result)
}
