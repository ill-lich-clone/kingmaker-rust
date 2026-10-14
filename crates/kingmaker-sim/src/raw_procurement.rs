//! State raw purchases use the real treasury after protecting current public services and debt.
use kingmaker_core::world_read::WorldRead;
use crate::{
    budget_funding::{add_money, error, state_requirements, treasury},
    operation_session::{OpResult, OperationSession},
};
use kingmaker_core::{production::ProductionPolicies, *};
pub fn free_procurement_budget(
    w: &dyn WorldRead,
    state: &ExternalId,
    currency: &ExternalId,
    policies: &ProductionPolicies,
) -> OpResult<Money> {
    let mut s = OperationSession::new(w)?;
    s.policies = policies.clone();
    free_budget(&s, state, currency)
}
fn free_budget(s: &OperationSession, state: &ExternalId, currency: &ExternalId) -> OpResult<Money> {
    let owner = treasury(state);
    let mut reserve = Money::ZERO;
    for r in state_requirements(s)? {
        if &r.state == state && &r.currency == currency {
            reserve = add_money(
                reserve,
                Money((r.amount.0 - s.cash(&r.owner, currency, &r.purpose).0).max(0)),
            )?;
        }
    }
    for d in s.base.obligations().values() {
        if (&d.payer_id == state || d.payer_id == owner)
            && &d.currency_id == currency
            && !d.purpose.starts_with("state_service:")
        {
            if d.amount_minor.0 < 0 {
                return Err(error("Negative state procurement debt"));
            }
            reserve = add_money(reserve, d.amount_minor)?;
        }
    }
    Ok(Money(
        (s.cash(&owner, currency, "operating_capital").0 - reserve.0).max(0),
    ))
}
use crate::{
    market_operations::invoice,
    money_math::{allocate_budget, multiply_money, BudgetRequest as AllocationRequest},
    operation_history::operation_evidence,
    operation_session::canonical_operation,
    transport_validation::actor,
};
use kingmaker_core::{
    budget::BudgetRequest, calendar::GameDate, market::*, operations::*, transport::*,
};
use num_traits::ToPrimitive;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
pub struct PreparedRawProcurement {
    reads: Option<crate::world_transaction::PrepareProof>,
    state: Option<crate::world_transaction::PreparedWorldDelta>,
    receipt: TransportPhaseReceipt,
    request: BudgetRequest,
}
fn hash(v: &Value) -> OpResult<String> {
    Ok(Sha256::digest(serde_json::to_vec(v)?)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}
fn transport(r: &BudgetRequest) -> TransportRequest {
    TransportRequest {
        expected_revision: r.expected_revision,
        period: r.period.clone(),
        stage: TransportStage::StateProcurement,
    }
}
fn boundary(w: &dyn WorldRead, r: &BudgetRequest) -> String {
    format!(
        "supply-phase:{}:{}:state-procurement",
        w.campaign().campaign_id, r.period
    )
}
fn inputs(w: &dyn WorldRead, r: &BudgetRequest) -> OpResult<Vec<OperationEvidence>> {
    let dispatch = crate::physical_supply::replay(
        w,
        &TransportRequest {
            stage: TransportStage::Dispatch,
            ..transport(r)
        },
    )?
    .ok_or_else(|| error("Raw procurement requires same-period native physical supply"))?;
    Ok(dispatch.operation_evidence)
}
pub(crate) fn replay(w: &dyn WorldRead, r: &BudgetRequest) -> OpResult<Option<TransportPhaseReceipt>> {
    let found = w.all_transport_phase_receipts().select(|p| {
            p.request.stage == TransportStage::StateProcurement && p.request.period == r.period
        })?;
    if found.is_empty() {
        return Ok(None);
    }
    if found.len() != 1
        || found[0].campaign_id != w.campaign().campaign_id
        || found[0].request != transport(r)
        || !found[0].invariants.production_required_passed()
    {
        return Err(error("Conflicting raw procurement receipt"));
    }
    let p = &*found[0];
    if p.input_evidence != inputs(w, r)? {
        return Err(error("Procurement prerequisite evidence altered"));
    }
    let history = crate::operation_history::CheckedHistory::new(w)?;
    let mut seen = HashSet::new();
    if p.operations
            .iter()
            .map(operation_evidence)
            .collect::<OpResult<Vec<_>>>()?
            != p.operation_evidence
        || p.operations.iter().any(|o| {
            !seen.insert(&o.event_id)
                || !history.matches(o)
        })
    {
        return Err(error("Procurement history missing or altered"));
    }
    let marker = p
        .operations
        .last()
        .ok_or_else(|| error("Procurement boundary missing"))?;
    if marker.kind != OperationKind::SupplyPhaseBoundary
        || marker.event_id.json() != json!(boundary(w, r))
        || marker.meta.get("registry_sha256") != Some(&json!(hash(&p.registry_changes)?))
        || marker.meta.get("decisions_sha256") != Some(&json!(hash(&json!(p.decisions))?))
        || marker.meta.get("policies_sha256") != Some(&json!(hash(&json!(r.policies))?))
        || marker.meta.get("date") != Some(&json!(p.date))
    {
        return Err(error("Procurement state/policy evidence altered"));
    }
    Ok(Some(p.clone()))
}
fn raw_funding(s: &OperationSession, state: &ExternalId) -> OpResult<f64> {
    let key = crate::needs::js_string(&state.json());
    let field = |p: &ProductionPolicies| {
        p.extra
            .get("state_budgets")
            .and_then(|v| v.get(&key))
            .and_then(|v| v.get("raw_procurement"))
            .filter(|v| !v.is_null())
            .cloned()
    };
    let n = field(&s.policies)
        .or_else(|| field(&s.base.production_policies()))
        .map(|v| {
            v.as_f64()
                .ok_or_else(|| error("Invalid raw procurement funding"))
        })
        .transpose()?
        .unwrap_or(100.);
    if !n.is_finite() || !(0. ..=100.).contains(&n) {
        return Err(error("Invalid raw procurement funding"));
    }
    Ok(n)
}
struct Request {
    order: MarketOrder,
    state: ExternalId,
    price: f64,
    amount: Money,
}
pub fn prepare_raw_procurement(
    w: &dyn WorldRead,
    r: &BudgetRequest,
) -> OpResult<PreparedRawProcurement> {
    prepare_raw_procurement_guarded(w,r,None)
}
fn prepare_raw_procurement_guarded(
    w: &dyn WorldRead,
    r: &BudgetRequest,
    local:Option<&crate::world_transaction::WorldTransaction<'_>>,
) -> OpResult<PreparedRawProcurement> {
    if let Some(receipt) = replay(w, r)? {
        return Ok(PreparedRawProcurement {
            reads: None,
            state: None,
            receipt,
            request: r.clone(),
        });
    }
    if r.expected_revision != w.campaign().revision || r.period.is_empty() {
        return Err(error("Invalid raw procurement revision/period"));
    }
    let input_evidence = inputs(w, r)?;
    if w.income_phase_receipts().iter().any(|p| {
        p.request.period == r.period
            && p.stage == kingmaker_core::income::IncomeStage::ClosingIncome
    }) {
        return Err(error("Procurement must precede closing income"));
    }
    if w.committed_operations().iter().any(|o| {
        o.event_id.json() == json!(boundary(w, r))
            || crate::needs::js_string(&o.event_id.json())
                .starts_with(&format!("state-procurement-period:[\"{}\",", r.period))
    }) {
        return Err(error("Orphan raw procurement history"));
    }
    let raw = w.production_catalog()
        .as_ref()
        .into_iter()
        .flat_map(|c| &c.methods)
        .filter(|m| m.kind == "rgo")
        .map(|m| m.output_goods_id.clone())
        .collect::<HashSet<_>>();
    let owners = w.resource_operations()
        .iter()
        .filter_map(|e| e.base.owner_id.clone())
        .collect::<HashSet<_>>();
    let mut held = HashMap::<ExternalId, f64>::new();
    for h in &w.goods_reservations() {
        if h.status == "held" {
            *held.entry(h.stock_id.clone()).or_default() += h.quantity;
        }
    }
    let mut allowed = HashMap::new();
    let mut groups = Vec::<Vec<Request>>::new();
    let mut positions = HashMap::new();
    let mut diagnostics = vec![];
    for offer in &w.sale_offers() {
        let Some(stock) = w.stockpiles()
            .iter()
            .find(|s| s.external_id == offer.stock_id)
        else {
            continue;
        };
        let currency = offer.extra.get("currency_id").and_then(|v| actor(v).ok());
        if offer.seller_id != stock.owner_id
            || offer.goods_id != stock.goods.external_id()
            || offer.seller_purpose.as_ref().is_none_or(|p| p.is_empty())
            || stock.unit.as_ref() != offer.extra.get("unit")
            || currency.is_none()
            || stock
                .currency_id
                .as_ref()
                .is_some_and(|c| currency.as_ref() != Some(c))
        {
            diagnostics.push(json!({"stock_id":stock.external_id,"offer_id":offer.id,"reason":"sale_title_or_payment_terms_mismatch"}));
            continue;
        }
        let state = w.territory()
            .iter()
            .find(|p| p.province_id == stock.province_id)
            .and_then(|p| p.extra.get("owner_state_id"))
            .filter(|v| crate::needs::js_truthy(v))
            .map(actor)
            .transpose()?;
        let Some(state) = state else { continue };
        let status = offer.extra.get("status").and_then(Value::as_str);
        if !raw.contains(&stock.goods.external_id())
            || !owners.contains(&stock.owner_id)
            || !matches!(status, Some("approved" | "source"))
            || !offer
                .extra
                .get("source")
                .is_some_and(crate::needs::js_truthy)
            || status == Some("approved")
                && !offer
                    .extra
                    .get("approval_ref")
                    .is_some_and(crate::needs::js_truthy)
        {
            continue;
        }
        let q = (stock.quantity - held.get(&stock.external_id).copied().unwrap_or(0.)).max(0.);
        if !q.is_finite() {
            return Err(error("Invalid raw stock quantity"));
        }
        if q <= 1e-9 {
            continue;
        }
        let buyer = treasury(&state);
        let good = stock.goods.external_id();
        let Some(facility) = w.storage_facilities().iter().find(|f| {
            f.extra
                .get("state_raw_warehouse")
                .is_some_and(crate::needs::js_truthy)
                && f.owner_id == buyer
                && f.province_id == stock.province_id
                && f.goods.contains_key(&crate::needs::js_string(&good.json()))
        }) else {
            diagnostics.push(json!({"id":format!("state-procurement-stock:{}",crate::needs::js_string(&stock.external_id.json())),"stock_id":stock.external_id,"state_id":state,"reason":"state_raw_storage_unavailable"}));
            continue;
        };
        let currency = offer
            .extra
            .get("currency_id")
            .filter(|v| !v.is_null())
            .map(actor)
            .transpose()?;
        let mut order = MarketOrder {
            id: format!(
                "state-raw:{}:{}",
                r.period,
                offer
                    .id
                    .as_ref()
                    .map(|id| crate::needs::js_string(&id.json()))
                    .unwrap_or_else(|| "undefined".into())
            ),
            owner_id: buyer.clone(),
            pop_id: None,
            province_id: stock.province_id,
            currency_id: currency.clone(),
            purpose: "operating_capital".into(),
            goods_id: good,
            quantity: q,
            priority: "production_inputs".into(),
            consume: false,
            need_id: None,
            maintenance: false,
            facility_id: Some(facility.id.clone()),
            food_alternatives: vec![],
            food_gross_quantity: None,
            military_food_substitution: None,
            extra: Default::default(),
        };
        order.extra.insert("state_id".into(), state.json());
        let Some(price) = crate::market_policy::MarketPolicy::quote(w, &order) else {
            diagnostics.push(json!({"id":format!("state-procurement-stock:{}",crate::needs::js_string(&stock.external_id.json())),"stock_id":stock.external_id,"reason":"unknown_price_or_currency"}));
            continue;
        };
        let currency = currency.ok_or_else(|| error("Procurement currency missing"))?;
        let amount = Money(
            invoice(q, price)?
                .to_i64()
                .ok_or_else(|| error("Procurement invoice overflow"))?,
        );
        allowed.insert(stock.external_id.clone(), buyer);
        let key = (state.clone(), currency);
        let n = if let Some(&n) = positions.get(&key) {
            n
        } else {
            let n = groups.len();
            positions.insert(key, n);
            groups.push(vec![]);
            n
        };
        groups[n].push(Request {
            order,
            state,
            price,
            amount,
        });
    }
    let mut session = OperationSession::new(w)?;
    session.policies = r.policies.clone();
    session.procurement_sources = Some(allowed);
    let period = format!("{}:state-raw", r.period);
    let market_request = MarketRequest {
        expected_revision: r.expected_revision,
        period: period.clone(),
        stage: MarketStage::ClosingMarket,
        profile: Default::default(),
        priorities: Some(vec!["production_inputs".into()]),
    };
    session.configure_market(&market_request)?;
    let collator = crate::ordering::collator();
    let mut all_orders = vec![];
    let mut decisions = vec![];
    for mut group in groups {
        group.sort_by(|a, b| collator.compare(&a.order.id, &b.order.id));
        let state = group[0].state.clone();
        let currency = group[0].order.currency_id.clone().unwrap();
        let funding = raw_funding(&session, &state)?;
        let free = free_budget(&session, &state, &currency)?;
        let budget = multiply_money(free, funding / 100.)?;
        if budget.0 == 0 {
            diagnostics.push(json!({"id":format!("state-procurement-budget:{}",json!([state,currency])),"state_id":state,"currency_id":currency,"reason":if funding==0.{"state_procurement_disabled"}else{"no_uncommitted_treasury"}}));
            continue;
        }
        let requests = group
            .iter()
            .map(|x| AllocationRequest {
                id: x.order.id.clone().into(),
                amount: x.amount,
            })
            .collect::<Vec<_>>();
        let allocations = allocate_budget(&requests, budget)?;
        let mut orders = vec![];
        for a in allocations {
            if a.paid.0 == 0 {
                continue;
            }
            let x = group
                .iter()
                .find(|x| ExternalId::from(x.order.id.clone()) == a.id)
                .ok_or_else(|| error("Procurement allocation lost"))?;
            let mut order = x.order.clone();
            let mut low = 0.;
            let mut high = order.quantity * a.paid.0 as f64 / x.amount.0 as f64;
            if invoice(high, x.price)? > num_bigint::BigInt::from(a.paid.0) {
                for _ in 0..64 {
                    let mid = (low + high) / 2.;
                    if mid == low || mid == high {
                        break;
                    }
                    if invoice(mid, x.price)? <= num_bigint::BigInt::from(a.paid.0) {
                        low = mid;
                    } else {
                        high = mid;
                    }
                }
                high = low;
            }
            order.quantity = high;
            orders.push(order);
        }
        let marker: EconomicOperation = serde_json::from_value(
            json!({"event_id":format!("state-procurement-period:{}",json!([r.period,state,currency])),"type":"state_procurement_period","state_id":state,"currency_id":currency}),
        )?;
        crate::shipping_operations::stage_shipping_operations(
            &mut session,
            std::slice::from_ref(&marker),
        )?;
        let change = crate::market_clearing::plan_foundation_market(
            &mut session,
            &period,
            &orders,
            &["production_inputs".into()],
        )?;
        all_orders.extend(orders.clone());
        decisions.push(json!({"state_id":state,"currency_id":currency,"free_budget_minor":free,"funding":funding,"budget_minor":budget,"orders":orders}));
        diagnostics.extend(
            change
                .operations
                .iter()
                .filter(|o| o.kind == OperationKind::Blocked)
                .map(|o| json!(o)),
        );
    }
    let registry_changes = serde_json::to_value(session.registry_changes())?;
    let marker: EconomicOperation = serde_json::from_value(
        json!({"event_id":boundary(w,r),"type":"supply_phase_boundary","period":r.period,"stage":"state_procurement","date":GameDate::from(w.campaign()),"registry_sha256":hash(&registry_changes)?,"decisions_sha256":hash(&json!(decisions))?,"policies":r.policies,"policies_sha256":hash(&json!(r.policies))?}),
    )?;
    crate::shipping_operations::stage_shipping_operations(
        &mut session,
        std::slice::from_ref(&marker),
    )?;
    let operations = session.operations().to_vec();
    let audit = MarketPhaseReceipt {
        campaign_id: w.campaign().campaign_id.clone(),
        request: market_request,
        effective_priorities: vec!["production_inputs".into()],
        frozen_needs: vec![],
        orders: all_orders,
        operations: operations.clone(),
        maintenance_receipts: vec![],
        consumption: vec![],
        diagnostics: vec![],
        registry_changes: session.registry_changes(),
        invariants: Default::default(),
    };
    let invariants = crate::market_invariants::check_session_market_operations(&session, &audit);
    if !invariants.production_required_passed() {
        return Err(error("State procurement market audit failed"));
    }
    let changes = session.finish()?;
    let mut state=local.map_or_else(||crate::world_transaction::WorldTransaction::new(w),|parent|parent.child());
    state.apply_phase(changes)?;
    let receipt = TransportPhaseReceipt {
        campaign_id: w.campaign().campaign_id.clone(),
        request: transport(r),
        date: GameDate::from(w.campaign()),
        operations,
        operation_evidence: audit
            .operations
            .iter()
            .map(operation_evidence)
            .collect::<OpResult<_>>()?,
        input_evidence,
        registry_changes,
        diagnostics,
        decisions,
        invariants,
    };
    let key=state.transport_phase_receipts().next_key().map_err(crate::operation_session::OperationError)?;
    state.put_transport_phase_receipts(key,receipt.clone())?;
    Ok(PreparedRawProcurement {
        reads: Some(crate::world_transaction::PrepareProof::capture(w,local.map(|parent|parent.local_prepare_token()))?),
        state: Some(state.finish()?),
        receipt,
        request: r.clone(),
    })
}
pub fn apply_raw_procurement(
    w: &mut WorldState,
    p: PreparedRawProcurement,
) -> OpResult<TransportPhaseReceipt> {
    crate::world_transaction::execute_atomic(w,|w|apply_raw_procurement_in(w,p))
}
pub(crate) fn apply_raw_procurement_in(
    w: &mut crate::world_transaction::WorldTransaction<'_>,
    p: PreparedRawProcurement,
) -> OpResult<TransportPhaseReceipt> {
    if let Some(reads) = p.reads {
        reads.validate(w)?;
        w.apply_delta(p
            .state
            .ok_or_else(|| error("Prepared procurement state absent"))?)?;
    } else if replay(w, &p.request)?.as_ref() != Some(&p.receipt) {
        return Err(error("Procurement replay changed"));
    }
    Ok(p.receipt)
}
pub fn execute_raw_procurement(
    w: &mut WorldState,
    r: &BudgetRequest,
) -> OpResult<TransportPhaseReceipt> {
    crate::world_transaction::execute_atomic(w,|w|execute_raw_procurement_in(w,r))
}
pub(crate) fn execute_raw_procurement_in(
    w: &mut crate::world_transaction::WorldTransaction<'_>,
    r: &BudgetRequest,
) -> OpResult<TransportPhaseReceipt> {
    let p = prepare_raw_procurement_guarded(w, r, Some(w))?;
    apply_raw_procurement_in(w, p)
}

