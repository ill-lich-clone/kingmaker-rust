//! Isolated native producer cycle. Full weeks remain guarded by pipeline.rs.
use kingmaker_core::world_read::WorldRead;
use crate::{
    operation_session::*, production_activities::*, production_feasibility::*,
    production_payroll::*, storage::*,
};
use kingmaker_core::operations::*;
use kingmaker_core::production::*;
use kingmaker_core::*;
use serde_json::json;
use std::collections::{HashMap, HashSet};
pub type ProductionError = OperationError;
#[derive(Debug, Clone)]
pub struct ProductionRecipe {
    pub inputs: Vec<(Goods, f64)>,
    pub outputs: Vec<(Goods, f64)>,
    pub base_labor: u32,
}
#[derive(Debug, Default)]
pub struct ProductionResult {
    pub total_produced: HashMap<Goods, f64>,
    pub total_consumed: HashMap<Goods, f64>,
}
/// Detached legacy rows cannot establish ownership, payroll or storage.
pub fn execute_world_production(
    _provinces: &[ProvinceRecord],
    _enterprises: &[EnterpriseRecord],
    _stockpiles: &mut [StockpileRecord],
    _recipes: &HashMap<String, ProductionRecipe>,
) -> Result<ProductionResult, ProductionError> {
    Err(OperationError("Use execute_production_cycle with authoritative WorldState/request; detached legacy recipes cannot execute production".into()))
}
fn err(s: &str) -> OperationError {
    OperationError(s.into())
}
pub fn execute_production_cycle(world:&mut WorldState,request:&ProductionRequest)->Result<ProductionPhaseReceipt,ProductionError> {
    crate::world_transaction::execute_atomic(world,|world|execute_production_cycle_in(world,request))
}
pub(crate) fn execute_production_cycle_in(
    world: &mut crate::world_transaction::WorldTransaction<'_>,
    request: &ProductionRequest,
) -> Result<ProductionPhaseReceipt, ProductionError> {
    if request.expected_revision != world.campaign().revision || request.period.is_empty() {
        return Err(err("Invalid phase revision/period"));
    }
    let mut effective = world.production_policies().clone();
    effective.taxes.extend(request.policies.taxes.clone());
    effective.wages.extend(request.policies.wages.clone());
    if !request.policies.priorities.is_empty() {
        effective.priorities = request.policies.priorities.clone();
    }
    let prior: Vec<_> = world.production_phase_receipts()
        .iter()
        .filter(|r| r.request.period == request.period)
        .collect();
    if let Some(old) = prior.first() {
        if prior.len() != 1
            || old.campaign_id != world.campaign().campaign_id
            || old.request != *request
            || old.effective_policies != effective
        {
            return Err(err("Conflicting production phase replay"));
        }
        crate::operation_history::validate_production_history(world, old)?;
        return Ok((*old).clone());
    }
    crate::production_validation::validate_production_catalog(world)?;
    let candidates = plan_production_activities(world, request)?;
    let mut s = OperationSession::new(world)?;
    s.configure(request)?;
    plan_prior_production_obligations(&mut s, &request.period)?;
    let plan = plan_feasible_production(&mut s, request, &candidates)?;
    let payroll = pay_production_labor(&mut s, request, &plan)?;
    let outputs = actual_production(&mut s, request, &plan, &payroll)?;
    let totals = |accounts: Vec<&AccountBalance>| {
        let mut map = HashMap::new();
        for a in accounts {
            *map.entry(a.currency_id.clone()).or_insert(0i128) += i128::from(a.balance.0);
        }
        map
    };
    if totals(world.accounts().iter().collect()) != totals(s.accounts().collect()) {
        return Err(err("Unexplained production-cycle money delta"));
    }
    let mut receipt = ProductionPhaseReceipt {
        procurement_activities: plan
            .activities
            .iter()
            .map(|a| ProcurementActivity {
                activity_id: a.activity_id.clone(),
                subject_id: a.subject_id.clone(),
                method_id: a.method_id.clone(),
                potential_output: a.potential_output,
                labor_output: a.labor_output,
                output: a.output,
            })
            .collect(),
        planned_outputs: plan
            .activities
            .iter()
            .map(|a| {
                Ok(ProductionOutputReceipt {
                    assignments: a.assignments.clone(),
                    activity_id: a.activity_id.clone(),
                    subject_id: a.subject_id.clone(),
                    method_kind: method(world, &a.method_id)?.kind.clone(),
                    goods_id: a.output_goods_id.clone(),
                    quantity: a.output,
                    fte: a.fte,
                    blocked_reason: a.blocked_reason.clone(),
                })
            })
            .collect::<OpResult<_>>()?,
        registry_changes: s.registry_changes(),
        invariants: Default::default(),
        campaign_id: world.campaign().campaign_id.clone(),
        request: request.clone(),
        effective_policies: effective,
        outputs,
        payroll,
        event_ids: s
            .operations()
            .iter()
            .map(|op| op.event_id.clone())
            .collect(),
        operation_evidence: s
            .operations()
            .iter()
            .map(crate::operation_history::operation_evidence)
            .collect::<OpResult<_>>()?,
        purchase_priorities: plan.purchase_priorities,
    };
    for paid in &receipt.payroll {
        if !receipt.event_ids.contains(&paid.event_id) {
            receipt.event_ids.push(paid.event_id.clone());
            let op: EconomicOperation = serde_json::from_value(serde_json::to_value(paid)?)?;
            receipt
                .operation_evidence
                .push(crate::operation_history::operation_evidence(&op)?);
        }
    }
    receipt.invariants =
        crate::production_invariants::check_session_production_operations(&s, &receipt);
    if !receipt.invariants.production_required_passed() {
        return Err(OperationError(format!(
            "Production audit rejected: {:?}",
            receipt.invariants
        )));
    }
    let d = s.finish()?;
    world.apply_phase(d)?;
    let key=world.production_phase_receipts().next_key().map_err(OperationError)?;
    world.put_production_phase_receipts(key,receipt.clone())?;
    Ok(receipt)
}
fn actual_production(
    s: &mut OperationSession,
    request: &ProductionRequest,
    plan: &FeasibleProductionPlan,
    payroll: &[PayrollReceipt],
) -> OpResult<Vec<ProductionOutputReceipt>> {
    let mut by_employment = HashMap::new();
    for r in payroll {
        if r.period != request.period
            || !r.actual_fte.is_finite()
            || r.actual_fte < 0.0
            || s.committed(&r.event_id).is_none()
            || by_employment.insert(&r.employment_id, r).is_some()
        {
            return Err(err(
                "Actual production requires unique committed current payroll",
            ));
        }
    }
    let mut batch = StorageBatch::new(s)?;
    let mut outputs = vec![];
    let collator = crate::ordering::collator();
    for a in &plan.activities {
        let m = method(s.base, &a.method_id)?.clone();
        let entity = producer(s.base, &a.subject_id)?.clone();
        let event: ExternalId = format!(
            "production:{}:{}:{}",
            s.base.campaign().campaign_id, request.period, a.activity_id
        )
        .into();
        if s.committed(&event).is_some() {
            return Err(err(
                "Production event already committed without complete phase receipt",
            ));
        }
        let mut fraction = 1.0f64;
        for assignment in &a.assignments {
            if m.kind == "artisan" {
                continue;
            }
            let pop = s
                .base.pops().values()
                .find(|p| p.external_id == assignment.pop_id)
                .ok_or_else(|| err("Missing actual worker"))?;
            if pop.pop_type_id.as_str() == Some("slaves") {
                if request
                    .maintenance_receipts
                    .iter()
                    .find(|r| r.pop_id == assignment.pop_id)
                    .is_none_or(|r| {
                        !crate::production_validation::maintenance_verified(s, request, r)
                    })
                {
                    fraction = 0.0;
                }
                continue;
            }
            let fte = assignment.workers_count * assignment.time_fraction;
            if fte > 0.0 {
                fraction = fraction.min(
                    assignment
                        .employment_id
                        .as_ref()
                        .and_then(|id| by_employment.get(id))
                        .map(|r| r.actual_fte)
                        .unwrap_or(0.0)
                        / fte,
                );
            }
        }
        fraction = fraction.clamp(0.0, 1.0);
        let quantity = a.output * fraction;
        let fte = a
            .assignments
            .iter()
            .map(|r| r.workers_count * r.time_fraction * fraction)
            .sum();
        let held = a
            .storage_reservation_id
            .as_ref()
            .and_then(|id| s.storage_reservation(id))
            .cloned();
        if quantity > 0.0 {
            let owner = a
                .owner_id
                .as_ref()
                .ok_or_else(|| err("Unknown actual owner"))?;
            let held = held
                .as_ref()
                .filter(|r| {
                    r.status == "held"
                        && &r.owner_id == owner
                        && r.goods_id == m.output_goods_id
                        && Some(&r.facility_id) == entity.facility_id.as_ref()
                        && r.unit == m.output_unit
                        && r.quantity + 1e-9 >= quantity
                })
                .ok_or_else(|| err("Output requires actual owned finite storage"))?;
            if batch
                .facilities
                .get(&held.facility_id)
                .is_none_or(|f| f.province_id != a.province_id)
            {
                return Err(err("Output warehouse belongs to another province"));
            }
            let mut changes = vec![];
            let mut allocated: HashMap<ExternalId, f64> = HashMap::new();
            let mut protected = HashMap::new();
            for r in s.base.goods_reservations().values() {
                if r.status == "held" {
                    *protected.entry(&r.stock_id).or_insert(0.0) += r.quantity;
                }
            }
            for input in &m.inputs {
                let mut remaining = input.amount * quantity;
                let mut stocks: Vec<_> = s
                    .stocks()
                    .filter(|r| {
                        &r.owner_id == owner
                            && r.province_id == a.province_id
                            && r.goods.external_id() == input.goods_id
                            && r.purpose != "food_reserve"
                    })
                    .collect();
                stocks.sort_by(|a, b| {
                    collator.compare(
                        &external_text(&a.external_id),
                        &external_text(&b.external_id),
                    )
                });
                for row in stocks {
                    let used = remaining.min(
                        (row.quantity
                            - protected.get(&row.external_id).copied().unwrap_or(0.0)
                            - allocated.get(&row.external_id).copied().unwrap_or(0.0))
                        .max(0.0),
                    );
                    if used > 0.0 {
                        changes.push(StockDelta {
                            stock_id: row.external_id.clone(),
                            delta: -used,
                            create: None,
                        });
                        *allocated.entry(row.external_id.clone()).or_default() += used;
                        remaining -= used;
                    }
                }
                if remaining > 1e-8 {
                    return Err(err("Actual production exceeds owned free inputs"));
                }
            }
            let existing: Vec<_> = s
                .stocks()
                .filter(|r| {
                    &r.owner_id == owner
                        && r.province_id == a.province_id
                        && r.goods.external_id() == m.output_goods_id
                        && r.facility_id.as_ref() == Some(&held.facility_id)
                        && r.inventory_origin.as_deref() == Some("produced")
                })
                .collect();
            if existing.len() > 1 {
                return Err(err("Ambiguous produced inventory"));
            }
            let purpose = entity
                .payment_purpose
                .as_deref()
                .unwrap_or(if m.kind == "artisan" {
                    "operating_capital"
                } else {
                    "employer_operating_capital"
                });
            let currencies: HashSet<_> = s
                .accounts()
                .filter(|a| &a.owner_id == owner && a.purpose == purpose)
                .map(|a| a.currency_id.clone())
                .collect();
            if currencies.len() > 1 {
                return Err(err("Production accounting currency ambiguous"));
            }
            let fallback = entity
                .extra
                .get("working_capital")
                .and_then(|v| v.get("currency_id"))
                .cloned()
                .and_then(|v| ExternalId::from_json(v).ok())
                .or_else(|| {
                    s.base.artisans().values()
                        .find(|r| r.base.id == entity.id)
                        .and_then(|a| a.working_capital.as_ref())
                        .and_then(|c| c.get("currency_id"))
                        .cloned()
                        .and_then(|v| ExternalId::from_json(v).ok())
                });
            let currency = currencies
                .into_iter()
                .next()
                .or(fallback)
                .ok_or_else(|| err("Production accounting currency unresolved"))?;
            let stock_id = existing
                .first()
                .map(|r| r.external_id.clone())
                .unwrap_or_else(|| {
                    ExternalId::from(format!(
                        "produced:{}:{}:{}:{}",
                        external_text(owner),
                        a.province_id.0,
                        external_text(&m.output_goods_id),
                        external_text(&held.facility_id)
                    ))
                });
            let create = if existing.is_empty() {
                Some(StockCreate {
                    owner_id: owner.clone(),
                    province_id: a.province_id,
                    goods_id: m.output_goods_id.clone(),
                    unit: m.output_unit.clone(),
                    facility_id: Some(held.facility_id.clone()),
                    currency_id: Some(currency),
                    inventory_origin: "produced".into(),
                    method_kind: Some(m.kind.clone()),
                    extra: Extra::new(),
                })
            } else {
                None
            };
            changes.push(StockDelta {
                stock_id,
                delta: quantity,
                create,
            });
            let mut op: EconomicOperation = serde_json::from_value(
                json!({"event_id":event,"type":"production","method_kind":m.kind,"subject_id":a.subject_id,"activity_id":a.activity_id,"province_id":a.province_id.0,"goods_id":m.output_goods_id,"output_quantity":quantity,"planned_possible_output":a.output,"causes":["typed method","actual paid or independent labor","owned recipe inputs","finite reserved storage"]}),
            )?;
            op.stock_changes = changes;
            s.authorize_production(&op);
            s.stage(&op)?;
            consume_storage(s, &mut batch, &held.id, quantity)?;
            batch.adjust(
                &held.facility_id,
                quantity
                    * batch.facilities[&held.facility_id].goods[&external_text(&held.goods_id)]
                        .space_per_unit,
            )?;
        }
        if let Some(held) = held {
            if s.storage_reservation(&held.id).unwrap().status == "held" {
                release_storage(
                    s,
                    &mut batch,
                    &held.id,
                    &format!("release:{}:{}", request.period, external_text(&held.id)).into(),
                )?;
            }
        }
        if m.kind == "factory" {
            if let Some(f) = s
                .factory(&a.subject_id)
                .filter(|f| f.staff_count.is_some_and(|n| n > 0.0))
            {
                let updated = update_factory_tenure(f, fte)?;
                s.factory_rows.insert(a.subject_id.clone(), updated);
            }
        }
        outputs.push(ProductionOutputReceipt {
            assignments: a
                .assignments
                .iter()
                .cloned()
                .map(|mut r| {
                    r.time_fraction *= fraction;
                    r.fte = Some(r.workers_count * r.time_fraction);
                    r
                })
                .collect(),
            activity_id: a.activity_id.clone(),
            subject_id: a.subject_id.clone(),
            method_kind: m.kind,
            goods_id: m.output_goods_id,
            quantity,
            fte,
            blocked_reason: a.blocked_reason.clone(),
        });
    }
    Ok(outputs)
}
