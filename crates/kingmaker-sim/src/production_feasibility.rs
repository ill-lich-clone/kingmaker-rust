use kingmaker_core::world_read::WorldRead;
use crate::{money_math::*, operation_session::*, production_activities::*, storage::*};
use kingmaker_core::production::*;
use kingmaker_core::*;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, HashMap};
fn err(s: &str) -> OperationError {
    OperationError(s.into())
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PayrollRequest {
    pub employment_id: ExternalId,
    pub pop_id: ExternalId,
    pub contract_id: ExternalId,
    pub subject_id: ExternalId,
    pub activity_id: String,
    pub gross_minor: Money,
    pub working_gross_minor: Money,
    pub idle_guarantee_minor: Money,
    pub planned_fte: f64,
    pub employer_id: ExternalId,
    pub currency_id: ExternalId,
    pub purpose: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeasibleProductionPlan {
    pub activities: Vec<ProductionActivity>,
    pub payroll_requests: Vec<PayrollRequest>,
    pub blocked: Vec<(String, String)>,
    pub purchase_priorities: Vec<String>,
    pub input_reservations: Vec<(String, ExternalId, f64)>,
}
struct Wage {
    assignment: LaborAssignment,
    contract: LaborContract,
    rate: MoneyRate,
    multiplier: f64,
    full: Money,
    floor: Money,
    key: String,
    fte: f64,
}
struct Work {
    original: ProductionActivity,
    goal: f64,
    assignments: Vec<LaborAssignment>,
    demands: BTreeMap<String, f64>,
    factor: f64,
    wages: Vec<Wage>,
}
fn money_key(owner: &ExternalId, currency: &ExternalId, purpose: &str) -> String {
    json!(["money", owner, currency, purpose]).to_string()
}
fn goods_key(owner: &ExternalId, province: ProvinceId, goods: &ExternalId) -> String {
    json!(["goods", owner, province.0.to_string(), goods]).to_string()
}
pub fn plan_feasible_production(
    s: &mut OperationSession,
    request: &ProductionRequest,
    candidates: &[ProductionActivity],
) -> OpResult<FeasibleProductionPlan> {
    let result = feasible(s, request, candidates);
    if result.is_err() {
        s.poison();
    }
    result
}
fn feasible(
    s: &mut OperationSession,
    request: &ProductionRequest,
    candidates: &[ProductionActivity],
) -> OpResult<FeasibleProductionPlan> {
    if s.period.is_none() {
        s.configure(request)?;
    }
    let productivity = request.profile.productivity_multiplier.unwrap_or(1.0);
    if !productivity.is_finite() || productivity < 0.0 {
        return Err(err("Invalid productivity profile"));
    }
    let mut batch = StorageBatch::new(s)?;
    let mut availability: BTreeMap<String, f64> = BTreeMap::new();
    let mut money: HashMap<String, i128> = HashMap::new();
    for a in s.accounts() {
        money.insert(
            money_key(&a.owner_id, &a.currency_id, &a.purpose),
            i128::from(a.balance.0),
        );
    }
    for debt in s.base.obligations().values() {
        let d = s.debt(&debt.obligation_id).unwrap();
        let cash = money
            .entry(money_key(&d.payer_id, &d.currency_id, &d.purpose))
            .or_default();
        *cash = (*cash - i128::from(d.amount_minor.0)).max(0);
    }
    let mut held = HashMap::new();
    for r in s.base.goods_reservations().values() {
        if r.status == "held" {
            if !r.quantity.is_finite() || r.quantity < 0.0 {
                return Err(err("Invalid held input quantity"));
            }
            *held.entry(&r.stock_id).or_insert(0.0) += r.quantity;
        }
    }
    for stock in s.stocks() {
        if stock.purpose == "food_reserve" {
            continue;
        }
        if s.base.production_catalog().as_ref().is_some_and(|c| {
            c.methods.iter().any(|m| {
                m.inputs
                    .iter()
                    .any(|i| i.goods_id == stock.goods.external_id())
            })
        }) && stock.unit.as_ref()
            != Some(&crate::production_validation::goods_unit(
                s.base,
                &stock.goods.external_id(),
            )?)
        {
            return Err(err("Recipe input commodity unit mismatch"));
        }
        if !stock.quantity.is_finite() || stock.quantity < 0.0 {
            return Err(err("Invalid input quantity"));
        }
        *availability
            .entry(goods_key(
                &stock.owner_id,
                stock.province_id,
                &stock.goods.external_id(),
            ))
            .or_default() +=
            (stock.quantity - held.get(&stock.external_id).copied().unwrap_or(0.0)).max(0.0);
    }
    let contracts: HashMap<_, _> = s
        .base.contracts().values()
        .map(|c| (&c.contract_id, c))
        .collect();
    let pops: HashMap<_, _> = s.base.pops().values().map(|p| (&p.external_id, p)).collect();
    let collator = crate::ordering::collator();
    let mut candidates = candidates.to_vec();
    candidates.sort_by(|a, b| collator.compare(&a.activity_id, &b.activity_id));
    let mut work = vec![];
    let mut blocked = vec![];
    for a in candidates {
        let entity = producer(s.base, &a.subject_id)?;
        let m = method(s.base, &a.method_id)?;
        let base = a.potential_output.unwrap_or(a.output);
        let goal = if m.kind == "factory" {
            a.equipment_capacity_weekly
                .map(|c| (a.labor_output.unwrap_or(base) * productivity).min(c))
                .unwrap_or(0.0)
        } else {
            base * productivity
        };
        if !goal.is_finite() || goal < 0.0 {
            return Err(err("Nonfinite production goal"));
        }
        let equipment_factor = if a.labor_output.is_some_and(|v| v > 0.0) && productivity > 0.0 {
            (goal / (a.labor_output.unwrap() * productivity)).min(1.0)
        } else {
            1.0
        };
        let assignments: Vec<_> = a
            .assignments
            .iter()
            .cloned()
            .map(|mut a| {
                a.time_fraction *= equipment_factor;
                a
            })
            .collect();
        let mut item = Work {
            original: a,
            goal,
            assignments,
            demands: BTreeMap::new(),
            factor: 1.0,
            wages: vec![],
        };
        let fail = |item: &mut Work, blocked: &mut Vec<(String, String)>, reason: &str| {
            item.factor = 0.0;
            blocked.push((item.original.activity_id.clone(), reason.into()));
        };
        if let Some(reason) = item
            .original
            .blocked_reason
            .clone()
            .filter(|r| r != "owned inputs unavailable")
        {
            fail(&mut item, &mut blocked, &reason);
        }
        let Some(owner) = entity.owner_id.as_ref() else {
            fail(&mut item, &mut blocked, "production_owner_unknown");
            work.push(item);
            continue;
        };
        if goal <= 0.0 {
            item.factor = 0.0;
        }
        for i in &m.inputs {
            item.demands.insert(
                goods_key(owner, entity.province_id, &i.goods_id),
                goal * i.amount,
            );
        }
        if let Some(facility_id) = entity.facility_id.as_ref() {
            let storage = batch.availability(facility_id, &m.output_goods_id);
            if let Some(free) = storage.free_space {
                let facility = &batch.facilities[facility_id];
                let mapping = &facility.goods[&external_text(&m.output_goods_id)];
                let key = json!(["space", facility.id]).to_string();
                availability.insert(key.clone(), free);
                item.demands.insert(key, goal * mapping.space_per_unit);
                if &facility.owner_id != owner {
                    let lease = facility.access_contracts.iter().find(|c| {
                        c.owner_id.as_ref() == Some(owner)
                            && approved(
                                c.status.as_deref(),
                                c.source.as_ref(),
                                c.approval_ref.as_ref(),
                            )
                    });
                    if let Some(c) = lease.filter(|c| {
                        c.price_minor_per_space.is_some()
                            && c.currency_id.is_some()
                            && c.purpose.is_some()
                    }) {
                        let key = money_key(
                            owner,
                            c.currency_id.as_ref().unwrap(),
                            c.purpose.as_ref().unwrap(),
                        );
                        let rent = multiply_money(
                            c.price_minor_per_space.unwrap(),
                            goal * mapping.space_per_unit,
                        )?;
                        *item.demands.entry(key).or_default() += rent.0 as f64;
                    } else {
                        fail(&mut item, &mut blocked, "output_storage_access_unknown");
                    }
                }
            } else {
                fail(&mut item, &mut blocked, "output_storage_unknown");
            }
        } else {
            fail(&mut item, &mut blocked, "output_storage_unknown");
        }
        if item.original.payroll_required {
            for a in item.assignments.clone() {
                let pop = pops
                    .get(&a.pop_id)
                    .ok_or_else(|| err("Unknown payroll POP"))?;
                if pop.pop_type_id.as_str() == Some("slaves") {
                    let maintenance = request
                        .maintenance_receipts
                        .iter()
                        .find(|r| r.pop_id == a.pop_id);
                    if let Some(r) = maintenance.filter(|r| {
                        crate::production_validation::maintenance_verified(s, request, r)
                    }) {
                        item.factor = item.factor.min(r.fraction);
                    } else {
                        fail(&mut item, &mut blocked, "slave_maintenance_plan_required");
                    }
                    continue;
                }
                let Some(c) = a
                    .contract_id
                    .as_ref()
                    .and_then(|id| contracts.get(id))
                    .filter(|c| c.employer_entity_id.is_some() && c.currency_id.is_some())
                else {
                    fail(&mut item, &mut blocked, "production_wage_unknown");
                    continue;
                };
                let rate = match contract_rate(c) {
                    Ok(r) => r,
                    Err(_) => {
                        fail(&mut item, &mut blocked, "production_wage_unknown");
                        continue;
                    }
                };
                let employer = c.employer_entity_id.as_ref().unwrap();
                let multiplier = s
                    .policies
                    .wages
                    .get(&external_text(employer))
                    .copied()
                    .unwrap_or(1.0);
                if !multiplier.is_finite() || multiplier < 0.0 {
                    return Err(err("Invalid wage policy"));
                }
                let fte = a.workers_count * a.time_fraction;
                let floor = if let Some(r) = &c.guaranteed_idle_weekly_rate {
                    rate_pay(r, fte, 1.0)?
                } else {
                    Money::ZERO
                };
                let full = rate_pay(&rate, fte, multiplier)?;
                let key = money_key(
                    employer,
                    c.currency_id.as_ref().unwrap(),
                    c.payment_purpose
                        .as_deref()
                        .unwrap_or("employer_operating_capital"),
                );
                item.wages.push(Wage {
                    assignment: a,
                    contract: (*c).clone(),
                    rate,
                    multiplier,
                    full,
                    floor,
                    key,
                    fte,
                });
            }
        }
        work.push(item);
    }
    let mut groups: BTreeMap<String, Vec<(usize, usize)>> = BTreeMap::new();
    for (i, item) in work.iter().enumerate() {
        for (j, r) in item.wages.iter().enumerate() {
            groups.entry(r.key.clone()).or_default().push((i, j));
        }
    }
    for (key, rows) in groups {
        let cash = money.get(&key).copied().unwrap_or(0);
        let guarantees: i128 = rows
            .iter()
            .map(|&(i, j)| i128::from(work[i].wages[j].floor.0))
            .sum();
        if guarantees > cash {
            for &(i, _) in &rows {
                work[i].factor = 0.0;
            }
            money.insert(key.clone(), 0);
        } else {
            money.insert(key.clone(), cash - guarantees);
        }
        for (i, j) in rows {
            let r = &work[i].wages[j];
            let demand = (r.full.0 - r.floor.0).max(0) as f64;
            *work[i].demands.entry(key.clone()).or_default() += demand;
        }
    }
    for (key, cash) in money {
        availability.insert(key, cash as f64);
    }
    let mut users: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, item) in work.iter().enumerate() {
        for key in item.demands.keys() {
            users.entry(key.clone()).or_default().push(i);
        }
    }
    let mut keys: Vec<_> = users.keys().cloned().collect();
    keys.sort_by(|a, b| crate::ordering::utf16_compare(a, b));
    for key in keys {
        let indexes = users.remove(&key).expect("indexed constraint");
        let active: Vec<_> = indexes
            .into_iter()
            .filter(|i| work[*i].factor > 0.0 && work[*i].demands[&key] > 0.0)
            .collect();
        let demand: f64 = active
            .iter()
            .map(|i| work[*i].demands[&key] * work[*i].factor)
            .sum();
        let available = availability.get(&key).copied().unwrap_or(0.0);
        if demand > available {
            let ratio = available / demand;
            for i in active {
                work[i].factor *= ratio;
            }
        }
    }
    let mut plan = FeasibleProductionPlan {
        activities: vec![],
        payroll_requests: vec![],
        blocked,
        purchase_priorities: if s.policies.priorities.is_empty() {
            vec![
                "basic".into(),
                "production_inputs".into(),
                "everyday".into(),
                "luxury".into(),
            ]
        } else {
            s.policies.priorities.clone()
        },
        input_reservations: vec![],
    };
    for item in work {
        let mut a = item.original.clone();
        a.output = item.goal * item.factor;
        a.assignments = item
            .assignments
            .iter()
            .cloned()
            .map(|mut r| {
                r.time_fraction *= item.factor;
                r.fte = Some(r.workers_count * r.time_fraction);
                r
            })
            .collect();
        a.fte = a.assignments.iter().map(|r| r.fte.unwrap()).sum();
        a.feasible_fraction = item.factor;
        a.inputs = method(s.base, &a.method_id)?
            .inputs
            .iter()
            .cloned()
            .map(|mut i| {
                i.amount *= a.output;
                i
            })
            .collect();
        a.blocked_reason = if a.output == 0.0 {
            Some(
                item.original
                    .blocked_reason
                    .unwrap_or("resources_or_budget_unavailable".into()),
            )
        } else {
            None
        };
        for i in &a.inputs {
            plan.input_reservations
                .push((a.activity_id.clone(), i.goods_id.clone(), i.amount));
        }
        for r in item.wages {
            let working = rate_pay(&r.rate, r.fte * item.factor, r.multiplier)?;
            let idle = if let Some(rate) = &r.contract.guaranteed_idle_weekly_rate {
                rate_pay(rate, r.fte * (1.0 - item.factor), 1.0)?
            } else {
                Money::ZERO
            };
            let gross = Money(
                working
                    .0
                    .checked_add(idle.0)
                    .ok_or_else(|| err("Payroll overflow"))?,
            );
            plan.payroll_requests.push(PayrollRequest {
                employment_id: r
                    .assignment
                    .employment_id
                    .ok_or_else(|| err("Production employment identity required"))?,
                pop_id: r.assignment.pop_id,
                contract_id: r.contract.contract_id,
                subject_id: a.subject_id.clone(),
                activity_id: a.activity_id.clone(),
                gross_minor: gross,
                working_gross_minor: working,
                idle_guarantee_minor: idle,
                planned_fte: r.fte * item.factor,
                employer_id: r.contract.employer_entity_id.unwrap(),
                currency_id: r.contract.currency_id.unwrap(),
                purpose: r
                    .contract
                    .payment_purpose
                    .unwrap_or("employer_operating_capital".into()),
            });
        }
        if a.output > 0.0 {
            let req = StorageRequest {
                id: format!("output:{}:{}", request.period, a.activity_id).into(),
                owner_id: a
                    .owner_id
                    .clone()
                    .ok_or_else(|| err("Unknown feasible owner"))?,
                facility_id: a
                    .facility_id
                    .clone()
                    .ok_or_else(|| err("Unknown feasible facility"))?,
                goods_id: a.output_goods_id.clone(),
                unit: a.output_unit.clone(),
                quantity: a.output,
            };
            let reserved = reserve_storage(s, &mut batch, &req, &request.period)?;
            if reserved.reason.is_some() || reserved.partial {
                return Err(err("Feasible output/storage reservation mismatch"));
            }
            a.storage_reservation_id = Some(req.id);
        }
        plan.activities.push(a);
    }
    Ok(plan)
}
