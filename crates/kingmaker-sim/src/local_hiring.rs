//! Source competitive local allocation. Plans are closed; execution belongs to the labor phase.
use kingmaker_core::world_read::WorldRead;
use crate::{
    budget_funding::{error, wage_multiplier},
    labor_forecast::{HiringDemands, HiringForecast},
    labor_money::{available_labor_cash_with_external, contract_from_basket, BasicBasketPricer},
    labor_offers::{build_job_offers, JobOffer},
    labor_release::{apply_staff_change, LaborRows},
    labor_roster::{build_labor_roster, reconcile_labor_in, LaborRoster},
    money_math::{allocate_budget, rate_pay, BudgetRequest as MoneyRequest},
    operation_session::{external_text, OpResult, OperationSession},
    storage::truthy,
};
use kingmaker_core::{budget::Employment, operations::*, production::*, *};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
pub struct LaborPlan {
    pub hires: Vec<Value>,
    pub blocked: Vec<Value>,
    pub operations: Vec<EconomicOperation>,
    pub(crate) rows: LaborRows,
}
/// Restricts new candidates/offers; existing work and shared resources remain visible.
#[derive(Debug, Default)]
pub struct HiringScope {
    pub candidate_ids: Option<HashSet<ExternalId>>,
    pub offer_ids: Option<HashSet<ExternalId>>,
    pub external_wage_reservations: Vec<(AccountAddress, Money)>,
}
/// Read-only destination forecast; cash, need references and production budgets are indexed once.
pub struct DestinationFunding<'a> {
    world: &'a dyn WorldRead,
    policies: &'a ProductionPolicies,
    session: OperationSession<'a>,
    pricer: BasicBasketPricer<'a>,
    forecast: HiringForecast<'a>,
}
impl<'a> DestinationFunding<'a> {
    pub fn new(
        w: &'a dyn WorldRead,
        policies: &'a ProductionPolicies,
        roster: &LaborRoster,
    ) -> OpResult<Self> {
        let mut session = OperationSession::new(w)?;
        session.policies.wages.extend(policies.wages.clone());
        session.policies.extra.extend(policies.extra.clone());
        Ok(Self {
            world: w,
            policies,
            session,
            pricer: BasicBasketPricer::new(w)?,
            forecast: HiringForecast::new(
                w,
                available_labor_cash_with_external(w, policies, roster, &[])?,
            )?,
        })
    }
    pub fn capacity(&mut self, p: &PopRecord, o: &JobOffer) -> OpResult<u64> {
        let requested = (p.workforce.floor() as u64).min(o.places);
        if requested == 0 {
            return Ok(0);
        }
        let contract = contract_from_basket(
            self.world,
            p,
            o,
            requested,
            self.policies,
            self.pricer.price(p)?,
        )?;
        let mult = wage_multiplier(&self.session, &contract)?;
        if mult <= 0. || contract.currency_id != p.currency_id {
            return Ok(0);
        }
        let currency = p
            .currency_id
            .as_ref()
            .ok_or_else(|| error("Destination own currency unresolved"))?;
        let pool = AccountAddress {
            owner_id: o.owner_id.clone(),
            currency_id: currency.clone(),
            purpose: o.purpose.clone(),
        };
        let source = self
            .forecast
            .cash
            .source_for(&o.owner_id, currency, &o.purpose);
        let q = Quote {
            contract,
            mult,
            pool,
            source,
        };
        let money = funds(&self.forecast, &q)?;
        let count = affordable_count(&q, requested, money)?;
        if o.kind == "factory" {
            let mut destination = p.clone();
            destination.province_id = o.province_id;
            self.forecast
                .limit(&o.entity_id, &destination, count, |d, n| {
                    combined_affordable(d, &q, n, money)
                })
        } else {
            Ok(count)
        }
    }
}
impl LaborPlan {
    pub fn hired_fte(&self) -> f64 {
        self.hires.iter().map(|h| h["fte"].as_f64().unwrap()).sum()
    }
    pub fn contracts(&self) -> &[LaborContract] {
        &self.rows.contracts
    }
    pub fn employment(&self) -> &[Employment] {
        &self.rows.employment
    }
    pub fn pops(&self) -> &[PopRecord] {
        &self.rows.pops
    }
}
#[derive(Clone)]
struct Quote {
    contract: LaborContract,
    mult: f64,
    pool: AccountAddress,
    source: AccountAddress,
}
#[derive(Clone)]
struct Bid {
    id: ExternalId,
    offer: usize,
    pop: usize,
    count: u64,
    quote: Quote,
    amount: Money,
    quota: Money,
}
struct Quotas {
    places: HashMap<ExternalId, u64>,
    counts: HashMap<(ExternalId, ExternalId), u64>,
}
struct LaborIndex {
    provinces: HashMap<ProvinceId, Vec<usize>>,
    old: HashMap<(ExternalId, ExternalId), LaborAssignment>,
}
impl LaborIndex {
    fn new(w: &dyn WorldRead, scope: &HiringScope) -> OpResult<Self> {
        let mut provinces = HashMap::<_, Vec<_>>::new();
        for (i, p) in w.pops().values().enumerate() {
            if scope
                .candidate_ids
                .as_ref()
                .is_none_or(|ids| ids.contains(&p.external_id))
            {
                provinces.entry(p.province_id).or_default().push(i);
            }
        }
        let mut old = HashMap::new();
        for e in w.resource_operations().values()
            .map(|e| &e.base)
            .chain(w.factories().values().map(|e| &e.base))
        {
            for a in &e.assignments {
                if a.contract_id.is_some() {
                    old.entry((e.id.clone(), a.pop_id.clone()))
                        .or_insert_with(|| a.clone());
                }
            }
        }
        for e in w.employment().values() {
            let Some(job) = crate::budget_funding::optional_id(e.extra.get("job_id"))? else {
                continue;
            };
            let id = crate::labor_identity::service_offer_id(&job);
            old.insert(
                (id, e.pop_id.clone()),
                LaborAssignment {
                    employment_id: Some(e.employment_id.clone()),
                    contract_id: Some(e.contract_id.clone()),
                    pop_id: e.pop_id.clone(),
                    workers_count: e
                        .extra
                        .get("workers_count")
                        .and_then(Value::as_f64)
                        .unwrap_or(e.fte),
                    time_fraction: e
                        .extra
                        .get("time_fraction")
                        .and_then(Value::as_f64)
                        .unwrap_or(1.),
                    fte: Some(e.fte),
                    worker_group_id: None,
                    productivity_multiplier: e
                        .extra
                        .get("productivity_multiplier")
                        .and_then(Value::as_f64),
                    productivity_status: None,
                    extra: Default::default(),
                },
            );
        }
        Ok(Self { provinces, old })
    }
    fn population<'a>(
        &'a self,
        w: &'a dyn WorldRead,
        province: ProvinceId,
    ) -> impl Iterator<Item = (usize, &'a PopRecord)> + 'a {
        self.provinces
            .get(&province)
            .into_iter()
            .flatten()
            .map(|i| (*i, w.pops().at(*i).expect("validated row position")))
    }
}
fn english() -> icu_collator::CollatorBorrowed<'static> {
    icu_collator::Collator::try_new(
        "en".parse::<icu_locale_core::Locale>().unwrap().into(),
        Default::default(),
    )
    .unwrap()
}
fn apportion(weights: &[(String, u64)], total: u64, en: bool) -> Vec<u64> {
    let sum: u64 = weights.iter().map(|x| x.1).sum();
    if sum == 0 {
        return vec![0; weights.len()];
    }
    let n = total.min(sum);
    let mut out = vec![0; weights.len()];
    let mut fractions = Vec::new();
    for (i, (id, weight)) in weights.iter().enumerate() {
        let exact = n as f64 * (*weight as f64) / sum as f64;
        out[i] = exact.floor() as u64;
        fractions.push((i, exact.fract(), id));
    }
    let mut left = n - out.iter().sum::<u64>();
    let coll = if en {
        english()
    } else {
        crate::ordering::collator()
    };
    fractions.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| coll.compare(a.2, b.2)));
    for (i, _, _) in fractions {
        if left == 0 {
            break;
        }
        out[i] += 1;
        left -= 1;
    }
    out
}
fn expense(q: &Quote, n: u64) -> OpResult<Money> {
    let wage = rate_pay(
        q.contract
            .weekly_wage_rate
            .as_ref()
            .ok_or_else(|| error("Own wage rate unresolved"))?,
        n as f64,
        q.mult,
    )?;
    Ok(Money(
        wage.0
            .checked_mul(2)
            .ok_or_else(|| error("Hiring wage overflow"))?,
    ))
}
fn affordable_count(q: &Quote, requested: u64, money: Money) -> OpResult<u64> {
    let (mut lo, mut hi) = (0, requested);
    while lo < hi {
        let mid = lo + (hi - lo + 1) / 2;
        if expense(q, mid)?.0 <= money.0 {
            lo = mid
        } else {
            hi = mid - 1
        }
    }
    Ok(lo)
}
fn funds(f: &HiringForecast, q: &Quote) -> OpResult<Money> {
    let a = f
        .cash
        .balances
        .get(&q.pool)
        .copied()
        .unwrap_or(Money::ZERO)
        .0;
    let b = if q.source != q.pool {
        f.cash
            .balances
            .get(&q.source)
            .copied()
            .unwrap_or(Money::ZERO)
            .0
    } else {
        0
    };
    Ok(Money(
        a.checked_add(b)
            .ok_or_else(|| error("Hiring money overflow"))?,
    ))
}
fn eligible(p: &PopRecord, o: &JobOffer) -> bool {
    p.province_id == o.province_id
        && p.extra.get("mobility_status").and_then(Value::as_str) != Some("in_transit")
        && o.staffing.contains(&p.pop_type_id)
}
fn old_assignment(index: &LaborIndex, o: &JobOffer, p: &PopRecord) -> Option<LaborAssignment> {
    index
        .old
        .get(&(o.id.clone(), p.external_id.clone()))
        .cloned()
}
fn quotes(
    w: &dyn WorldRead,
    index: &LaborIndex,
    offers: &[JobOffer],
    pricer: &BasicBasketPricer,
    cash: &HiringForecast,
    policies: &ProductionPolicies,
) -> OpResult<HashMap<(ExternalId, ExternalId), OpResult<Quote>>> {
    let contracts: HashMap<_, _> = w.contracts().values()
        .map(|c| (&c.contract_id, c))
        .collect();
    let mut s = OperationSession::new(w)?;
    s.policies.taxes.extend(policies.taxes.clone());
    s.policies.wages.extend(policies.wages.clone());
    s.policies.extra.extend(policies.extra.clone());
    let mut out = HashMap::new();
    for o in offers {
        for (_, p) in index
            .population(w, o.province_id)
            .filter(|(_, p)| eligible(p, o))
        {
            let q = (|| {
                let old = old_assignment(index, o, p);
                let new_id = if o.kind == "rgo" {
                    format!(
                        "{}:own-contract:{}",
                        typed(&o.entity_id),
                        typed(&p.external_id)
                    )
                } else {
                    format!("hire:{}:contract:{}", typed(&o.id), typed(&p.external_id))
                };
                let contract = old
                    .as_ref()
                    .and_then(|a| a.contract_id.as_ref())
                    .and_then(|id| contracts.get(id))
                    .copied()
                    .or_else(|| contracts.get(&ExternalId::from(new_id)).copied())
                    .cloned()
                    .map(Ok)
                    .unwrap_or_else(|| {
                        contract_from_basket(w, p, o, 1, policies, pricer.price(p)?)
                    })?;
                if contract.currency_id != p.currency_id {
                    return Err(error("wage_currency_mismatch"));
                }
                let mult = wage_multiplier(&s, &contract)?;
                if !(mult > 0.) {
                    return Err(error("funding_disabled"));
                }
                let currency = p
                    .currency_id
                    .as_ref()
                    .ok_or_else(|| error("Own wage currency unresolved"))?;
                let pool = AccountAddress {
                    owner_id: o.owner_id.clone(),
                    currency_id: currency.clone(),
                    purpose: o.purpose.clone(),
                };
                let source = cash.cash.source_for(&o.owner_id, currency, &o.purpose);
                Ok(Quote {
                    contract,
                    mult,
                    pool,
                    source,
                })
            })();
            out.insert((o.id.clone(), p.external_id.clone()), q);
        }
    }
    Ok(out)
}
fn quotas(
    w: &dyn WorldRead,
    index: &LaborIndex,
    offers: &[JobOffer],
    r: &LaborRoster,
    forecast: &mut HiringForecast,
    quotes: &HashMap<(ExternalId, ExternalId), OpResult<Quote>>,
) -> OpResult<Quotas> {
    let mut groups = Vec::<Vec<usize>>::new();
    let mut indices = HashMap::new();
    for (i, o) in offers.iter().enumerate() {
        let mut staffing = o.staffing.clone();
        staffing
            .sort_by(|a, b| crate::ordering::utf16_compare(&external_text(a), &external_text(b)));
        let k = (o.province_id, staffing);
        let g = *indices.entry(k).or_insert_with(|| {
            groups.push(vec![]);
            groups.len() - 1
        });
        groups[g].push(i);
    }
    let mut places = HashMap::new();
    for same in groups {
        let first = &offers[same[0]];
        let populations: Vec<_> = index
            .population(w, first.province_id)
            .filter(|(_, p)| eligible(p, first))
            .map(|(_, p)| p)
            .collect();
        let free = populations
            .iter()
            .map(|p| r.by_pop[&p.external_id].free.floor() as u64)
            .sum();
        let mut weights = vec![];
        for index in &same {
            let o = &offers[*index];
            let mut feasible = 0;
            for p in &populations {
                let available = r.by_pop[&p.external_id].free.floor() as u64;
                if available == 0 {
                    continue;
                }
                let Some(Ok(q)) = quotes.get(&(o.id.clone(), p.external_id.clone())) else {
                    continue;
                };
                let affordable = affordable_count(q, available.min(o.places), funds(forecast, q)?)?;
                feasible += if o.kind == "factory" {
                    forecast.limit(&o.entity_id, p, affordable, |_, _| Ok(true))?
                } else {
                    affordable
                };
            }
            weights.push((external_text(&o.id), o.places.min(feasible)));
        }
        for (index, n) in same.iter().zip(apportion(&weights, free, false)) {
            places.insert(offers[*index].id.clone(), n);
        }
    }
    let mut pending: HashMap<_, _> = r
        .by_pop
        .iter()
        .map(|(id, p)| (id.clone(), p.free.floor() as u64))
        .collect();
    let mut bids = Vec::<Bid>::new();
    for (i, o) in offers.iter().enumerate() {
        let slots = *places.get(&o.id).unwrap_or(&0);
        if slots == 0 {
            continue;
        }
        let candidates: Vec<_> = index
            .population(w, o.province_id)
            .filter(|(_, p)| eligible(p, o) && pending[&p.external_id] > 0)
            .collect();
        let weights: Vec<_> = candidates
            .iter()
            .map(|(_, p)| (external_text(&p.external_id), pending[&p.external_id]))
            .collect();
        for ((pindex, p), count) in candidates.iter().zip(apportion(&weights, slots, false)) {
            if count == 0 {
                continue;
            }
            let Some(Ok(q)) = quotes.get(&(o.id.clone(), p.external_id.clone())) else {
                continue;
            };
            let id = ExternalId::from(json!([o.id, p.external_id]).to_string());
            bids.push(Bid {
                id,
                offer: i,
                pop: *pindex,
                count,
                quote: q.clone(),
                amount: expense(q, count)?,
                quota: Money::ZERO,
            });
            *pending.get_mut(&p.external_id).unwrap() -= count;
        }
    }
    for source in [false, true] {
        let mut groups = Vec::<(AccountAddress, Vec<usize>)>::new();
        let mut idx = HashMap::new();
        for (i, b) in bids.iter().enumerate() {
            if source && b.quote.source == b.quote.pool {
                continue;
            }
            let k = if source {
                &b.quote.source
            } else {
                &b.quote.pool
            };
            let g = *idx.entry(k.clone()).or_insert_with(|| {
                groups.push((k.clone(), vec![]));
                groups.len() - 1
            });
            groups[g].1.push(i);
        }
        for (k, rows) in groups {
            let requests: Vec<_> = rows
                .iter()
                .map(|i| MoneyRequest {
                    id: bids[*i].id.clone(),
                    amount: Money(bids[*i].amount.0 - if source { bids[*i].quota.0 } else { 0 }),
                })
                .collect();
            let paid = allocate_budget(
                &requests,
                forecast
                    .cash
                    .balances
                    .get(&k)
                    .copied()
                    .unwrap_or(Money::ZERO),
            )?;
            let by_id: HashMap<_, _> = paid.into_iter().map(|p| (p.id, p.paid)).collect();
            for i in rows.iter() {
                bids[*i].quota.0 = bids[*i]
                    .quota
                    .0
                    .checked_add(
                        by_id
                            .get(&bids[*i].id)
                            .ok_or_else(|| error("Hiring money allocation ID missing"))?
                            .0,
                    )
                    .ok_or_else(|| error("Hiring quota overflow"))?;
            }
        }
    }
    let mut counts = HashMap::new();
    for b in bids {
        counts.insert(
            (
                offers[b.offer].id.clone(),
                w.pops().at(b.pop).expect("validated row position").external_id.clone(),
            ),
            affordable_count(&b.quote, b.count, b.quota)?,
        );
    }
    Ok(Quotas { places, counts })
}
fn portions(
    w: &dyn WorldRead,
    index: &LaborIndex,
    o: &JobOffer,
    r: &LaborRoster,
    q: &Quotas,
    forecast: &mut HiringForecast,
    quotes: &HashMap<(ExternalId, ExternalId), OpResult<Quote>>,
) -> OpResult<Vec<(usize, u64)>> {
    let mut candidates: Vec<_> = index
        .population(w, o.province_id)
        .filter(|(_, p)| eligible(p, o) && r.by_pop[&p.external_id].free >= 1.)
        .collect();
    let en = english();
    candidates.sort_by(|(_, a), (_, b)| {
        if o.kind == "factory" {
            (b.pop_type_id.as_str() == Some("craftsmen"))
                .cmp(&(a.pop_type_id.as_str() == Some("craftsmen")))
        } else {
            std::cmp::Ordering::Equal
        }
        .then_with(|| {
            en.compare(
                &external_text(&a.external_id),
                &external_text(&b.external_id),
            )
        })
    });
    let weights: Vec<_> = candidates
        .iter()
        .map(|(_, p)| {
            (
                external_text(&p.external_id),
                r.by_pop[&p.external_id].free.floor() as u64,
            )
        })
        .collect();
    let rows: Vec<_> = candidates
        .iter()
        .zip(apportion(
            &weights,
            *q.places.get(&o.id).unwrap_or(&0),
            true,
        ))
        .map(|((i, _), n)| (*i, n))
        .collect();
    if o.kind != "factory" {
        return Ok(rows);
    }
    let mut groups = Vec::<Vec<(usize, u64)>>::new();
    let mut types = HashMap::new();
    for (i, n) in rows {
        let p = w.pops().at(i).expect("validated row position");
        let count = forecast.limit(
            &o.entity_id,
            p,
            n.min(
                *q.counts
                    .get(&(o.id.clone(), p.external_id.clone()))
                    .unwrap_or(&0),
            ),
            |_, _| Ok(true),
        )?;
        let g = *types.entry(p.pop_type_id.clone()).or_insert_with(|| {
            groups.push(vec![]);
            groups.len() - 1
        });
        groups[g].push((i, count));
    }
    let mut result = vec![];
    for group in groups {
        let weights: Vec<_> = group
            .iter()
            .map(|(i, n)| (external_text(&w.pops().at(*i).expect("validated row position").external_id), *n))
            .collect();
        let (mut lo, mut hi) = (0, group.iter().map(|(_, n)| *n).sum::<u64>());
        while lo < hi {
            let mid = lo + (hi - lo + 1) / 2;
            let rows: Vec<_> = group
                .iter()
                .zip(apportion(&weights, mid, true))
                .map(|((i, _), n)| (*i, n))
                .collect();
            let ok = forecast.preview(|f| {
                for (i, n) in &rows {
                    if *n == 0 {
                        continue;
                    }
                    let p = w.pops().at(*i).expect("validated row position");
                    if f.limit(&o.entity_id, p, *n, |_, _| Ok(true))? != *n {
                        return Ok(false);
                    }
                    f.accept(&o.entity_id, p, *n)?;
                    let Some(Ok(quote)) = quotes.get(&(o.id.clone(), p.external_id.clone())) else {
                        return Ok(false);
                    };
                    if !f.reserve_wages(&quote.pool, &quote.source, expense(quote, *n)?)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            })?;
            if ok {
                lo = mid
            } else {
                hi = mid - 1
            }
        }
        result.extend(
            group
                .iter()
                .zip(apportion(&weights, lo, true))
                .map(|((i, _), n)| (*i, n)),
        );
    }
    Ok(result)
}
pub fn plan_local_hiring(
    w: &dyn WorldRead,
    period: &str,
    policies: &ProductionPolicies,
) -> OpResult<LaborPlan> {
    plan_local_hiring_scoped(w, period, policies, &HiringScope::default())
}
pub fn plan_local_hiring_scoped(w:&dyn WorldRead,period:&str,policies:&ProductionPolicies,scope:&HiringScope)->OpResult<LaborPlan> {
    let mut projected=crate::world_transaction::WorldTransaction::new(w);
    let result=plan_local_hiring_in(&mut projected,period,policies,scope)?;
    Ok(LaborPlan {hires:result.hires,blocked:result.blocked,operations:result.operations,rows:LaborRows::capture(&projected)})
}
pub(crate) struct HiringResult {pub hires:Vec<Value>,pub blocked:Vec<Value>,pub operations:Vec<EconomicOperation>}
pub(crate) fn plan_local_hiring_in(w:&mut crate::world_transaction::WorldTransaction<'_>,period:&str,policies:&ProductionPolicies,scope:&HiringScope)->OpResult<HiringResult> {
    if period.is_empty() {
        return Err(error("Hiring period required"));
    }

    let mut hires = vec![];
    let mut operations = vec![];
    let mut pass = 0;
    let mut blocked;
    loop {
        let pass_period = if pass == 0 {
            period.to_owned()
        } else {
            format!("{period}:allocation:{pass}")
        };
        let result = local_pass(w, &pass_period, policies, scope)?;
        blocked = result.1;
        if result.0.is_empty() {
            break;
        }
        hires.extend(result.0);
        operations.extend(result.2);
        pass += 1;
    }
    Ok(HiringResult {
        hires,
        blocked,
        operations,
    })
}
fn local_pass(
    w: &mut crate::world_transaction::WorldTransaction<'_>,
    period: &str,
    policies: &ProductionPolicies,
    scope: &HiringScope,
) -> OpResult<(Vec<Value>, Vec<Value>, Vec<EconomicOperation>)> {
    let mut roster = build_labor_roster(w)?;
    let index = LaborIndex::new(w, scope)?;
    let result = build_job_offers(w, &roster)?;
    let mut blocked = result.blocked;
    let mut offers = result.offers;
    offers.retain(|o| {
        scope
            .offer_ids
            .as_ref()
            .is_none_or(|ids| ids.contains(&o.id))
    });
    let cash = available_labor_cash_with_external(
        w,
        policies,
        &roster,
        &scope.external_wage_reservations,
    )?;
    let pricer = BasicBasketPricer::new(w)?;
    let mut forecast = HiringForecast::new(w, cash)?;
    let quotes = quotes(w, &index, &offers, &pricer, &forecast, policies)?;
    let quotas = quotas(w, &index, &offers, &roster, &mut forecast, &quotes)?;
    let en = english();
    offers.sort_by(|a, b| en.compare(&external_text(&a.id), &external_text(&b.id)));
    let mut rows=crate::world_transaction::WorldTransaction::new(w);
    let mut contract_ids:HashSet<_>=w.contracts().iter().map(|c|c.contract_id.clone()).collect();
    let mut job_ids:HashSet<_>=w.jobs().iter().filter_map(|j|j.get("job_id")).map(Value::to_string).collect();
    let mut hires = vec![];
    let mut operations = vec![];
    for o in &offers {
        crate::run_control::work("hiring_offers")?;
        for (i, requested) in portions(w, &index, o, &roster, &quotas, &mut forecast, &quotes)? {
            if requested == 0 {
                continue;
            }
            let p = w.pops().at(i).expect("validated POP position");
            let quote = match quotes.get(&(o.id.clone(), p.external_id.clone())) {
                Some(Ok(q)) => q,
                Some(Err(e)) => {
                    blocked.push(json!({"entity_id":o.id,"pop_id":p.external_id,"reason":"own_wage_unresolved","detail":e.0}));
                    continue;
                }
                None => continue,
            };
            let money = funds(&forecast, quote)?;
            let lo = affordable_count(
                quote,
                requested.min(
                    *quotas
                        .counts
                        .get(&(o.id.clone(), p.external_id.clone()))
                        .unwrap_or(&0),
                ),
                money,
            )?;
            let count = if o.kind == "factory" {
                forecast.limit(&o.entity_id, p, lo, |d, n| {
                    combined_affordable(d, quote, n, money)
                })?
            } else {
                lo
            };
            if count == 0 {
                blocked.push(json!({"entity_id":o.id,"pop_id":p.external_id,"reason":if lo>0{"production_inputs_storage_or_support_unavailable"}else{"wage_budget_unavailable"}}));
                continue;
            }
            let old = old_assignment(&index, o, p);
            let coefficient = old
                .as_ref()
                .and_then(|a| a.productivity_multiplier)
                .or_else(|| {
                    p.extra
                        .get("species_labor_coefficient")
                        .and_then(Value::as_f64)
                })
                .or_else(|| {
                    p.extra
                        .get("species_id")
                        .and_then(|id| {
                            w.species()
                                .iter()
                                .find(|s| s.get("id") == Some(id))
                        })
                        .and_then(|s| s["coefficients"]["physical"].as_f64())
                });
            let Some(coefficient) = coefficient.filter(|n| n.is_finite() && *n >= 0.) else {
                blocked.push(json!({"entity_id":o.id,"pop_id":p.external_id,"reason":"species_labor_coefficient_unknown"}));
                continue;
            };
            if o.kind == "factory" {
                forecast.accept(&o.entity_id, p, count)?;
            }
            let local = forecast
                .cash
                .balances
                .get(&quote.pool)
                .copied()
                .unwrap_or(Money::ZERO)
                .0;
            let general = forecast
                .cash
                .balances
                .get(&quote.source)
                .copied()
                .unwrap_or(Money::ZERO)
                .0;
            let expense = expense(quote, count)?.0;
            let transfer = (expense - local).max(0);
            forecast.set_cash(quote.pool.clone(), Money(local + transfer - expense));
            if transfer > 0 {
                forecast.set_cash(quote.source.clone(), Money(general - transfer));
                let mut op = EconomicOperation::transfer(
                    format!("hiring-funding:{}", json!([period, o.id, p.external_id])).into(),
                    quote.source.clone(),
                    quote.pool.clone(),
                    Money(transfer),
                );
                op.kind = if o.kind == "service" {
                    OperationKind::StateBudgetFunding
                } else {
                    OperationKind::LaborHiringFunding
                };
                op.meta.insert("income_recognized".into(), json!(false));
                if o.kind == "service" {
                    op.meta.insert("state_id".into(), o.owner_id.json());
                    op.meta.insert("budget_sector".into(), json!(o.sector));
                    op.meta
                        .insert("currency_id".into(), p.currency_id.as_ref().unwrap().json());
                }
                operations.push(op);
            }
            let c = &quote.contract;
            if contract_ids.insert(c.contract_id.clone()) {
                rows.put_contracts(c.contract_id.clone(),c.clone())?;
            }
            let job_id = crate::budget_funding::optional_id(c.extra.get("job_id"))?
                .ok_or_else(|| error("Hiring contract job unresolved"))?;
            if job_ids.insert(job_id.json().to_string()) {
                let key=rows.jobs().next_key().map_err(crate::operation_session::OperationError)?;
                rows.put_jobs(key,json!({"job_id":job_id,"building_id":o.id,"province_id":o.province_id.0,"slots_total":o.capacity,"capacity_pool_id":o.id,"capacity_pool_total":o.capacity,"slots_filled":0}))?;
            }
            let id = old
                .as_ref()
                .and_then(|a| a.employment_id.clone())
                .unwrap_or_else(|| {
                    format!("hire:{}:{}", typed(&o.id), typed(&p.external_id)).into()
                });
            let previous = old
                .as_ref()
                .map(|a| a.workers_count * a.time_fraction)
                .unwrap_or(0.);
            let fte = previous + count as f64;
            let workers = fte.ceil();
            let mut next = old.unwrap_or(LaborAssignment {
                pop_id: p.external_id.clone(),
                workers_count: 0.,
                time_fraction: 1.,
                fte: None,
                employment_id: None,
                contract_id: None,
                worker_group_id: None,
                productivity_multiplier: None,
                productivity_status: None,
                extra: Default::default(),
            });
            next.employment_id = Some(id.clone());
            next.contract_id = Some(c.contract_id.clone());
            next.workers_count = workers;
            next.time_fraction = fte / workers;
            next.productivity_multiplier = Some(coefficient);
            if next.fte.is_some() {
                next.fte = Some(fte)
            }
            if o.kind == "rgo" {
                let mut a=rows.resource_operations().get(&o.entity_id).cloned().ok_or_else(||error("Hiring RGO disappeared"))?;
                upsert_assignment(&mut a.base.assignments,next.clone());
                rows.put_resource_operations(a.base.id.clone(),a)?;
            } else if o.kind=="factory" {
                let a=rows.factories().get(&o.entity_id).ok_or_else(||error("Hiring factory disappeared"))?;
                let mut a=apply_staff_change(a,0,count)?;
                upsert_assignment(&mut a.base.assignments,next.clone());a.status=Some("operating".into());
                rows.put_factories(a.base.id.clone(),a)?;
            }
            let mut e=rows.employment().get(&id).cloned().unwrap_or(Employment {employment_id:id.clone(),pop_id:p.external_id.clone(),contract_id:c.contract_id.clone(),fte,extra:Default::default()});
            e.contract_id = c.contract_id.clone();
            e.fte = fte;
            e.extra.extend([
                ("job_id".into(), job_id.json()),
                (
                    "building_id".into(),
                    c.extra
                        .get("building_id")
                        .cloned()
                        .unwrap_or(o.entity_id.json()),
                ),
                ("province_id".into(), json!(p.province_id.0)),
                ("workers_count".into(), json!(workers)),
                ("time_fraction".into(), json!(fte / workers)),
                ("productivity_multiplier".into(), json!(coefficient)),
                ("effective_labor".into(), json!(fte * coefficient)),
                ("status".into(), json!("active_assignment")),
            ]);
            rows.put_employment(id.clone(),e)?;
            let used = roster.by_pop.get_mut(&p.external_id).unwrap();
            used.free -= count as f64;
            used.occupied += count as f64;
            hires.push(json!({"pop_id":p.external_id,"entity_id":o.id,"employment_id":id,"fte":count,"kind":o.kind}));
        }
    }
    drop(forecast);
    let changes=rows.finish()?;
    w.apply_delta(changes)?;
    apply_private_transfers_in(w, &operations)?;
    reconcile_labor_in(w, &build_labor_roster(w)?)?;
    Ok((hires, blocked, operations))
}
fn combined_affordable(d: &HiringDemands, q: &Quote, n: u64, money: Money) -> OpResult<bool> {
    let rent = d
        .rent
        .as_ref()
        .filter(|(k, _)| k == &q.pool || k == &q.source)
        .map(|(_, m)| m.0)
        .unwrap_or(0);
    Ok(expense(q, n)?.0 as i128 + rent as i128 <= money.0 as i128)
}
fn upsert_assignment(rows: &mut Vec<LaborAssignment>, next: LaborAssignment) {
    if let Some(a) = rows
        .iter_mut()
        .find(|a| a.employment_id == next.employment_id)
    {
        *a = next
    } else {
        rows.push(next)
    }
}
pub(crate) fn apply_private_transfers(w:&mut WorldState,ops:&[EconomicOperation])->OpResult<()> {
    crate::world_transaction::execute_atomic(w,|w|apply_private_transfers_in(w,ops))
}
pub(crate) fn apply_private_transfers_in(w:&mut crate::world_transaction::WorldTransaction<'_>,ops:&[EconomicOperation])->OpResult<()> {
    let mut max=w.accounts().iter().map(|a|a.id.0).max().unwrap_or(0);
    for op in ops {
        for d in &op.debits {
            let mut a=w.accounts().get(&d.address()).cloned().ok_or_else(||error("Missing hiring source account"))?;
            a.balance.0=a.balance.0.checked_sub(d.amount_minor.0).filter(|n|*n>=0).ok_or_else(||error("Hiring source money insufficient"))?;
            w.put_accounts(a.address(),a)?;
        }
        for c in &op.credits {
            let mut a=if let Some(a)=w.accounts().get(&c.address()) {a.clone()} else {
                max=max.checked_add(1).ok_or_else(||error("Hiring account id overflow"))?;
                AccountBalance {id:AccountId(max),external_id:format!("labor-private:{max}").into(),owner_id:c.owner_id.clone(),currency_id:c.currency_id.clone(),purpose:c.purpose.clone(),balance:Money::ZERO}
            };
            a.balance.0=a.balance.0.checked_add(c.amount_minor.0).ok_or_else(||error("Hiring credit overflow"))?;
            w.put_accounts(a.address(),a)?;
        }
    }
    Ok(())
}
pub fn plan_artisan_hiring(w:&dyn WorldRead,period:&str)->OpResult<LaborPlan> {
    let mut projected=crate::world_transaction::WorldTransaction::new(w);
    let result=plan_artisan_hiring_in(&mut projected,period)?;
    Ok(LaborPlan {hires:result.hires,blocked:result.blocked,operations:result.operations,rows:LaborRows::capture(&projected)})
}
pub(crate) fn plan_artisan_hiring_in(w:&mut crate::world_transaction::WorldTransaction<'_>, period: &str) -> OpResult<HiringResult> {
    if period.is_empty() {
        return Err(error("Artisan hiring period required"));
    }
    let mut roster = build_labor_roster(w)?;
    let mut stock = HashMap::new();
    for s in w.stockpiles().values() {
        *stock
            .entry((s.owner_id.clone(), s.province_id, s.goods.external_id()))
            .or_insert(0.) += s.quantity;
    }
    let methods: HashMap<_, _> = w.production_catalog()
        .as_ref()
        .ok_or_else(|| error("Artisan hiring catalog unresolved"))?
        .methods
        .iter()
        .map(|m| (&m.method_id, m))
        .collect();
    for e in w.artisans().values() {
        if let Some(m) = methods.get(&e.base.method_id) {
            let fte: f64 = e
                .base
                .assignments
                .iter()
                .map(|a| a.workers_count * a.time_fraction)
                .sum();
            for i in &m.inputs {
                let key = (
                    e.base
                        .owner_id
                        .clone()
                        .ok_or_else(|| error("Artisan owner unresolved"))?,
                    e.base.province_id,
                    i.goods_id.clone(),
                );
                let value = stock.get(&key).copied().unwrap_or(0.)
                    - fte * m.weekly_productivity.unwrap_or(0.) * i.amount;
                stock.insert(key, value.max(0.));
            }
        }
    }
    let cash = crate::labor_money::LaborCash::from_accounts(w);
    let mut forecast = HiringForecast::new(w, cash);
    let f = forecast.as_mut().map_err(|e| error(&e.0))?;
    let pops: HashMap<_, _> = w.pops().values().map(|p| (&p.external_id, p)).collect();
    let mut rows=crate::world_transaction::WorldTransaction::new(w);
    let mut changed = HashMap::new();
    let mut hires = vec![];
    for original in w.artisans().iter() {
        let mut e=original.clone();
        let Some(m) = methods.get(&e.base.method_id) else {
            continue;
        };
        if m.kind != "artisan"
            || m.status != "ready"
            || e.pending_retooling.as_ref().is_some_and(truthy)
            || m.technology
                .iter()
                .any(|t| !e.implemented_technologies.contains(t))
        {
            continue;
        }
        let mut slots = e.people_slots.unwrap_or(0.)
            - e.base
                .assignments
                .iter()
                .map(|a| a.workers_count)
                .sum::<f64>();
        let owner = e
            .base
            .owner_id
            .as_ref()
            .ok_or_else(|| error("Artisan owner unresolved"))?;
        for a in &mut e.base.assignments {
            let Some(p) = pops.get(&a.pop_id) else {
                continue;
            };
            if p.pop_type_id.as_str() != Some("artisans")
                || p.province_id != e.base.province_id
                || p.extra.get("mobility_status").and_then(Value::as_str) == Some("in_transit")
            {
                continue;
            }
            let mut count = slots
                .min(roster.by_pop[&p.external_id].free)
                .max(0.)
                .floor() as u64;
            for input in &m.inputs {
                let key = (owner.clone(), e.base.province_id, input.goods_id.clone());
                count = count.min(
                    (stock.get(&key).copied().unwrap_or(0.)
                        / (input.amount * m.weekly_productivity.unwrap_or(0.)))
                    .max(0.)
                    .floor() as u64,
                );
            }
            count = f.limit(&e.base.id, p, count, |_, _| Ok(true))?;
            if count == 0 {
                continue;
            }
            f.accept(&e.base.id, p, count)?;
            let fte = a.workers_count * a.time_fraction + count as f64;
            let workers = fte.ceil();
            a.workers_count = workers;
            a.time_fraction = fte / workers;
            if a.fte.is_some() {
                a.fte = Some(fte)
            }
            slots -= count as f64;
            let used = roster.by_pop.get_mut(&p.external_id).unwrap();
            used.free -= count as f64;
            used.occupied += count as f64;
            for input in &m.inputs {
                *stock
                    .entry((owner.clone(), e.base.province_id, input.goods_id.clone()))
                    .or_default() -=
                    count as f64 * input.amount * m.weekly_productivity.unwrap_or(0.);
            }
            changed.insert(
                a.employment_id
                    .clone()
                    .ok_or_else(|| error("Artisan employment unresolved"))?,
                (a.clone(), fte),
            );
            hires.push(
                json!({"entity_id":e.base.id,"pop_id":p.external_id,"fte":count,"kind":"artisan"}),
            );
        }
        if e!=*original {rows.put_artisans(e.base.id.clone(),e)?;}
    }
    for e in w.employment().iter() {
        if let Some((a, fte)) = changed.get(&e.employment_id) {
            let mut e=e.clone();
            e.fte = *fte;
            e.extra
                .insert("workers_count".into(), json!(a.workers_count));
            e.extra
                .insert("time_fraction".into(), json!(a.time_fraction));
            rows.put_employment(e.employment_id.clone(),e)?;
        }
    }
    drop(forecast);
    let delta=rows.finish()?;
    w.apply_delta(delta)?;
    let roster=build_labor_roster(w)?;
    reconcile_labor_in(w,&roster)?;
    Ok(HiringResult {
        hires,
        blocked: vec![],
        operations: vec![],
    })
}

fn typed(id: &ExternalId) -> String {
    crate::labor_identity::component(id)
}
