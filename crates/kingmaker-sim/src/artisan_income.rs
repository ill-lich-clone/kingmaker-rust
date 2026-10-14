//! Read-only payout plan; closed income_cycle alone publishes money and registry changes.
use kingmaker_core::world_read::WorldRead;
use crate::{
    budget_funding::error,
    income_attribution::attribute_artisan_operations,
    income_basis::{validate_income_basis, IncomeOpeningBasis},
    income_capital::working_capital_requirements,
    income_recognition::required_income_operations,
    money_math::multiply_money,
    needs::{js_string, js_truthy},
    operation_session::{OpResult, OperationSession},
    producer_income::effective_income_policies,
    profit_rights::integer,
    transport_validation::actor,
};
use kingmaker_core::{
    income::{IncomeRequest, StockContribution},
    operations::EconomicOperation,
    production::ArtisanEnterprise,
    ExternalId, Money,
};
use num_bigint::BigInt;
use num_traits::{Signed, ToPrimitive, Zero};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize)]
pub struct ArtisanIncomePlan {
    pub operations: Vec<EconomicOperation>,
    pub enterprises: Vec<ArtisanEnterprise>,
    pub stock_contributions: Vec<StockContribution>,
    pub diagnostics: Vec<Value>,
}
fn minor(v: &BigInt) -> OpResult<Money> {
    v.to_i64()
        .map(Money)
        .ok_or_else(|| error("Artisan payout overflow"))
}
fn value_or_zero(v: Option<&Value>) -> OpResult<BigInt> {
    v.map(integer)
        .transpose()
        .map(|v| v.unwrap_or_else(BigInt::zero))
}
pub fn plan_artisan_income(
    w: &dyn WorldRead,
    r: &IncomeRequest,
    basis: &IncomeOpeningBasis,
) -> OpResult<ArtisanIncomePlan> {
    validate_income_basis(w, r, basis)?;
    let operations = required_income_operations(w, r)?;
    let attribution = attribute_artisan_operations(w, basis, &operations)?;
    let capital = working_capital_requirements(w, &[])?;
    let policies = effective_income_policies(w, r);
    let session = OperationSession::new(w)?;
    let pops: HashMap<_, _> = w.pops().values().map(|p| (&p.external_id, p)).collect();
    let mut used: HashMap<(ExternalId, ExternalId, String), BigInt> = HashMap::new();
    let mut result = ArtisanIncomePlan {
        operations: vec![],
        enterprises: w.artisans().values().cloned().collect::<Vec<_>>(),
        stock_contributions: attribution.contributions,
        diagnostics: attribution.diagnostics,
    };
    for (e, amount) in result.enterprises.iter_mut().zip(attribution.amounts) {
        let Some(config) = e.base.extra.get("income_policy").filter(|p| {
            p.get("source").is_some_and(js_truthy) && p.get("cash_reserve_minor").is_some()
        }) else {
            continue;
        };
        let Some(fraction) = config["payout_fraction"]
            .as_f64()
            .filter(|f| f.is_finite() && *f >= 0. && *f <= 1.)
        else {
            continue;
        };
        let owner = e
            .base
            .owner_id
            .as_ref()
            .ok_or_else(|| error("Artisan owner missing"))?;
        let purpose = e
            .base
            .payment_purpose
            .as_deref()
            .unwrap_or("operating_capital");
        let currencies: Vec<_> = w.accounts().values()
            .filter(|a| &a.owner_id == owner && a.purpose == purpose)
            .map(|a| &a.currency_id)
            .collect();
        if currencies.len() > 1 {
            return Err(error("Artisan accounting currency ambiguous"));
        }
        let currency = currencies.first().map(|v| (*v).clone()).or_else(|| {
            e.working_capital
                .as_ref()?
                .get("currency_id")
                .and_then(|v| actor(v).ok())
        });
        let Some(currency) = currency else {
            continue;
        };
        let authored = integer(&config["cash_reserve_minor"])?;
        if authored.is_negative() {
            return Err(error("Invalid artisan capital reserve"));
        }
        let carry = value_or_zero(e.base.extra.get("loss_carry_minor"))?;
        let prior = value_or_zero(e.base.extra.get("undistributed_profit_minor"))?;
        if carry.is_negative() || prior.is_negative() {
            return Err(error("Negative artisan loss or retained profit"));
        }
        let net = (!amount.unresolved).then(|| &amount.revenue - &amount.cost);
        if !amount.unresolved {
            if let Some(c) = e.working_capital.as_mut() {
                let principal = value_or_zero(c.get("personal_funded_minor"))?;
                if principal.is_negative() {
                    return Err(error("Negative artisan personal principal"));
                }
                c["personal_funded_minor"] =
                    json!((principal - &amount.spent).max(BigInt::zero()).to_string());
            }
        }
        let reserve = if e.working_capital.is_some() {
            capital
                .groups
                .iter()
                .find(|g| &g.owner_id == owner && g.currency_id == currency)
                .map(|g| BigInt::from(g.minimum_minor.0))
                .unwrap_or_else(BigInt::zero)
                .max(authored)
        } else {
            authored
        };
        let key = (owner.clone(), currency.clone(), purpose.to_owned());
        let already = used.get(&key).cloned().unwrap_or_else(BigInt::zero);
        let cash = BigInt::from(session.cash(owner, &currency, purpose).0) - already;
        let basis = net
            .as_ref()
            .map(|n| (n - &carry).max(BigInt::zero()))
            .unwrap_or_else(BigInt::zero);
        let wanted = BigInt::from(multiply_money(minor(&basis)?, fraction)?.0);
        let property = e.base.assignments.is_empty()
            && e.working_capital.is_some()
            && pops.contains_key(owner);
        let recipients: Vec<_> = if property {
            vec![(owner.clone(), BigInt::from(1))]
        } else {
            e.base
                .assignments
                .iter()
                .map(|a| {
                    let weight = (a.workers_count * a.time_fraction * 1_000_000. + 0.5).floor();
                    if !weight.is_finite() || weight < 0. || weight > 9_007_199_254_740_991. {
                        return Err(error("Invalid artisan income FTE"));
                    }
                    Ok((a.pop_id.clone(), BigInt::from(weight as u64)))
                })
                .collect::<OpResult<_>>()?
        };
        let weight = recipients.iter().map(|r| &r.1).sum::<BigInt>();
        let budget = if weight.is_zero() {
            if wanted > BigInt::zero() {
                result.diagnostics.push(json!({"id":e.base.id,"reason":"artisan_income_no_eligible_recipient","distribution_minor":"0"}));
            }
            BigInt::zero()
        } else {
            wanted.min((cash - reserve).max(BigInt::zero()))
        };
        let newcarry = net
            .as_ref()
            .map(|n| (&carry - n).max(BigInt::zero()))
            .unwrap_or_else(|| carry.clone());
        let retained = net
            .as_ref()
            .map(|n| (&prior + n - &budget).max(BigInt::zero()))
            .unwrap_or_else(|| prior.clone());
        e.base
            .extra
            .insert("loss_carry_minor".into(), json!(newcarry.to_string()));
        e.base.extra.insert(
            "undistributed_profit_minor".into(),
            json!(retained.to_string()),
        );
        e.base.extra.insert("income_history".into(),json!({"period":r.period,"currency_id":currency,"revenue_minor":amount.revenue.to_string(),"recognized_input_cost_minor":if amount.unresolved {None} else {Some(amount.cost.to_string())},"opening_stock_revenue_minor":amount.opening_revenue.to_string(),"net_income_minor":net.map(|n|n.to_string()),"taxable_distribution_minor":budget.to_string(),"cost_basis":"authored_current_quote_on_actual_consumption"}));
        if weight.is_zero() || budget.is_zero() {
            continue;
        }
        let mut paid = BigInt::zero();
        for (i, (id, share)) in recipients.iter().enumerate() {
            let gross = if i + 1 == recipients.len() {
                &budget - &paid
            } else {
                &budget * share / &weight
            };
            paid += &gross;
            if gross.is_zero() {
                continue;
            }
            let pop = pops
                .get(id)
                .ok_or_else(|| error("Artisan income POP missing"))?;
            if (!property && pop.pop_type_id.as_str() != Some("artisans"))
                || pop.currency_id.as_ref() != Some(&currency)
            {
                return Err(error("Artisan income beneficiary rights unresolved"));
            }
            let rate = pop
                .state_id
                .as_ref()
                .and_then(|s| policies.taxes.get(&js_string(&s.json())))
                .copied()
                .unwrap_or(0.);
            if !rate.is_finite() || !(0.0..=1.0).contains(&rate) {
                return Err(error("Invalid enterprise income tax"));
            }
            let gross = minor(&gross)?;
            let tax = multiply_money(gross, rate)?;
            let mut credits = vec![
                json!({"owner_id":id,"currency_id":currency,"purpose":"personal_savings","amount_minor":Money(gross.0-tax.0)}),
            ];
            if tax.0 > 0 {
                credits.push(json!({"owner_id":format!("treasury:{}",js_string(&pop.state_id.as_ref().unwrap().json())),"currency_id":currency,"purpose":"operating_capital","amount_minor":tax}));
            }
            let mut op = json!({"event_id":format!("artisan-income:{}:{}:{}",r.period,js_string(&e.base.id.json()),js_string(&id.json())),"type":"enterprise_income","subject_id":e.base.id,"pop_id":id,"province_id":e.base.province_id.0,"currency_id":currency,"paid_gross_minor":gross,"tax_minor":tax,"income_recognized":true,"debits":[{"owner_id":owner,"currency_id":currency,"purpose":purpose,"amount_minor":gross}],"credits":credits});
            if property {
                op["income_kind"] = json!("property");
            }
            result.operations.push(serde_json::from_value(op)?);
        }
        *used.entry(key).or_default() += paid;
    }
    Ok(result)
}
