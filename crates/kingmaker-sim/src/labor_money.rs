use kingmaker_core::world_read::WorldRead;
use crate::{
    budget_funding::{error, payroll_purpose, wage_multiplier},
    labor_offers::JobOffer,
    labor_roster::LaborRoster,
    market_operations::invoice,
    money_math::{multiply_money, rate_pay},
    needs::resolve_need_quantities,
    operation_session::{external_text, OpResult, OperationSession},
};
use kingmaker_core::{market::NeedProfileRef, production::*, *};
use num_bigint::BigInt;
use num_traits::ToPrimitive;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
pub struct BasicBasketPricer<'a> {
    world: &'a dyn WorldRead,
    refs: HashMap<&'a ExternalId, &'a NeedProfileRef>,
}
impl<'a> BasicBasketPricer<'a> {
    pub fn new(world: &'a dyn WorldRead) -> OpResult<Self> {
        let needs = world.needs()
            .as_ref()
            .ok_or_else(|| error("Basic need profiles unresolved"))?;
        let refs = needs
            .profile_refs
            .iter()
            .map(|r| (&r.pop_id, r))
            .collect::<HashMap<_, _>>();
        if refs.len() != needs.profile_refs.len() {
            return Err(error("Duplicate basic need references"));
        }
        Ok(Self { world, refs })
    }
    pub fn price(&self, pop: &PopRecord) -> OpResult<Money> {
        let currency = pop
            .currency_id
            .as_ref()
            .ok_or_else(|| error("Own basket currency unresolved"))?;
        let mut reference = (*self
            .refs
            .get(&pop.external_id)
            .ok_or_else(|| error("Own basic need profile unresolved"))?)
        .clone();
        reference.extra.insert("size".into(), json!(pop.size));
        reference
            .extra
            .insert("workforce".into(), json!(pop.workforce));
        let needs = self.world.needs().as_ref().unwrap();
        let rows = resolve_need_quantities(needs, &reference)?;
        let calibration = self
            .world
            .price_calibration()
            .as_ref()
            .ok_or_else(|| error("Own basket prices unresolved"))?;
        let prices = calibration
            .prices_by_currency
            .get(&external_text(currency))
            .ok_or_else(|| error("Own basket currency quotes unresolved"))?;
        let mut total = BigInt::from(0);
        for id in ["food", "clothing"] {
            let need = rows
                .iter()
                .find(|n| n.need_id.as_str() == Some(id))
                .ok_or_else(|| error("Own basic quantity unresolved"))?;
            let quantity = need
                .quantity
                .filter(|q| q.is_finite() && *q >= 0.)
                .ok_or_else(|| error("Own basic quantity unresolved"))?;
            let alternatives = if id == "food" {
                need.extra
                    .get("alternatives")
                    .and_then(Value::as_array)
                    .cloned()
                    .ok_or_else(|| error("Own food alternatives unresolved"))?
            } else {
                vec![json!({"goods_id":"clothing","share":1})]
            };
            if alternatives.is_empty() {
                return Err(error("Own food price unresolved"));
            }
            let mut price = 0.;
            for a in alternatives {
                let goods = ExternalId::from_json(a["goods_id"].clone())
                    .map_err(crate::operation_session::OperationError)?;
                let p = prices
                    .get(&external_text(&goods))
                    .and_then(Value::as_f64)
                    .filter(|p| p.is_finite() && *p > 0.)
                    .ok_or_else(|| error("Own basic price unresolved"))?;
                let share = a
                    .get("share")
                    .and_then(Value::as_f64)
                    .filter(|n| n.is_finite() && *n >= 0.)
                    .ok_or_else(|| error("Own food share unresolved"))?;
                price += p * share;
            }
            total += invoice(quantity, price)?;
        }
        Ok(Money(
            total
                .to_i64()
                .ok_or_else(|| error("Own basket price overflow"))?,
        ))
    }
}
pub fn basic_basket_price(w: &dyn WorldRead, pop: &PopRecord) -> OpResult<Money> {
    BasicBasketPricer::new(w)?.price(pop)
}
pub fn own_labor_contract(
    w: &dyn WorldRead,
    p: &PopRecord,
    o: &JobOffer,
    workers: u64,
    policies: &ProductionPolicies,
) -> OpResult<LaborContract> {
    let price = basic_basket_price(w, p)?;
    contract_from_basket(w, p, o, workers, policies, price)
}
pub(crate) fn contract_from_basket(
    w: &dyn WorldRead,
    p: &PopRecord,
    o: &JobOffer,
    workers: u64,
    policies: &ProductionPolicies,
    basket: Money,
) -> OpResult<LaborContract> {
    if !p.workforce.is_finite()
        || p.workforce <= 0.
        || p.workforce.fract() != 0.
        || p.workforce > 9_007_199_254_740_991.
        || workers as f64 > p.workforce
    {
        return Err(error("Invalid own wage workforce"));
    }
    let workforce = BigInt::from(p.workforce as u64);
    let currency = p
        .currency_id
        .as_ref()
        .ok_or_else(|| error("Own wage currency unresolved"))?;
    let typed = crate::labor_identity::component;
    if o.kind == "rgo" {
        let rgo = w.resource_operations().values()
            .find(|r| r.base.id == o.entity_id)
            .ok_or_else(|| error("Own RGO contract source unresolved"))?;
        let mult: i64 = if [
            "grain",
            "livestock",
            "fish",
            "produce",
            "fibers",
            "silk",
            "sweeteners",
        ]
        .contains(&external_text(&rgo.resource_id).as_str())
        {
            100
        } else {
            110
        };
        let n = BigInt::from(basket.0) * 30 * mult;
        let d = BigInt::from(7) * &workforce * 100;
        let monthly: BigInt = (n + &d / 2) / &d;
        let rate = MoneyRate {
            numerator: (monthly * BigInt::from(7)).to_string(),
            denominator: "30".into(),
        };
        let gross = rate_pay(&rate, workers as f64, 1.)?;
        let tax_rate = p
            .state_id
            .as_ref()
            .and_then(|id| policies.taxes.get(&external_text(id)))
            .copied()
            .unwrap_or(0.);
        let tax = multiply_money(gross, tax_rate)?;
        return Ok(serde_json::from_value(
            json!({"contract_id":format!("{}:own-contract:{}",typed(&o.entity_id),typed(&p.external_id)),"job_id":format!("{}:source-job",typed(&o.entity_id)),"capacity_pool_id":format!("{}:source-job",typed(&o.entity_id)),"building_id":o.entity_id,"method_id":rgo.base.method_id,"employer_entity_id":o.owner_id,"province_id":p.province_id.0,"pop_type_id":p.pop_type_id,"currency_id":currency,"organization":if mult==100{"primary"}else{"extraction"},"role_id":p.pop_type_id,"requested_fte":workers,"reserved_fte":workers,"weekly_wage_rate":rate,"wage_weekly_per_worker":rate.numerator.parse::<f64>().map_err(|_|error("Own wage display overflow"))?/30.,"wage_multiplier":mult as f64/100.,"wage_unit":"currency_minor","period_days":7,"status":"active","wage_status":"calculated","funding_status":"not_reserved_by_correction","expectedPayroll":{"gross_minor":gross,"tax_minor":tax,"net_minor":Money(gross.0-tax.0),"reserved_minor":0,"actual_t0_payment":0,"tax_rate":tax_rate,"status":"calculated_not_paid"},"provenance":{"kind":"own_pop_basket_rate","basis":"own monthly basic basket / own workforce * existing primary or extraction multiplier; monthly minor rounded before exact 7/30","pop_id":p.external_id,"weekly_basket_minor":basket.0.to_string()}}),
        )?);
    }
    let role = o
        .job
        .as_ref()
        .and_then(|j| j.get("role_id"))
        .cloned()
        .unwrap_or_else(|| p.pop_type_id.json());
    let mult: i64 = match role.as_str() {
        Some("officer") => 135,
        Some("soldier") => 120,
        _ => {
            if o.kind == "service" {
                105
            } else {
                115
            }
        }
    };
    let rate = MoneyRate {
        numerator: (BigInt::from(basket.0) * mult).to_string(),
        denominator: (workforce * BigInt::from(100)).to_string(),
    };
    let mut value = json!({"contract_id":format!("hire:{}:contract:{}",typed(&o.id),typed(&p.external_id)),"job_id":o.job.as_ref().and_then(|j|j.get("job_id")).cloned().unwrap_or_else(||json!(format!("{}:source-job",typed(&o.id)))),"capacity_pool_id":o.job.as_ref().and_then(|j|j.get("capacity_pool_id")).cloned().unwrap_or_else(||o.id.json()),"building_id":o.entity_id,"employer_entity_id":o.owner_id,"pop_type_id":p.pop_type_id,"role_id":role,"currency_id":currency,"payment_purpose":o.purpose,"weekly_wage_rate":rate,"provenance":{"kind":"own_pop_basket_rate","pop_id":p.external_id,"multiplier":mult as f64/100.}});
    if o.kind == "service" {
        value["budget_state_id"] = o.owner_id.json();
        value["budget_sector"] = json!(o.sector);
    }
    Ok(serde_json::from_value(value)?)
}
#[derive(Debug, Clone)]
pub struct LaborCash {
    pub balances: HashMap<AccountAddress, Money>,
    states: HashSet<ExternalId>,
}
impl LaborCash {
    pub(crate) fn from_accounts(w: &dyn WorldRead) -> Self {
        Self {
            balances: w.accounts().values()
                .map(|a| (a.address(), a.balance))
                .collect(),
            states: HashSet::new(),
        }
    }
    pub fn amount(&self, owner: &ExternalId, currency: &ExternalId, purpose: &str) -> Money {
        self.balances
            .get(&AccountAddress {
                owner_id: owner.clone(),
                currency_id: currency.clone(),
                purpose: purpose.into(),
            })
            .copied()
            .unwrap_or(Money::ZERO)
    }
    pub fn source_for(
        &self,
        owner: &ExternalId,
        currency: &ExternalId,
        purpose: &str,
    ) -> AccountAddress {
        AccountAddress {
            owner_id: if self.states.contains(owner) && purpose.starts_with("state_service:") {
                format!("treasury:{}", external_text(owner)).into()
            } else {
                owner.clone()
            },
            currency_id: currency.clone(),
            purpose: "operating_capital".into(),
        }
    }
}
pub fn available_labor_cash(
    w: &dyn WorldRead,
    policies: &ProductionPolicies,
    roster: &LaborRoster,
) -> OpResult<LaborCash> {
    available_labor_cash_with_external(w, policies, roster, &[])
}
pub(crate) fn available_labor_cash_with_external(
    w: &dyn WorldRead,
    policies: &ProductionPolicies,
    roster: &LaborRoster,
    external: &[(AccountAddress, Money)],
) -> OpResult<LaborCash> {
    let states = w.profiles().values()
        .map(|p| crate::budget_funding::optional_id(p.get("state_id")))
        .collect::<OpResult<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect();
    let mut result = LaborCash {
        balances: HashMap::new(),
        states,
    };
    let mut raw = HashMap::<AccountAddress, i128>::new();
    for a in w.accounts().values() {
        *raw.entry(a.address()).or_default() += i128::from(a.balance.0);
    }
    let mut s = OperationSession::new(w)?;
    s.policies.taxes.extend(policies.taxes.clone());
    s.policies.wages.extend(policies.wages.clone());
    s.policies.extra.extend(policies.extra.clone());
    for e in roster.by_employment.values() {
        let Some(c) = e.contract_id.as_ref().and_then(|id| s.contract(id)) else {
            continue;
        };
        let Some(rate) = &c.weekly_wage_rate else {
            continue;
        };
        let owner = c
            .employer_entity_id
            .as_ref()
            .ok_or_else(|| error("Existing labor employer unresolved"))?;
        let currency = c
            .currency_id
            .as_ref()
            .ok_or_else(|| error("Existing labor currency unresolved"))?;
        let address = AccountAddress {
            owner_id: owner.clone(),
            currency_id: currency.clone(),
            purpose: payroll_purpose(c)?,
        };
        *raw.entry(address).or_default() -=
            i128::from(rate_pay(rate, e.fte, wage_multiplier(&s, c)?)?.0) * 2;
    }
    for (address, amount) in external {
        if amount.0 < 0 {
            return Err(error("Negative external wage reservation"));
        }
        *raw.entry(address.clone()).or_default() -= i128::from(amount.0);
    }
    for d in w.obligations().values() {
        if d.amount_minor.0 > 0 {
            *raw.entry(AccountAddress {
                owner_id: d.payer_id.clone(),
                currency_id: d.currency_id.clone(),
                purpose: d.purpose.clone(),
            })
            .or_default() -= i128::from(d.amount_minor.0);
        }
    }
    let shortages = raw
        .iter()
        .filter(|(_, v)| **v < 0)
        .map(|(k, v)| (k.clone(), *v))
        .collect::<Vec<_>>();
    for (key, value) in shortages {
        let source = result.source_for(&key.owner_id, &key.currency_id, &key.purpose);
        if source != key {
            *raw.entry(source).or_default() += value;
        }
        raw.insert(key, 0);
    }
    for (key, value) in raw {
        result.balances.insert(
            key,
            Money(i64::try_from(value.max(0)).map_err(|_| error("Available labor cash overflow"))?),
        );
    }
    Ok(result)
}
