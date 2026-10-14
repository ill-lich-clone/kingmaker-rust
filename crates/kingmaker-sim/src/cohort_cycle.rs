//! Closed native cohort splits. No public operation payload can grant title/issuance authority.
use kingmaker_core::world_read::WorldRead;
use crate::{
    budget_funding::error,
    cohort_assets::{self, AssetPlan},
    labor_roster::{build_labor_roster, LaborEntry},
    operation_session::*,
};
use kingmaker_core::{market::*, mobility::*, operations::*, production::Extra, *};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

fn child_id(event: &str) -> ExternalId {
    let hash = Sha256::digest(event.as_bytes());
    let hex: String = hash[..12].iter().map(|b| format!("{b:02x}")).collect();
    format!("pop-cohort:{hex}").into()
}
fn linked(d: &Obligation, id: &ExternalId) -> bool {
    &d.payer_id == id || &d.payee_id == id || d.pop_id.as_ref() == Some(id)
}
#[derive(PartialEq)]
struct CohortReads {
    campaign: CampaignHeader,
    parent: PopRecord,
    accounts: Vec<AccountBalance>,
    stocks: Vec<StockpileRecord>,
    claims: Vec<Obligation>,
    labor: Vec<LaborEntry>,
    unemployment: Option<Value>,
    needs: Vec<(usize, NeedProfileRef)>,
    territory: Vec<(usize, MarketTerritory)>,
    provinces: Vec<ProvinceRecord>,
    assets: AssetPlan,
    child_exists: bool,
}
impl CohortReads {
    fn capture(
        w: &dyn WorldRead,
        parent: &PopRecord,
        child: &ExternalId,
        full: bool,
        target: &SplitTarget,
        indexed_labor: Option<&[LaborEntry]>,
    ) -> OpResult<Self> {
        let id = &parent.external_id;
        let mut labor: Vec<_> = if let Some(entries) = indexed_labor {
            entries.to_vec()
        } else {
            build_labor_roster(w)?
                .by_employment
                .into_values()
                .filter(|e| &e.pop_id == id)
                .collect()
        };
        labor.sort_by_key(|e| serde_json::to_string(&e.employment_id).unwrap());
        Ok(Self {
            campaign: w.campaign().clone(),
            parent: parent.clone(),
            accounts: w.accounts().values()
                .filter(|a| &a.owner_id == id)
                .cloned()
                .collect(),
            stocks: w.stockpiles().values()
                .filter(|s| &s.owner_id == id)
                .cloned()
                .collect(),
            claims: w.obligations().values()
                .filter(|d| linked(d, id))
                .cloned()
                .collect(),
            labor,
            unemployment: w.pop_mobility()
                .as_ref()
                .and_then(|v| v.get("unemployment"))
                .and_then(|v| v.get(cohort_assets::text(id)))
                .cloned(),
            needs: w.needs()
                .iter()
                .flat_map(|n| n.profile_refs.iter())
                .enumerate()
                .filter(|(_, r)| &r.pop_id == id)
                .map(|(position, r)| (position, r.clone()))
                .collect(),
            territory: w.territory().values()
                .enumerate()
                .filter(|(_, t)| {
                    t.province_id == parent.province_id || target.province_id == Some(t.province_id)
                })
                .map(|(position, t)| (position, t.clone()))
                .collect(),
            provinces: w.provinces().values().cloned().collect::<Vec<_>>(),
            assets: cohort_assets::plan_assets(w, parent, child, full)?,
            child_exists: w.pops().values().any(|p| &p.external_id == child),
        })
    }
}
pub struct PreparedPopSplit {
    reads: Option<CohortReads>,
    money: Option<PreparedChanges>,
    receipt: CohortSplitReceipt,
    parent: Option<PopRecord>,
    child: Option<PopRecord>,
    views: Vec<EnterpriseRecord>,
    unemployment: Option<(Value, Value)>,
    needs: Vec<(usize, NeedProfileRef, NeedProfileRef)>,
    territory: Vec<(usize, MarketTerritory)>,
}
fn replay(w: &dyn WorldRead, r: &CohortSplitRequest) -> OpResult<Option<CohortSplitReceipt>> {
    let rows: Vec<_> = w.all_mobility_cohort_receipts().select(|p| p.request.event_id == r.event_id)?;
    if rows.is_empty() {
        return Ok(None);
    }
    if rows.len() != 1 || rows[0].campaign_id != w.campaign().campaign_id || rows[0].request != *r {
        return Err(error("Conflicting cohort split request"));
    }
    let p = &*rows[0];
    let history = crate::operation_history::CheckedHistory::new(w)?;
    for op in &p.operations {
        if !history.matches(op) {
            return Err(error("Missing/conflicting cohort split history"));
        }
    }
    if !w.pops().values()
        .any(|p| Some(&p.external_id) == rows[0].child_id.as_ref())
    {
        return Err(error("Cohort receipt child missing"));
    }
    Ok(Some(p.clone()))
}
/// Phase replay still requires the independent split evidence after later lawful state changes.
pub(crate) fn validate_cohort_receipts(
    w: &dyn WorldRead,
    operations: &[EconomicOperation],
    revision: u64,
) -> OpResult<()> {
    let mut receipts = HashMap::<_, Vec<_>>::new();
    for receipt in w.mobility_cohort_receipts().values() {
        receipts
            .entry(receipt.request.event_id.as_str())
            .or_default()
            .push(receipt);
    }
    let pops: HashSet<_> = w.pops().values().map(|p| &p.external_id).collect();
    let ops: HashMap<_, _> = operations.iter().map(|op| (&op.event_id, op)).collect();
    if ops.len() != operations.len() {
        return Err(error("Duplicate phase operation evidence"));
    }
    for op in operations
        .iter()
        .filter(|op| op.kind == OperationKind::PopCohortSplit)
    {
        let event = crate::operation_session::external_text(&op.event_id);
        let rows = receipts
            .get(event.as_str())
            .ok_or_else(|| error("Required cohort receipt missing"))?;
        if rows.len() != 1 {
            return Err(error("Required cohort receipt duplicated"));
        }
        let receipt = &*rows[0];
        let child = child_id(&event);
        if receipt.campaign_id != w.campaign().campaign_id
            || receipt.request.expected_revision != revision
            || receipt.child_id.as_ref() != Some(&child)
            || receipt.blocked_reason.is_some()
            || receipt.people_moved == 0
            || receipt.workforce_moved == 0
            || receipt.workforce_moved != receipt.request.workforce_count
            || op.meta.get("pop_id") != Some(&receipt.request.pop_id.json())
            || op.meta.get("child_pop_id") != Some(&child.json())
            || !pops.contains(&child)
            || !receipt.operations.iter().any(|required| required == op)
            || receipt
                .operations
                .iter()
                .any(|required| ops.get(&required.event_id).copied() != Some(required))
        {
            return Err(error(
                "Required cohort receipt conflicts with phase evidence",
            ));
        }
    }
    Ok(())
}
fn blank(w: &dyn WorldRead, r: &CohortSplitRequest) -> PreparedPopSplit {
    PreparedPopSplit {
        reads: None,
        money: None,
        receipt: CohortSplitReceipt {
            campaign_id: w.campaign().campaign_id.clone(),
            request: r.clone(),
            child_id: None,
            people_moved: 0,
            workforce_moved: 0,
            blocked_reason: None,
            operations: vec![],
        },
        parent: None,
        child: None,
        views: vec![],
        unemployment: None,
        needs: vec![],
        territory: vec![],
    }
}
fn split_history(rows: Option<&Value>, workers: u64) -> OpResult<(Value, Value)> {
    let mut rows = match rows {
        None => vec![],
        Some(Value::Array(rows)) => rows.clone(),
        _ => return Err(error("Invalid unemployment cohorts")),
    };
    for row in &rows {
        if !row["workers"]
            .as_f64()
            .is_some_and(|n| n.is_finite() && n >= 0.)
            || row["weeks"].as_u64().is_none()
        {
            return Err(error("Invalid unemployment history"));
        }
    }
    rows.sort_by_key(|r| std::cmp::Reverse(r["weeks"].as_u64().unwrap()));
    let mut remaining = workers as f64;
    let (mut stayed, mut moving) = (vec![], vec![]);
    for row in rows {
        let count = row["workers"].as_f64().unwrap();
        let take = remaining.min(count);
        if take > 0. {
            let mut next = row.clone();
            next["workers"] = json!(take);
            moving.push(next)
        }
        if count > take {
            let mut next = row;
            next["workers"] = json!(count - take);
            stayed.push(next)
        }
        remaining -= take;
    }
    if remaining > 0. {
        moving.push(json!({"workers":remaining,"weeks":0}))
    }
    Ok((json!(stayed), json!(moving)))
}
fn stock_create(s: &StockpileRecord) -> StockCreate {
    let mut extra = s.extra.clone();
    extra.insert("inventory_purpose".into(), json!(s.purpose));
    if let Some(id) = &s.beneficiary_pop_id {
        extra.insert("beneficiary_pop_id".into(), id.json());
    }
    StockCreate {
        owner_id: s.owner_id.clone(),
        province_id: s.province_id,
        goods_id: s.goods.external_id(),
        unit: s.unit.clone().unwrap_or(Value::Null),
        facility_id: s.facility_id.clone(),
        currency_id: s.currency_id.clone(),
        inventory_origin: s.inventory_origin.clone().unwrap_or_default(),
        method_kind: s.method_kind.clone(),
        extra,
    }
}
fn prepare_split(
    w: &dyn WorldRead,
    r: &CohortSplitRequest,
    indexed_labor: Option<&[LaborEntry]>,
) -> OpResult<PreparedPopSplit> {
    if r.event_id.is_empty() || r.expected_revision != w.campaign().revision {
        return Err(error("Invalid cohort event/revision"));
    }
    if let Some(receipt) = replay(w, r)? {
        let mut p = blank(w, r);
        p.receipt = receipt;
        return Ok(p);
    }
    if w.pop_mobility().as_ref().is_some_and(|v| {
        !v.is_object()
            || v.get("unemployment")
                .is_some_and(|u| !u.is_object() && !u.is_null() && !crate::needs::undefined(u))
    }) {
        return Err(error("Invalid mobility registry"));
    }
    let p = w.pops().values()
        .find(|p| p.external_id == r.pop_id)
        .ok_or_else(|| error("Unknown source POP"))?;
    if p.size == 0
        || !p.workforce.is_finite()
        || p.workforce.fract() != 0.
        || p.workforce <= 0.
        || p.workforce > f64::from(p.size)
        || r.workforce_count == 0
        || r.workforce_count > 9_007_199_254_740_991
    {
        return Err(error("Invalid cohort population/workforce"));
    }
    if r.workforce_count as f64 > p.workforce {
        return Err(error("Split exceeds free workforce"));
    }
    let moved = (u128::from(p.size) * u128::from(r.workforce_count) / p.workforce as u128) as u32;
    let full = moved == p.size;
    let childid = child_id(&r.event_id);
    let reads = CohortReads::capture(w, p, &childid, full, &r.target, indexed_labor)?;
    let occupied: f64 = reads.labor.iter().map(|e| e.fte).sum();
    if r.workforce_count as f64 > p.workforce - occupied + 1e-9 {
        return Err(error("Split exceeds free workforce"));
    }
    let mut groups = HashMap::<ExternalId, f64>::new();
    for e in &reads.labor {
        let key = e.worker_group_id.as_ref().unwrap_or(&e.employment_id);
        let n = groups.entry(key.clone()).or_default();
        *n = n.max(e.workers_count);
    }
    let blocked = if r.workforce_count as f64 > p.workforce - groups.values().sum::<f64>() + 1e-9 {
        Some("unassigned_physical_workers_unresolved")
    } else if matches!(
        p.pop_type_id.as_str(),
        Some("aristocrats" | "capitalists" | "slaves")
    ) {
        Some("protected_pop_rights")
    } else {
        reads.assets.blocked.as_deref()
    };
    if let Some(reason) = blocked {
        let mut result = blank(w, r);
        result.receipt.blocked_reason = Some(reason.into());
        return Ok(result);
    }
    if reads.child_exists {
        return Err(error("Cohort ID exists without applied split"));
    }
    let mut result = blank(w, r);
    let (mut parent, mut child) = (p.clone(), p.clone());
    child.external_id = childid.clone();
    child.size = moved;
    child.workforce = r.workforce_count as f64;
    child.employed = 0.;
    child.unemployed = child.workforce;
    child.account_id = None;
    child.account_status = "unbound".into();
    child
        .extra
        .insert("parent_pop_id".into(), p.external_id.json());
    if let Some(id) = &r.target.pop_type_id {
        child.pop_type_id = id.clone();
        child.pop_type = cohort_assets::text(id);
    }
    if let Some(id) = r.target.province_id {
        child.province_id = id
    }
    if let Some(id) = &r.target.state_id {
        child.state_id = Some(id.clone())
    }
    if let Some(id) = &r.target.currency_id {
        child.currency_id = Some(id.clone())
    }
    parent.size -= moved;
    parent.workforce -= child.workforce;
    parent.employed = occupied;
    parent.unemployed = parent.workforce - occupied;
    if parent.size == 0 {
        parent.extra.insert("status".into(), json!("dormant"));
    }
    if let Some(n) = p
        .extra
        .get("literate_count")
        .filter(|v| !v.is_null() && !crate::needs::undefined(v))
    {
        let n = n
            .as_u64()
            .filter(|n| *n <= u64::from(p.size))
            .ok_or_else(|| error("Invalid literate population"))?;
        let share = n * u64::from(moved) / u64::from(p.size);
        parent
            .extra
            .insert("literate_count".into(), json!(n - share));
        child.extra.insert("literate_count".into(), json!(share));
    }
    let provinces: HashSet<_> = w.provinces().values().map(|p| p.id).collect();
    if provinces.len() != w.provinces().len()
        || !provinces.contains(&child.province_id)
        || w.pops().values().any(|p| !provinces.contains(&p.province_id))
        || w.pops().len() >= u32::MAX as usize
    {
        return Err(error("Invalid cohort province/ranges"));
    }
    let mut op = EconomicOperation {
        event_id: r.event_id.clone().into(),
        kind: OperationKind::PopCohortSplit,
        debits: vec![],
        credits: vec![],
        stock_changes: vec![],
        obligation: None,
        meta: Extra::from([
            ("pop_id".into(), p.external_id.json()),
            ("child_pop_id".into(), childid.json()),
            ("income_recognized".into(), json!(false)),
        ]),
    };
    let money_share = |n: i64| -> OpResult<Money> {
        if n < 0 {
            return Err(error("Negative cohort balance/claim"));
        }
        Ok(Money(
            (i128::from(n) * i128::from(moved) / i128::from(p.size)) as i64,
        ))
    };
    for a in &reads.accounts {
        if !full && a.purpose != "personal_savings" {
            continue;
        }
        let amount = money_share(a.balance.0)?;
        if amount.0 == 0 {
            continue;
        }
        op.debits.push(MoneyLeg {
            owner_id: p.external_id.clone(),
            currency_id: a.currency_id.clone(),
            purpose: a.purpose.clone(),
            amount_minor: amount,
        });
        op.credits.push(MoneyLeg {
            owner_id: childid.clone(),
            currency_id: a.currency_id.clone(),
            purpose: a.purpose.clone(),
            amount_minor: amount,
        });
    }
    let mut templates = vec![];
    for s in &reads.stocks {
        if (!full && s.quantity == 0.) || reads.assets.retained.contains(&s.external_id) {
            continue;
        }
        let amount = (if full {
            s.quantity
        } else {
            (s.quantity - reads.assets.held.get(&s.external_id).copied().unwrap_or(0.)).max(0.)
        }) * f64::from(moved)
            / f64::from(p.size);
        if amount < 0. || (!full && amount == 0.) {
            continue;
        }
        let id: ExternalId = format!(
            "{}:{}",
            cohort_assets::text(&s.external_id),
            cohort_assets::text(&childid)
        )
        .into();
        if w.stockpiles().values().any(|s| s.external_id == id) {
            return Err(error("Cohort stock exists without receipt"));
        }
        let mut next = s.clone();
        next.owner_id = childid.clone();
        if next.beneficiary_pop_id.as_ref() == Some(&p.external_id) {
            next.beneficiary_pop_id = Some(childid.clone())
        }
        op.stock_changes.push(StockDelta {
            stock_id: s.external_id.clone(),
            delta: -amount,
            create: None,
        });
        op.stock_changes.push(StockDelta {
            stock_id: id.clone(),
            delta: amount,
            create: Some(stock_create(&next)),
        });
        templates.push((id, next));
    }
    let mut operations = vec![op];
    for debt in &reads.claims {
        if !full
            && debt
                .extra
                .get("shipment_id")
                .is_some_and(crate::needs::js_truthy)
        {
            continue;
        }
        let amount = money_share(debt.amount_minor.0)?;
        if amount.0 == 0 {
            continue;
        }
        let mut old = debt.clone();
        old.amount_minor = Money(debt.amount_minor.0 - amount.0);
        let mut next = debt.clone();
        next.obligation_id = format!(
            "{}:{}",
            cohort_assets::text(&debt.obligation_id),
            cohort_assets::text(&childid)
        )
        .into();
        next.amount_minor = amount;
        if w.obligations().values()
            .any(|d| d.obligation_id == next.obligation_id)
        {
            return Err(error("Cohort claim exists without receipt"));
        }
        if next.payer_id == p.external_id {
            next.payer_id = childid.clone()
        }
        if next.payee_id == p.external_id {
            next.payee_id = childid.clone()
        }
        if next.pop_id.as_ref() == Some(&p.external_id) {
            next.pop_id = Some(childid.clone())
        }
        for (side, d) in [("parent", old), ("child", next)] {
            operations.push(EconomicOperation {
                event_id: format!(
                    "{}:debt:{side}:{}",
                    r.event_id,
                    cohort_assets::text(&debt.obligation_id)
                )
                .into(),
                kind: OperationKind::PopCohortClaim,
                debits: vec![],
                credits: vec![],
                stock_changes: vec![],
                obligation: Some(d),
                meta: Extra::from([("income_recognized".into(), json!(false))]),
            });
        }
    }
    let mut session = OperationSession::new(w)?;
    for op in &operations {
        session.stage_cohort(op, &templates)?;
    }
    child.id = PopId(session.next_ordinal()?);
    child.account_id = child.currency_id.as_ref().and_then(|currency| {
        session
            .account(&AccountAddress {
                owner_id: childid.clone(),
                currency_id: currency.clone(),
                purpose: "personal_savings".into(),
            })
            .map(|a| a.id)
    });
    child.account_status = if child.account_id.is_some() {
        "bound"
    } else {
        "unbound"
    }
    .into();
    if w.production_catalog().is_some() {
        result.views = w
            .derive_enterprise_views_with_artisans(
                session.accounts(),
                w.factories().values(),
                w.artisans().values().enumerate().map(|(i, a)| {
                    reads
                        .assets
                        .artisans
                        .iter()
                        .find(|c| c.position == i)
                        .map(|c| &c.after)
                        .unwrap_or(a)
                }),
                u64::from(child.id.0),
            )
            .map_err(OperationError)?;
    } else {
        result.views = w.enterprises().values().cloned().collect::<Vec<_>>()
    }
    result.money = Some(session.finish()?);
    if w.pop_mobility().is_some() {
        result.unemployment = Some(split_history(
            reads.unemployment.as_ref(),
            r.workforce_count,
        )?)
    }
    for (position, reference) in w.needs()
        .iter()
        .flat_map(|n| n.profile_refs.iter())
        .enumerate()
        .filter(|(_, r)| r.pop_id == p.external_id)
    {
        let (mut left, mut next) = (reference.clone(), reference.clone());
        left.extra.insert("size".into(), json!(parent.size));
        left.extra
            .insert("workforce".into(), json!(parent.workforce));
        next.pop_id = childid.clone();
        next.extra.insert("size".into(), json!(child.size));
        next.extra
            .insert("workforce".into(), json!(child.workforce));
        for (key, value) in [
            (
                "pop_type_id",
                r.target.pop_type_id.as_ref().map(ExternalId::json),
            ),
            ("province_id", r.target.province_id.map(|id| json!(id.0))),
            ("state_id", r.target.state_id.as_ref().map(ExternalId::json)),
            (
                "currency_id",
                r.target.currency_id.as_ref().map(ExternalId::json),
            ),
        ] {
            if let Some(v) = value {
                next.extra.insert(key.into(), v);
            }
        }
        result.needs.push((position, left, next));
    }
    if child.province_id != p.province_id {
        for (position, t) in
            w.territory().values().enumerate().filter(|(_, t)| {
                t.province_id == p.province_id || t.province_id == child.province_id
            })
        {
            if let Some(n) = t.extra.get("population_total") {
                let n = n
                    .as_f64()
                    .filter(|n| n.is_finite() && *n >= 0.)
                    .ok_or_else(|| error("Invalid territory population"))?;
                let next = n + if t.province_id == p.province_id {
                    -(moved as f64)
                } else {
                    moved as f64
                };
                if next < 0. {
                    return Err(error("Negative territory population"));
                }
                let mut after = t.clone();
                after.extra.insert("population_total".into(), json!(next));
                result.territory.push((position, after));
            }
        }
    }
    result.receipt.child_id = Some(childid);
    result.receipt.people_moved = moved;
    result.receipt.workforce_moved = r.workforce_count;
    result.receipt.operations = operations;
    result.parent = Some(parent);
    result.child = Some(child);
    result.reads = Some(reads);
    Ok(result)
}
fn apply_split(
    w: &mut WorldState,
    p: PreparedPopSplit,
    indexed_labor: Option<&[LaborEntry]>,
) -> OpResult<CohortSplitReceipt> {
    if let Some(receipt) = replay(w, &p.receipt.request)? {
        return Ok(receipt);
    }
    if p.receipt.blocked_reason.is_some() {
        return Ok(p.receipt);
    }
    let r = &p.receipt.request;
    let old = w
        .pops
        .iter()
        .find(|p| p.external_id == r.pop_id)
        .ok_or_else(|| error("Prepared cohort source vanished"))?;
    let reads = p
        .reads
        .ok_or_else(|| error("Prepared cohort reads missing"))?;
    if reads
        != CohortReads::capture(
            w,
            old,
            p.receipt.child_id.as_ref().unwrap(),
            p.receipt.people_moved == old.size,
            &r.target,
            indexed_labor,
        )?
    {
        return Err(error("Prepared cohort consulted rows changed"));
    }
    apply_prepared(
        w,
        p.money
            .ok_or_else(|| error("Prepared cohort money missing"))?,
    )?;
    // Every fallible check is complete. Only sparse row installation remains.
    reads.assets.install(w);
    let parent = p.parent.unwrap();
    let child = p.child.unwrap();
    let index = w
        .pops
        .iter()
        .position(|p| p.external_id == parent.external_id)
        .unwrap();
    w.pops[index] = parent;
    w.pops.insert(index + 1, child);
    let ranks: HashMap<_, _> = w
        .provinces
        .iter()
        .enumerate()
        .map(|(i, p)| (p.id, i))
        .collect();
    w.pops.sort_by_key(|p| ranks[&p.province_id]);
    let mut cursor = 0;
    for province in &mut w.provinces {
        let start = cursor;
        while cursor < w.pops.len() && w.pops[cursor].province_id == province.id {
            cursor += 1
        }
        province.pop_range = (start as u32, cursor as u32);
    }
    w.enterprises = p.views;
    if let Some((stayed, moving)) = p.unemployment {
        let mobility = w.labor_data.pop_mobility.as_mut().unwrap();
        if mobility.get("unemployment").is_none_or(|v| !v.is_object()) {
            mobility["unemployment"] = json!({})
        }
        mobility["unemployment"][cohort_assets::text(&r.pop_id)] = stayed;
        mobility["unemployment"][cohort_assets::text(p.receipt.child_id.as_ref().unwrap())] =
            moving;
    }
    if let Some(needs) = w.market_data.needs.as_mut() {
        for (position, parent, child) in p.needs.into_iter().rev() {
            needs.profile_refs[position] = parent;
            needs.profile_refs.insert(position + 1, child);
        }
    }
    for (position, row) in p.territory {
        w.market_data.territory[position] = row;
    }
    w.mobility_data.cohort_receipts.push(p.receipt.clone());
    Ok(p.receipt)
}
pub fn execute_pop_split(
    w: &mut WorldState,
    r: &CohortSplitRequest,
) -> OpResult<CohortSplitReceipt> {
    let p = prepare_pop_split(w, r)?;
    apply_pop_split(w, p)
}
pub fn prepare_pop_split(w: &dyn WorldRead, r: &CohortSplitRequest) -> OpResult<PreparedPopSplit> {
    prepare_split(w, r, None)
}
pub fn apply_pop_split(w: &mut WorldState, p: PreparedPopSplit) -> OpResult<CohortSplitReceipt> {
    apply_split(w, p, None)
}
pub(crate) fn execute_indexed_split(
    w: &mut WorldState,
    r: &CohortSplitRequest,
    entries: &[LaborEntry],
) -> OpResult<CohortSplitReceipt> {
    let p = prepare_split(w, r, Some(entries))?;
    apply_split(w, p, Some(entries))
}
