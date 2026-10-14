//! Closed automatic investment phase; no caller-authored contribution operations.
use kingmaker_core::world_read::WorldRead;
use crate::{
    budget_funding::error,
    construction_operations::stage_construction_operations,
    needs::{js_string, js_truthy},
    operation_history::operation_evidence,
    operation_session::{canonical_operation, OpResult, OperationSession},
    transport_validation::{actor, province},
};
use kingmaker_core::{construction::*, operations::*, production::*, *};
use num_bigint::BigInt;
use num_traits::{ToPrimitive, Zero};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
pub struct PreparedInvestment {
    reads: Option<crate::world_transaction::PrepareProof>,
    state: Option<crate::world_transaction::PreparedWorldDelta>,
    receipt: ConstructionPhaseReceipt,
}
fn digest(value: &Value) -> OpResult<String> {
    Ok(Sha256::digest(serde_json::to_vec(value)?)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}
fn money(v: &Value) -> OpResult<Money> {
    let m: Money = serde_json::from_value(v.clone())?;
    if m.0 < 0 {
        return Err(error("Negative investment money"));
    }
    Ok(m)
}
fn integer(v: &Value) -> OpResult<BigInt> {
    js_string(v)
        .parse()
        .map_err(|_| error("Invalid investment integer"))
}
fn gcd(mut a: BigInt, mut b: BigInt) -> BigInt {
    while !b.is_zero() {
        let r = &a % &b;
        a = b;
        b = r;
    }
    a
}
fn cash(
    w: &dyn WorldRead,
    owner: &ExternalId,
    currency: &ExternalId,
    purpose: &str,
) -> OpResult<Money> {
    let address = AccountAddress { owner_id: owner.clone(), currency_id: currency.clone(), purpose: purpose.into() };
    Ok(w.accounts().get(&address).map(|a| a.balance).unwrap_or(Money::ZERO))
}
fn approved(v: &Value) -> bool {
    crate::storage::approved(v["status"].as_str(), v.get("source"), v.get("approval_ref"))
}
fn budget_evidence(w: &dyn WorldRead, r: &ConstructionRequest) -> OpResult<Vec<OperationEvidence>> {
    let receipts: Vec<_> = w.all_budget_opening_receipts().select(|p| p.request.period == r.period)?;
    if receipts.len() != 1
        || receipts[0].campaign_id != w.campaign().campaign_id
        || receipts[0].request.expected_revision != r.expected_revision
    {
        return Err(error("Investment requires actual opening budget"));
    }
    crate::budget_cycle::validate_budget_receipt(w,&receipts[0])?;
    let history = crate::operation_history::CheckedHistory::new(w)?;
    let mut seen = HashSet::new();
    let mut evidence = vec![];
    for op in &receipts[0].operations {
        if !seen.insert(&op.event_id)
            || !history.matches(op)
        {
            return Err(error("Missing or altered investment budget history"));
        }
        evidence.push(operation_evidence(op)?);
    }
    Ok(evidence)
}
fn registry(w: &dyn WorldRead) -> Value {
    json!({"projects":w.construction_projects(),"investments":w.project_investments(),"resource_operations":w.resource_operations(),"employment":w.employment()})
}
fn boundary_id(w: &dyn WorldRead, r: &ConstructionRequest) -> String {
    format!(
        "construction-phase:{}:{}:investments",
        w.campaign().campaign_id, r.period
    )
}
pub(crate) fn replay(w: &dyn WorldRead, r: &ConstructionRequest) -> OpResult<Option<ConstructionPhaseReceipt>> {
    let receipts: Vec<_> = w.all_construction_phase_receipts().select(|p| p.request.period == r.period && p.stage == ConstructionStage::Investments)?;
    if receipts.is_empty() {
        return Ok(None);
    }
    if receipts.len() != 1
        || receipts[0].request != *r
        || receipts[0].campaign_id != w.campaign().campaign_id
    {
        return Err(error("Conflicting investment receipt"));
    }
    let p = &*receipts[0];
    if !p.invariants.all_required_passed() || budget_evidence(w, r)? != p.input_evidence {
        return Err(error("Invalid investment prerequisite evidence"));
    }
    let mut seen = HashSet::new();
    let history = crate::operation_history::CheckedHistory::new(w)?;
    let evidence = p
        .operations
        .iter()
        .map(operation_evidence)
        .collect::<OpResult<Vec<_>>>()?;
    if evidence != p.operation_evidence
        || p.operations.iter().any(|op| {
            !seen.insert(&op.event_id)
                || !history.matches(op)
        })
    {
        return Err(error("Missing or altered investment history"));
    }
    let boundary = p
        .operations
        .last()
        .ok_or_else(|| error("Investment boundary missing"))?;
    if boundary.kind != OperationKind::ConstructionPhaseBoundary
        || boundary.event_id.json() != json!(boundary_id(w, r))
        || boundary.meta.get("registry_sha256") != Some(&json!(digest(&p.registry_changes)?))
        || !boundary.debits.is_empty()
        || !boundary.credits.is_empty()
        || !boundary.stock_changes.is_empty()
    {
        return Err(error("Investment registry history altered"));
    }
    Ok(Some(p.clone()))
}
fn construction(method: &ProductionMethod, currency: &ExternalId) -> Value {
    if let Some(by) = method.extra.get("construction_by_currency") {
        by.get(js_string(&currency.json()))
            .cloned()
            .unwrap_or(json!({"status":"unresolved"}))
    } else {
        method
            .extra
            .get("construction")
            .cloned()
            .unwrap_or(Value::Null)
    }
}
struct ContributionIndex {
    rows: HashMap<ExternalId, Vec<(Value,Value)>>,
    totals: HashMap<ExternalId, HashMap<ExternalId, BigInt>>,
    events: HashSet<String>,
}
impl ContributionIndex {
    fn new(rows: &(impl kingmaker_core::row_store::ReadSequence<Value> + ?Sized)) -> OpResult<Self> {
        let mut out=Self{rows:HashMap::new(),totals:HashMap::new(),events:HashSet::new()};
        for row in rows.iter() {
            crate::run_control::work("investment_history_index")?;
            if let Some(id)=row["event_id"].as_str() {out.events.insert(id.into());}
            if let Ok(project)=actor(&row["project_id"]) {out.rows.entry(project).or_default().push((row["investor_id"].clone(),row["amount_minor"].clone()));}
        }
        Ok(out)
    }
    fn totals(&mut self, project:&ExternalId)->OpResult<&mut HashMap<ExternalId,BigInt>> {
        if !self.totals.contains_key(project) {
            let mut totals=HashMap::new();
            for (investor,amount) in self.rows.remove(project).unwrap_or_default() {
                crate::run_control::work("investment_share_rows")?;
                let n=integer(&amount)?;
                if n<=BigInt::zero() {return Err(error("Invalid actual investment contribution"));}
                *totals.entry(actor(&investor)?).or_default()+=n;
            }
            self.totals.insert(project.clone(),totals);
        }
        Ok(self.totals.get_mut(project).unwrap())
    }
    fn shares(&mut self, project:&ExternalId,budget:&BigInt)->OpResult<Value> {
        shares_from_totals(self.totals(project)?,budget)
    }
    fn add(&mut self, project:&ExternalId, investor:&ExternalId, amount:i64)->OpResult<()> {
        *self.totals(project)?.entry(investor.clone()).or_default()+=BigInt::from(amount);Ok(())
    }
}
fn shares_from_totals(totals:&HashMap<ExternalId,BigInt>,budget:&BigInt)->OpResult<Value> {
    let collator = crate::ordering::collator();
    let mut totals: Vec<_> = totals.iter().collect();
    totals.sort_by(|a, b| collator.compare(&js_string(&a.0.json()), &js_string(&b.0.json())));
    Ok(json!(totals.into_iter().map(|(id,n)|{let d=gcd(n.clone(),budget.clone());json!({"investor_id":id,"numerator":(n/&d).to_string(),"denominator":(budget/&d).to_string()})}).collect::<Vec<_>>()))
}
fn propose(
    w: &mut crate::world_transaction::WorldTransaction<'_>,
    investor: &Value,
    o: &Value,
    method: &ProductionMethod,
    c: &Value,
    region: &Value,
) -> OpResult<ExternalId> {
    let c = o.get("construction").unwrap_or(c);
    let terms = &o["terms"];
    let budget = money(&terms["budget_minor"])?;
    let cost = money(&c["cost_minor"])?;
    let reserve = terms
        .get("operating_reserve_minor")
        .map(money)
        .transpose()?
        .unwrap_or(Money::ZERO);
    let currency = actor(&investor["currency_id"])?;
    let id: ExternalId = format!("private-project:{}", js_string(&o["id"])).into();
    if !approved(o)
        || c["status"] != "ready"
        || !js_truthy(&terms["source"])
        || !terms["version"]
            .as_f64()
            .is_some_and(|v| v >= 1. && v.fract() == 0. && v <= 9007199254740991.)
        || terms["share_basis"] != "cash_contribution"
        || terms["refund_rule"] != "pro_rata_unspent"
        || terms["currency_id"] != currency.json()
        || c["currency_id"] != currency.json()
        || budget.0 <= 0
        || budget.0 < cost.0
        || reserve.0 > budget.0 - cost.0
    {
        return Err(error("Invalid investment project terms or reserve"));
    }
    let place = province(&o["province_id"])?;
    if region.is_null() {return Err(error("Unknown factory region"));}
    if crate::construction_limits::factory_places(w, region)? == 0 {
        return Err(error("Regional factory limit reached"));
    }
    if let Some(reason) = crate::construction_limits::new_factory_limit(w, place)? {
        return Err(error(reason));
    }
    if w.factories().iter().any(|f| {
        f.base.province_id == place
            && f.base.method_id == method.method_id
            && f.status.as_deref() != Some("demolished")
    }) {
        return Err(error("Factory type already exists in province"));
    }
    let mut p = json!({"id":id,"kind":"factory","region_id":region,"province_id":place.0,"status":"awaiting_funding","place_reserved":true,"initiator_id":investor["id"],"method_id":method.method_id,"owner_id":format!("project:{}",js_string(&id.json())),"terms":terms,"funded_minor":"0","shares":[],"acceptances":[{"investor_id":investor["id"],"terms_version":terms["version"]}],"construction":c,"assignments":[]});
    for (source, target, default) in [
        ("assignments", "planned_assignments", json!([])),
        ("staff_transfers", "staff_transfers", json!([])),
        ("future_assignments", "future_assignments", json!([])),
        ("material_facility_ids", "material_facility_ids", json!({})),
        (
            "factory_configuration",
            "factory_configuration",
            Value::Null,
        ),
    ] {
        p[target] = o.get(source).cloned().unwrap_or(default);
    }
    let row:kingmaker_core::production::ConstructionProjectLabor=serde_json::from_value(p)?;
        w.put_construction_projects(row.id.clone(),row)?;
    Ok(id)
}
fn contribute(
    w: &mut crate::world_transaction::WorldTransaction<'_>,
    r: &ConstructionRequest,
    id: &ExternalId,
    investor: &Value,
    history: &mut ContributionIndex,
) -> OpResult<Option<(EconomicOperation, Value)>> {
    let p = w.construction_projects().get(id).expect("selected project").clone();
    let terms = &p.extra["terms"];
    let budget = money(&terms["budget_minor"])?;
    let funded = money(&p.extra["funded_minor"])?;
    if funded.0 > budget.0 {
        return Err(error("Project overfunded"));
    }
    let owner = actor(&investor["owner_id"])?;
    let currency = actor(&investor["currency_id"])?;
    let purpose = investor["account_purpose"]
        .as_str()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| error("Investor account purpose unresolved"))?;
    if terms["currency_id"] != currency.json() {
        return Err(error("Contribution currency mismatch"));
    }
    let keep = investor["policy"]
        .get("cash_reserve_minor")
        .map(money)
        .transpose()?
        .unwrap_or(Money::ZERO);
    let available = (cash(w, &owner, &currency, purpose)?.0 - keep.0).max(0);
    let limit = investor["policy"]
        .get("contribution_limit_minor")
        .filter(|v| !v.is_null())
        .map(money)
        .transpose()?
        .unwrap_or(Money(available));
    let amount = (budget.0 - funded.0).min(available).min(limit.0);
    if amount <= 0 {
        return Ok(None);
    }
    let reserve = terms
        .get("operating_reserve_minor")
        .map(money)
        .transpose()?
        .unwrap_or(Money::ZERO);
    let cost = money(&p.extra["construction"]["cost_minor"])?;
    if reserve.0 > budget.0 - cost.0 {
        return Err(error("Invalid operating reserve"));
    }
    let b = BigInt::from(budget.0);
    let protected =
        crate::money_math::round_ratio(&(BigInt::from(funded.0 + amount) * reserve.0), &b)
            .map_err(|e| error(&e.to_string()))?
            - crate::money_math::round_ratio(&(BigInt::from(funded.0) * reserve.0), &b)
                .map_err(|e| error(&e.to_string()))?;
    let protected = protected
        .to_i64()
        .ok_or_else(|| error("Protected reserve overflow"))?;
    let project_owner = actor(&p.extra["owner_id"])?;
    let event_id = format!(
        "investment:{}:{}:{}",
        r.period,
        js_string(&p.id.json()),
        js_string(&investor["id"])
    );
    if history.events.contains(&event_id) {return Err(error("Orphan investment contribution"));}
    let row = json!({"id":event_id,"event_id":event_id,"project_id":p.id,"investor_id":investor["id"],"owner_id":owner,"source_account_purpose":purpose,"currency_id":currency,"amount_minor":amount.to_string(),"terms_version":terms["version"]});
    let mut credits = vec![
        json!({"owner_id":project_owner,"currency_id":currency,"purpose":"construction_escrow","amount_minor":(amount-protected).to_string()}),
    ];
    if protected > 0 {
        credits.push(json!({"owner_id":project_owner,"currency_id":currency,"purpose":"factory_startup_reserve","amount_minor":protected.to_string()}));
    }
    let op: EconomicOperation = serde_json::from_value(
        json!({"event_id":event_id,"type":"capitalist_investment","project_id":p.id,"investor_id":investor["id"],"debits":[{"owner_id":owner,"currency_id":currency,"purpose":purpose,"amount_minor":amount.to_string()}],"credits":credits,"income_recognized":false}),
    )?;
    let mut s = OperationSession::new(w)?;
    stage_construction_operations(&mut s, std::slice::from_ref(&op))?;
    let changes = s.finish()?;
    w.apply_phase(changes)?;
    let key=w.project_investments().next_key().map_err(crate::operation_session::OperationError)?;
        w.put_project_investments(key,row)?;
    history.events.insert(event_id.clone());
    history.add(id,&actor(&investor["id"])?,amount)?;
    let ownership=history.shares(id,&b)?;
    let mut p=w.construction_projects().get(id).expect("selected project").clone();
    p.extra.insert(
        "funded_minor".into(),
        json!((funded.0 + amount).to_string()),
    );
    p.status = if funded.0 + amount == budget.0 {
        "funded"
    } else {
        "awaiting_funding"
    }
    .into();
    p.extra.insert("shares".into(), ownership);
    if reserve.0 > 0 {
        p.extra
            .insert("protected_operating_reserve".into(), json!(true));
    }
    let acceptance = json!({"investor_id":investor["id"],"terms_version":terms["version"]});
    let acceptance_list = p
        .extra
        .get_mut("acceptances")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| error("Project acceptances missing"))?;
    if !acceptance_list.contains(&acceptance) {
        acceptance_list.push(acceptance);
    }
    w.put_construction_projects(p.id.clone(),p)?;
    Ok(Some((
        op,
        json!({"investor_id":investor["id"],"project_id":id,"amount_minor":amount.to_string()}),
    )))
}
pub fn prepare_automatic_investments(
    w: &dyn WorldRead,
    r: &ConstructionRequest,
) -> OpResult<PreparedInvestment> {
    prepare_automatic_investments_guarded(w,r,None)
}
fn prepare_automatic_investments_guarded(
    w: &dyn WorldRead,
    r: &ConstructionRequest,
    local:Option<&crate::world_transaction::WorldTransaction<'_>>,
) -> OpResult<PreparedInvestment> {
    if let Some(receipt) = replay(w, r)? {
        return Ok(PreparedInvestment {
            reads: None,
            state: None,
            receipt,
        });
    }
    if r.period.is_empty() || r.expected_revision != w.campaign().revision {
        return Err(error("Invalid investment revision/period"));
    }
    let input_evidence = budget_evidence(w, r)?;
    if w.production_phase_receipts()
        .iter()
        .any(|p| p.request.period == r.period)
        || w.market_phase_receipts()
            .iter()
            .any(|p| p.request.period == r.period)
    {
        return Err(error("Investments must precede production and market"));
    }
    let boundary = boundary_id(w, r);
    if w.committed_operations().iter().any(|op| {
        op.event_id.json() == json!(boundary)
            || js_string(&op.event_id.json()).starts_with(&format!("investment:{}:", r.period))
    }) {
        return Err(error("Orphan investment history"));
    }
    let mut state=local.map_or_else(||crate::world_transaction::WorldTransaction::new(w),|parent|parent.child());
    let mut ops = vec![];
    let mut decisions = vec![];
    let mut diagnostics = vec![];
    let opportunities = crate::investment_assessment::assess_investment_opportunities(&state, r)?
        .into_iter()
        .filter(approved)
        .collect::<Vec<_>>();
    let mut history=ContributionIndex::new(&state.project_investments())?;
    let catalog=w.production_catalog().as_ref().ok_or_else(||error("Construction catalog missing"))?;
    let mut methods=HashMap::new();
    for (i,m) in catalog.methods.iter().enumerate() {crate::run_control::work("investment_method_index")?;methods.entry(m.method_id.json().to_string()).or_insert(i);}
    let mut projects=HashMap::new();
    for p in state.construction_projects().values() {
        crate::run_control::work("investment_project_index")?;
        if p.status!="cancelled" {if let (Some(method),Some(place))=(p.extra.get("method_id"),p.extra.get("province_id").and_then(|v|province(v).ok())) {
            projects.entry((method.to_string(),place)).or_insert_with(||p.id.clone());
        }}
    }
    let mut territories=HashMap::new();
    for (key,t) in state.territory().keyed() {crate::run_control::work("investment_territory_index")?;territories.entry(t.province_id).or_insert(key);}
    // Static rejection is evaluated once; malformed margins stay in order and are
    // rejected only if execution reaches them, as before.
    let mut by_currency:HashMap<String,Vec<&Value>>=HashMap::new();
    for o in &opportunities {
        crate::run_control::work("investment_offer_index")?;
        if o["demand"].as_f64().unwrap_or(0.)<=0. {continue;}
        if let Ok(margin)=integer(&o["margin_minor"]) {
            if margin<=BigInt::zero() || o["workforce_available"].as_f64().unwrap_or(0.)<=0. || o["inputs_accessible"]!=true {continue;}
        }
        by_currency.entry(o.get("currency_id").unwrap_or(&o["terms"]["currency_id"]).to_string()).or_default().push(o);
    }
    for investor in w.capitalist_entities().values() {
        crate::run_control::work("investment_investor")?;
        let owner = actor(&investor["owner_id"])?;
        let currency = actor(&investor["currency_id"])?;
        let purpose = investor["account_purpose"]
            .as_str()
            .ok_or_else(|| error("Investor purpose missing"))?;
        if cash(&state, &owner, &currency, purpose)?.0 <= 0 {
            continue;
        }
        for o in by_currency.get(&currency.json().to_string()).into_iter().flatten() {
            crate::run_control::work("investment_match")?;
            if o.get("currency_id").unwrap_or(&o["terms"]["currency_id"]) != &currency.json()
                || o["demand"].as_f64().unwrap_or(0.) <= 0.
                || integer(&o["margin_minor"])? <= BigInt::zero()
                || o["workforce_available"].as_f64().unwrap_or(0.) <= 0.
                || o["inputs_accessible"] != true
            {
                continue;
            }
            let method=&catalog.methods[*methods.get(&o["method_id"].to_string()).ok_or_else(||error("Unknown factory method"))?];
            let c = construction(&method, &currency);
            if method.kind != "factory"
                || method.status != "ready"
                || c["status"] != "ready"
                || c["currency_id"] != currency.json()
                || method.technology.iter().any(|t| {
                    !investor["implemented_technologies"]
                        .as_array()
                        .is_some_and(|v| v.contains(&t.json()))
                })
            {
                continue;
            }
            let place = province(&o["province_id"])?;
            let project_key=(o["method_id"].to_string(),place);
            let id = if let Some(id)=projects.get(&project_key) {
                if state.construction_projects().get(id).expect("indexed project").status!="awaiting_funding" {continue;}
                id.clone()
            } else {
                let Some(key)=territories.get(&place) else {continue;};
                let t=state.territory().get(key).expect("indexed territory");
                if crate::construction_limits::factory_places(
                    &state,
                    &t.region_id
                        .as_ref()
                        .map(ExternalId::json)
                        .unwrap_or(Value::Null),
                )? == 0
                {
                    continue;
                }
                if o.get("terms").is_none() {
                    diagnostics.push(json!({"id":o["id"],"reason":"project_terms_unresolved"}));
                    continue;
                }
                let region=t.region_id.as_ref().map(ExternalId::json).unwrap_or(Value::Null);
                let id=propose(&mut state, investor, o, method, &c,&region)?;
                projects.insert(project_key,id.clone());id
            };
            if let Some((op, decision)) = contribute(&mut state, r, &id, investor,&mut history)? {
                ops.push(op);
                decisions.push(decision);
            }
            break;
        }
    }
    let (staff_ops, staff_blocked) = crate::investment_staff::activate_staff(&mut state, r)?;
    ops.extend(staff_ops);
    diagnostics.extend(staff_blocked);
    let registry_changes = registry(&state);
    let op: EconomicOperation = serde_json::from_value(
        json!({"event_id":boundary,"type":"construction_phase_boundary","period":r.period,"stage":"investments","registry_sha256":digest(&registry_changes)?}),
    )?;
    let mut session = OperationSession::new(&state)?;
    stage_construction_operations(&mut session, std::slice::from_ref(&op))?;
    let changes = session.finish()?;
    state.apply_phase(changes)?;
    ops.push(op);
    let invariants = crate::investment_invariants::verify_investment(w, &state, &ops)?;
    let receipt = ConstructionPhaseReceipt {
        campaign_id: w.campaign().campaign_id.clone(),
        request: r.clone(),
        stage: ConstructionStage::Investments,
        operation_evidence: ops
            .iter()
            .map(operation_evidence)
            .collect::<OpResult<_>>()?,
        input_evidence,
        operations: ops,
        registry_changes,
        decisions,
        diagnostics,
        invariants,
    };
    let key=state.construction_phase_receipts().next_key().map_err(crate::operation_session::OperationError)?;
    state.put_construction_phase_receipts(key,receipt.clone())?;
    Ok(PreparedInvestment {
        reads: Some(crate::world_transaction::PrepareProof::capture(w,local.map(|parent|parent.local_prepare_token()))?),
        state: Some(state.finish()?),
        receipt,
    })
}
pub fn apply_automatic_investments(
    w: &mut WorldState,
    p: PreparedInvestment,
) -> OpResult<ConstructionPhaseReceipt> {
    crate::world_transaction::execute_atomic(w,|w|apply_automatic_investments_in(w,p))
}
pub(crate) fn apply_automatic_investments_in(
    w: &mut crate::world_transaction::WorldTransaction<'_>,
    p: PreparedInvestment,
) -> OpResult<ConstructionPhaseReceipt> {
    if let Some(reads) = p.reads {
        reads.validate(w)?;
        w.apply_delta(p.state.ok_or_else(|| error("Investment state missing"))?)?;
    } else if replay(w, &p.receipt.request)?.as_ref() != Some(&p.receipt) {
        return Err(error("Prepared investment replay changed"));
    }
    Ok(p.receipt)
}
pub fn execute_automatic_investments(
    w: &mut WorldState,
    r: &ConstructionRequest,
) -> OpResult<ConstructionPhaseReceipt> {
    crate::world_transaction::execute_atomic(w,|w|execute_automatic_investments_in(w,r))
}
pub(crate) fn execute_automatic_investments_in(
    w: &mut crate::world_transaction::WorldTransaction<'_>,
    r: &ConstructionRequest,
) -> OpResult<ConstructionPhaseReceipt> {
    let p = prepare_automatic_investments_guarded(w, r, Some(w))?;
    apply_automatic_investments_in(w, p)
}

#[cfg(test)]
#[path="../tests/internal/investment_indexes.rs"]
mod index_tests;
