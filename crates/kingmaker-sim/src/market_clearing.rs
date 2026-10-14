use kingmaker_core::world_read::WorldRead;
use crate::{
    market_operations::{invoice, purchased_stock_id, PurchaseEvidence, SaleAllocation},
    market_policy::{MarketLocation, MarketPolicy},
    money_math::allocate_big_budget,
    operation_session::{OpResult, OperationError, OperationSession},
};
use kingmaker_core::{
    market::MarketOrder,
    operations::{EconomicOperation, MoneyLeg, OperationKind, StockCreate, StockDelta},
    AccountAddress, ExternalId, Money, StockpileRecord,
};
use num_bigint::BigInt;
use num_traits::ToPrimitive;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

#[derive(Clone)]
pub struct MarketView<'a> {
    pub(crate) base: &'a dyn WorldRead,
    pub(crate) stocks: Vec<StockpileRecord>,
    stock_indices: HashMap<ExternalId, usize>,
    pub(crate) cash: HashMap<AccountAddress, Money>,
    pub(crate) currencies: HashMap<ExternalId, HashSet<ExternalId>>,
    pub(crate) policy: MarketPolicy,
    pub(crate) events: HashSet<ExternalId>,
}
impl<'a> MarketView<'a> {
    pub(crate) fn add_virtual_food(
        &mut self,
        source: &StockpileRecord,
        output: StockpileRecord,
    ) -> OpResult<()> {
        if self.stock_indices.contains_key(&output.external_id) {
            return Err(error("Virtual food ID collision"));
        }
        self.policy.derive_food_offer(source, &output);
        self.stock_indices
            .insert(output.external_id.clone(), self.stocks.len());
        self.stocks.push(output);
        Ok(())
    }
    pub fn from_session(s: &OperationSession<'a>) -> OpResult<Self> {
        let accounts = s.accounts().collect::<Vec<_>>();
        let mut currencies: HashMap<_, HashSet<_>> = HashMap::new();
        for a in &accounts {
            currencies
                .entry(a.owner_id.clone())
                .or_default()
                .insert(a.currency_id.clone());
        }
        let stocks = s.stocks().cloned().collect::<Vec<_>>();
        let stock_indices = stocks
            .iter()
            .enumerate()
            .map(|(i, s)| (s.external_id.clone(), i))
            .collect();
        Ok(Self {
            base: s.base,
            stocks,
            stock_indices,
            cash: accounts.iter().map(|a| (a.address(), a.balance)).collect(),
            currencies,
            policy: MarketPolicy::from_session(s)?,
            events: s
                .base.committed_operations().values()
                .map(|e| e.event_id.clone())
                .chain(s.operations().iter().map(|e| e.event_id.clone()))
                .collect(),
        })
    }
}
#[derive(Debug, Default, Clone)]
pub struct MarketClearResult {
    pub operations: Vec<EconomicOperation>,
    pub fulfilled: HashMap<String, f64>,
    pub(crate) proofs: HashMap<ExternalId, (MarketOrder, PurchaseEvidence)>,
}
fn error(s: &str) -> OperationError {
    OperationError(s.into())
}
fn metadata(id: ExternalId, kind: OperationKind, fields: Value) -> EconomicOperation {
    EconomicOperation {
        event_id: id,
        kind,
        debits: vec![],
        credits: vec![],
        stock_changes: vec![],
        obligation: None,
        meta: serde_json::from_value(fields).expect("constant operation metadata"),
    }
}
fn address(o: &MarketOrder) -> OpResult<AccountAddress> {
    Ok(AccountAddress {
        owner_id: o.owner_id.clone(),
        currency_id: o
            .currency_id
            .clone()
            .ok_or_else(|| error("Market currency missing"))?,
        purpose: o.purpose.clone(),
    })
}
fn finite(v: f64) -> OpResult<f64> {
    if v.is_finite() {
        Ok(v)
    } else {
        Err(error("Nonfinite progressive market quantity"))
    }
}
#[derive(Clone)]
struct Bid {
    order: MarketOrder,
    event: ExternalId,
    price: f64,
    requested: BigInt,
    budget: Money,
    own: bool,
    candidates: Vec<usize>,
    wanted: f64,
    fulfilled: f64,
    allocations: Vec<(usize, f64)>,
}
// Keep the first-seen stock order and exactly the original per-stock f64
// addition order. Hash iteration must never decide payouts or rounding order.
fn coalesce_allocations(rows: Vec<(usize, f64)>) -> Vec<(usize, f64)> {
    let mut allocated: Vec<(usize, f64)> = Vec::new();
    let mut positions: HashMap<usize, usize> = HashMap::new();
    for (stock, quantity) in rows {
        if let Some(&position) = positions.get(&stock) {
            allocated[position].1 += quantity;
        } else {
            positions.insert(stock, allocated.len());
            allocated.push((stock, quantity));
        }
    }
    allocated
}

fn fund_bids(rows: Vec<Bid>, budget: Money) -> OpResult<Vec<Bid>> {
    let allocations = allocate_big_budget(
        &rows.iter()
            .map(|b| (ExternalId::from(b.order.id.clone()), b.requested.clone()))
            .collect::<Vec<_>>(),
        budget,
    )?;
    // phase_clear already validated unique order IDs. Move each candidate
    // vector instead of cloning it while retaining the budget allocator's order.
    let mut by_id: HashMap<ExternalId, Bid> = rows.into_iter()
        .map(|b| (ExternalId::from(b.order.id.clone()), b))
        .collect();
    let mut funded = Vec::with_capacity(allocations.len());
    for a in allocations {
        let mut b = by_id.remove(&a.id).expect("validated unique market order");
        b.budget = a.paid;
        funded.push(b);
    }
    Ok(funded)
}

impl MarketView<'_> {
    fn can_settle(&self, s: &StockpileRecord, o: &MarketOrder) -> bool {
        s.owner_id == o.owner_id
            || s.currency_id == o.currency_id
            || o.currency_id.as_ref().is_some_and(|c| {
                self.currencies
                    .get(&s.owner_id)
                    .is_some_and(|set| set.contains(c))
            })
    }
    fn apply(&mut self, op: &EconomicOperation) -> OpResult<()> {
        for (sign, legs) in [(-1i128, &op.debits), (1i128, &op.credits)] {
            for leg in legs {
                let addr = leg.address();
                let amount = self.cash.get(&addr).copied().unwrap_or(Money(0));
                let next =
                    i64::try_from(i128::from(amount.0) + sign * i128::from(leg.amount_minor.0))
                        .map_err(|_| error("Market wallet overflow"))?;
                if next < 0 {
                    return Err(error("Market overspent wallet"));
                }
                self.cash.insert(addr, Money(next));
                if sign > 0 {
                    self.currencies
                        .entry(leg.owner_id.clone())
                        .or_default()
                        .insert(leg.currency_id.clone());
                }
            }
        }
        for change in &op.stock_changes {
            let index = self.stock_indices.get(&change.stock_id).copied();
            let index = if let Some(i) = index {
                i
            } else {
                let c = change
                    .create
                    .as_ref()
                    .ok_or_else(|| error("Market unknown stock creation"))?;
                self.stocks.push(StockpileRecord {
                    extra: c
                        .extra
                        .iter()
                        .filter(|(key, _)| {
                            !matches!(*key, "inventory_purpose" | "beneficiary_pop_id")
                        })
                        .map(|(k, v)| (k.to_owned(), v.clone()))
                        .collect(),
                    external_id: change.stock_id.clone(),
                    province_id: c.province_id,
                    goods: kingmaker_core::Goods::from_external(c.goods_id.clone()),
                    owner_id: c.owner_id.clone(),
                    unit: Some(c.unit.clone()),
                    facility_id: c.facility_id.clone(),
                    currency_id: c.currency_id.clone(),
                    inventory_origin: Some(c.inventory_origin.clone()),
                    method_kind: c.method_kind.clone(),
                    purpose: c
                        .extra
                        .get("inventory_purpose")
                        .and_then(Value::as_str)
                        .unwrap_or("commercial")
                        .into(),
                    beneficiary_pop_id: c
                        .extra
                        .get("beneficiary_pop_id")
                        .map(|v| ExternalId::from_json(v.clone()).map_err(OperationError))
                        .transpose()?,
                    ..Default::default()
                });
                self.stocks.len() - 1
            };
            self.stock_indices.insert(change.stock_id.clone(), index);
            self.stocks[index].quantity = finite(self.stocks[index].quantity + change.delta)?;
            if self.stocks[index].quantity < -1e-9 {
                return Err(error("Market depleted stock below zero"));
            }
            if self.stocks[index].quantity < 0.0 {
                self.stocks[index].quantity = 0.0;
            }
        }
        self.events.insert(op.event_id.clone());
        Ok(())
    }
}
fn phase_clear(
    view: &mut MarketView,
    period: &str,
    orders: &[MarketOrder],
) -> OpResult<MarketClearResult> {
    let collation = crate::ordering::collator();
    let mut normalized = orders.to_vec();
    normalized.sort_by(|a, b| collation.compare(&a.id, &b.id));
    let mut ids = HashSet::new();
    for o in &normalized {
        crate::run_control::work("market_orders")?;
        if !ids.insert(&o.id) || !o.quantity.is_finite() || o.quantity < 0.0 {
            return Err(error("Duplicate market order ID or invalid quantity"));
        }
    }
    let mut result = MarketClearResult::default();
    let mut eligible = Vec::new();
    let mut buyers: Vec<(AccountAddress, Vec<Bid>)> = Vec::new();
    let mut buyer_index = HashMap::<AccountAddress, usize>::new();
    let mut local_index: HashMap<_, Vec<usize>> = HashMap::new();
    for (i, s) in view.stocks.iter().enumerate() {
        crate::run_control::work("market_stocks")?;
        local_index
            .entry((view.policy.scope.key(s.province_id), s.goods.external_id()))
            .or_default()
            .push(i);
    }
    for o in normalized.into_iter().filter(|o| o.quantity > 0.0) {
        let event: ExternalId = format!(
            "market:{}:{}:{}",
            view.base.campaign().campaign_id, period, o.id
        )
        .into();
        if view.events.contains(&event) {
            continue;
        }
        if let Some(reason) = o
            .extra
            .get("blocked_reason")
            .filter(|v| crate::needs::js_truthy(v))
        {
            result.operations.push(metadata(event,OperationKind::Blocked,json!({"reason":reason,"order_id":o.id,"province_id":o.province_id.0,"goods_id":o.goods_id,"requested_quantity":o.quantity})));
            continue;
        }
        let price = MarketPolicy::quote(view.base, &o);
        let location = view.policy.scope.key(o.province_id);
        let local = local_index
            .get(&(location, o.goods_id.clone()))
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let candidates = local
            .iter()
            .copied()
            .filter(|&i| {
                view.policy.free(&view.stocks[i]) > 0.0
                    && view.can_settle(&view.stocks[i], &o)
                    && view.policy.can_sell(&view.stocks[i], &o)
            })
            .collect::<Vec<_>>();
        if price.is_none() || candidates.is_empty() {
            let exists = local
                .iter()
                .any(|&i| view.policy.free(&view.stocks[i]) > 0.0);
            let buyable = local.iter().any(|&i| {
                view.policy.free(&view.stocks[i]) > 0.0 && view.policy.can_sell(&view.stocks[i], &o)
            });
            let reason = if price.is_none() {
                "unknown_price_or_currency"
            } else if exists {
                "unknown_settlement_or_fx"
            } else {
                "pending_transport"
            };
            let constraint = if price.is_none() {
                reason
            } else if !exists {
                "no_supply"
            } else if !buyable {
                "no_buyable_supply"
            } else {
                "settlement_unavailable"
            };
            result.operations.push(metadata(event,OperationKind::Blocked,json!({"reason":reason,"constraint_reason":constraint,"order_id":o.id,"province_id":o.province_id.0,"goods_id":o.goods_id,"pop_id":o.pop_id,"requested_quantity":o.quantity,"priority":o.priority})));
            continue;
        }
        let own = candidates
            .iter()
            .copied()
            .filter(|&i| view.stocks[i].owner_id == o.owner_id)
            .collect::<Vec<_>>();
        let own_only = !own.is_empty();
        let price = price.unwrap();
        let bid = Bid {
            requested: invoice(o.quantity, price)?,
            order: o,
            event,
            price,
            budget: Money(0),
            own: own_only,
            candidates: if own_only { own } else { candidates },
            wanted: 0.0,
            fulfilled: 0.0,
            allocations: vec![],
        };
        if own_only {
            eligible.push(bid);
        } else {
            let addr = address(&bid.order)?;
            if let Some(&i) = buyer_index.get(&addr) {
                buyers[i].1.push(bid);
            } else {
                buyer_index.insert(addr.clone(), buyers.len());
                buyers.push((addr, vec![bid]));
            }
        }
    }
    for (addr, rows) in buyers {
        eligible.extend(fund_bids(
            rows,
            view.cash.get(&addr).copied().unwrap_or(Money(0)),
        )?);
    }
    for b in &mut eligible {
        b.wanted = if b.own || b.requested == BigInt::from(0) {
            b.order.quantity
        } else {
            finite(
                b.order.quantity * b.budget.0 as f64
                    / b.requested
                        .to_f64()
                        .ok_or_else(|| error("Requested market ratio cannot become f64"))?,
            )?
        };
    }
    let mut groups: Vec<((MarketLocation, ExternalId), Vec<usize>)> = Vec::new();
    let mut market_index = HashMap::<(MarketLocation, ExternalId), usize>::new();
    for (i, b) in eligible.iter().enumerate() {
        let key = (
            view.policy.scope.key(b.order.province_id),
            b.order.goods_id.clone(),
        );
        if let Some(&j) = market_index.get(&key) {
            groups[j].1.push(i);
        } else {
            market_index.insert(key.clone(), groups.len());
            groups.push((key, vec![i]));
        }
    }
    let mut available = view
        .stocks
        .iter()
        .map(|s| view.policy.free(s))
        .collect::<Vec<_>>();
    for (_, rows) in groups {
        for _ in 0..32 {
            let mut proposals: Vec<(usize, Vec<(usize, f64)>)> = Vec::new();
            let mut proposal_index = HashMap::<usize, usize>::new();
            for &i in &rows {
                let b = &eligible[i];
                let remaining = b.wanted - b.fulfilled;
                if remaining <= 1e-12 {
                    continue;
                }
                let candidates = b
                    .candidates
                    .iter()
                    .copied()
                    .filter(|&s| available[s] > 1e-12)
                    .collect::<Vec<_>>();
                let total = candidates.iter().map(|&s| available[s]).sum::<f64>();
                for stock in candidates {
                    let q = finite(remaining * available[stock] / total)?;
                    if let Some(&j) = proposal_index.get(&stock) {
                        proposals[j].1.push((i, q));
                    } else {
                        proposal_index.insert(stock, proposals.len());
                        proposals.push((stock, vec![(i, q)]));
                    }
                }
            }
            if proposals.is_empty() {
                break;
            }
            proposals.sort_by(|(a, _), (b, _)| {
                collation.compare(
                    &crate::needs::js_string(&view.stocks[*a].external_id.json()),
                    &crate::needs::js_string(&view.stocks[*b].external_id.json()),
                )
            });
            let mut progress = 0.0;
            for (stock, proposed) in proposals {
                let demand = proposed.iter().map(|(_, q)| q).sum::<f64>();
                let factor = (available[stock] / demand).min(1.0);
                for (i, q) in proposed {
                    let q = finite(q * factor)?;
                    eligible[i].fulfilled += q;
                    eligible[i].allocations.push((stock, q));
                    available[stock] -= q;
                    progress += q;
                }
            }
            if progress <= 1e-12 {
                break;
            }
        }
    }
    eligible.sort_by(|a, b| collation.compare(&a.order.id, &b.order.id));
    for b in eligible {
        if b.fulfilled <= 0.0 {
            continue;
        }
        let actual = invoice(b.fulfilled, b.price)?;
        let cost = if b.own {
            Money(0)
        } else {
            Money(
                actual
                    .min(BigInt::from(b.budget.0))
                    .to_i64()
                    .ok_or_else(|| error("Actual market payment overflow"))?,
            )
        };
        let mut allocated = coalesce_allocations(b.allocations);
        allocated.sort_by(|(a, _), (b, _)| {
            collation.compare(
                &crate::needs::js_string(&view.stocks[*a].external_id.json()),
                &crate::needs::js_string(&view.stocks[*b].external_id.json()),
            )
        });
        let weights = allocated
            .iter()
            .map(|(_, q)| BigInt::from((q / b.fulfilled * 1_000_000_000.0).round() as i64))
            .collect::<Vec<_>>();
        let weight_sum = weights.iter().sum::<BigInt>();
        let mut used = 0i64;
        let mut credits = Vec::new();
        let currency = b
            .order
            .currency_id
            .clone()
            .ok_or_else(|| error("Eligible purchase currency missing"))?;
        if cost.0 > 0 {
            for (j, (i, _)) in allocated.iter().enumerate() {
                let amount = if j + 1 == allocated.len() {
                    cost.0 - used
                } else {
                    (BigInt::from(cost.0) * &weights[j] / &weight_sum)
                        .to_i64()
                        .ok_or_else(|| error("Seller payment overflow"))?
                };
                used += amount;
                credits.push(MoneyLeg {
                    owner_id: view.stocks[*i].owner_id.clone(),
                    currency_id: currency.clone(),
                    purpose: view.policy.seller_purpose(&view.stocks[*i]),
                    amount_minor: Money(amount),
                });
            }
        }
        let mut changes = allocated
            .iter()
            .map(|(i, q)| StockDelta {
                stock_id: view.stocks[*i].external_id.clone(),
                delta: -q,
                create: None,
            })
            .collect::<Vec<_>>();
        if !b.order.consume {
            let unit = view.stocks[allocated[0].0]
                .unit
                .clone()
                .unwrap_or(Value::Null);
            let mut extra = Default::default();
            if let Some(p) = b
                .order
                .extra
                .get("inventory_purpose")
                .filter(|p| crate::needs::js_truthy(p))
            {
                extra = serde_json::from_value(
                    json!({"inventory_purpose":p,"beneficiary_pop_id":b.order.pop_id}),
                )?;
            }
            changes.push(StockDelta {
                stock_id: purchased_stock_id(&b.order),
                delta: b.fulfilled,
                create: Some(StockCreate {
                    owner_id: b.order.owner_id.clone(),
                    province_id: b.order.province_id,
                    goods_id: b.order.goods_id.clone(),
                    unit,
                    facility_id: b.order.facility_id.clone(),
                    currency_id: Some(currency.clone()),
                    inventory_origin: "purchased".into(),
                    method_kind: None,
                    extra,
                }),
            });
        }
        let mut op = metadata(
            b.event.clone(),
            OperationKind::Purchase,
            json!({"order_id":b.order.id,"province_id":b.order.province_id.0,"goods_id":b.order.goods_id,"quantity":b.fulfilled,"requested_quantity":b.order.quantity,"consumed":b.order.consume,"paid_minor":cost,"currency_id":currency,"priority":b.order.priority,"maintenance":b.order.maintenance,"partial":b.fulfilled<b.order.quantity-1e-9,"causes":["local owned stock","common buyer budget","eligible currency quote","joint proportional clearing"]}),
        );
        if let Some(pop) = &b.order.pop_id {
            op.meta.insert("pop_id".into(), pop.json());
        }
        if let Some(need) = &b.order.need_id {
            op.meta.insert("need_id".into(), need.json());
        }
        if view.policy.procurement_sources.is_some() {
            op.meta.insert("public_procurement".into(), json!(true));
            op.meta.insert(
                "state_id".into(),
                b.order
                    .extra
                    .get("state_id")
                    .cloned()
                    .ok_or_else(|| error("Procurement order state missing"))?,
            );
        }
        op.debits = if cost.0 > 0 {
            vec![MoneyLeg {
                owner_id: b.order.owner_id.clone(),
                currency_id: currency,
                purpose: b.order.purpose.clone(),
                amount_minor: cost,
            }]
        } else {
            vec![]
        };
        op.credits = credits;
        op.stock_changes = changes;
        result.fulfilled.insert(b.order.id.clone(), b.fulfilled);
        result.proofs.insert(
            op.event_id.clone(),
            (
                b.order,
                PurchaseEvidence {
                    budget_minor: b.budget,
                    allocations: allocated
                        .iter()
                        .map(|(i, q)| SaleAllocation {
                            stock_id: view.stocks[*i].external_id.clone(),
                            quantity: *q,
                        })
                        .collect(),
                },
            ),
        );
        result.operations.push(op);
    }
    for op in &result.operations {
        view.apply(op)?;
    }
    Ok(result)
}
fn append(result: &mut MarketClearResult, next: MarketClearResult) {
    for (id, q) in next.fulfilled {
        *result.fulfilled.entry(id).or_default() += q;
    }
    result.operations.extend(next.operations);
    result.proofs.extend(next.proofs);
}
fn clear_rows(
    view: &mut MarketView,
    period: &str,
    rows: &[MarketOrder],
    result: &mut MarketClearResult,
    phase: &mut usize,
) -> OpResult<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let owned = view
        .stocks
        .iter()
        .filter(|s| view.policy.free(s) > 0.0)
        .map(|s| {
            (
                s.owner_id.clone(),
                view.policy.scope.key(s.province_id),
                s.goods.external_id(),
            )
        })
        .collect::<HashSet<_>>();
    let own = rows
        .iter()
        .filter(|o| {
            owned.contains(&(
                o.owner_id.clone(),
                view.policy.scope.key(o.province_id),
                o.goods_id.clone(),
            ))
        })
        .cloned()
        .collect::<Vec<_>>();
    if !own.is_empty() {
        *phase += 1;
        append(
            result,
            phase_clear(view, &format!("{period}:phase{phase}"), &own)?,
        );
    }
    let remaining = rows
        .iter()
        .map(|o| {
            let mut n = o.clone();
            n.quantity =
                (o.quantity - result.fulfilled.get(&o.id).copied().unwrap_or(0.0)).max(0.0);
            n
        })
        .filter(|o| o.quantity > 1e-10)
        .collect::<Vec<_>>();
    if !remaining.is_empty() {
        *phase += 1;
        append(
            result,
            phase_clear(view, &format!("{period}:phase{phase}"), &remaining)?,
        );
    }
    Ok(())
}
pub fn plan_local_orders(
    view: &MarketView,
    period: &str,
    orders: &[MarketOrder],
    priorities: &[String],
) -> OpResult<MarketClearResult> {
    let mut state = view.clone();
    plan_local_orders_in_place(&mut state, period, orders, priorities)
}

// Private callers may consume their own scratch view. The public API above
// still leaves its input unchanged, including when planning fails.
fn plan_local_orders_in_place(
    state: &mut MarketView,
    period: &str,
    orders: &[MarketOrder],
    priorities: &[String],
) -> OpResult<MarketClearResult> {
    let mut result = MarketClearResult::default();
    let mut phase = 0;
    let supply_by_pop = orders
        .iter()
        .filter(|o| o.goods_id.as_str() == Some("military_supply"))
        .map(|o| (o.pop_id.clone(), o))
        .collect::<HashMap<_, _>>();
    for priority in priorities {
        let rows = orders
            .iter()
            .filter(|o| &o.priority == priority)
            .cloned()
            .collect::<Vec<_>>();
        clear_rows(state, period, &rows, &mut result, &mut phase)?;
        let mut groups: Vec<((Option<ExternalId>, ExternalId), Vec<&MarketOrder>)> = Vec::new();
        let mut group_index = HashMap::new();
        for o in rows
            .iter()
            .filter(|o| o.need_id.as_ref().and_then(ExternalId::as_str) == Some("food"))
        {
            let key = (o.pop_id.clone(), o.owner_id.clone());
            if let Some(&i) = group_index.get(&key) {
                let (_, r): &mut ((Option<ExternalId>, ExternalId), Vec<&MarketOrder>) =
                    &mut groups[i];
                r.push(o);
            } else {
                group_index.insert(key.clone(), groups.len());
                groups.push((key, vec![o]));
            }
        }
        let mut fallback = Vec::new();
        let mut local_stocks: HashMap<MarketLocation, Vec<&StockpileRecord>> = HashMap::new();
        for stock in &state.stocks {
            local_stocks
                .entry(state.policy.scope.key(stock.province_id))
                .or_default()
                .push(stock);
        }
        for (_, group) in groups {
            let first = group[0];
            let base = group.iter().map(|o| o.quantity).sum::<f64>();
            let supply = supply_by_pop.get(&first.pop_id).copied();
            let supply_fraction = supply
                .filter(|s| s.quantity > 0.0)
                .map(|s| {
                    (result.fulfilled.get(&s.id).copied().unwrap_or(0.0) / s.quantity).min(1.0)
                })
                .unwrap_or(0.0);
            let substitution = first.military_food_substitution.unwrap_or(0.0);
            let desired = first
                .food_gross_quantity
                .map(|g| (g - (substitution * supply_fraction).min(g)).max(0.0))
                .unwrap_or(base + substitution * (1.0 - supply_fraction));
            let done = group
                .iter()
                .map(|o| result.fulfilled.get(&o.id).copied().unwrap_or(0.0))
                .sum::<f64>();
            let remaining = desired - done;
            if remaining <= 1e-10 {
                continue;
            }
            let mut eligible: HashMap<ExternalId, f64> = HashMap::new();
            for stock in local_stocks
                .get(&state.policy.scope.key(first.province_id))
                .into_iter()
                .flatten()
            {
                let free = state.policy.free(stock);
                let good = stock.goods.external_id();
                if free > 1e-10
                    && state
                        .policy
                        .scope
                        .same(stock.province_id, first.province_id)
                    && first
                        .food_alternatives
                        .iter()
                        .any(|a| a.get("goods_id") == Some(&good.json()))
                    && state.can_settle(stock, first)
                    && (stock.owner_id == first.owner_id || {
                        let mut o = first.clone();
                        o.goods_id = good.clone();
                        MarketPolicy::quote(state.base, &o).is_some()
                    })
                {
                    *eligible.entry(good).or_default() += free;
                }
            }
            let candidates = first
                .food_alternatives
                .iter()
                .filter_map(|a| {
                    a.get("goods_id")
                        .and_then(|v| ExternalId::from_json(v.clone()).ok())
                })
                .filter(|id| eligible.contains_key(id))
                .collect::<Vec<_>>();
            let total = candidates.iter().map(|id| eligible[id]).sum::<f64>();
            for good in candidates {
                let mut o = first.clone();
                o.id = format!(
                    "fallback:{}:food:{}",
                    first
                        .pop_id
                        .as_ref()
                        .map(|id| crate::needs::js_string(&id.json()))
                        .unwrap_or_else(|| "undefined".into()),
                    crate::needs::js_string(&good.json())
                );
                o.quantity = finite(remaining.max(0.0) * eligible[&good] / total)?;
                o.goods_id = good;
                fallback.push(o);
            }
        }
        clear_rows(state, period, &fallback, &mut result, &mut phase)?;
    }
    Ok(result)
}
pub(crate) fn apply_market_result(
    session: &mut OperationSession,
    result: &MarketClearResult,
) -> OpResult<()> {
    let applied = (|| {
        for op in &result.operations {
            match op.kind {
                OperationKind::Purchase => {
                    let (order, proof) = result
                        .proofs
                        .get(&op.event_id)
                        .ok_or_else(|| error("Planned purchase proof absent"))?;
                    session.authorize_purchase(op, order, proof)?;
                    session.stage(op)?;
                }
                OperationKind::Blocked => {
                    session.stage(op)?;
                }
                _ => {
                    session.poison();
                    return Err(error("Unexpected market planned operation"));
                }
            }
        }
        Ok(())
    })();
    if applied.is_err() {
        session.poison();
    }
    applied
}

pub(crate) fn plan_foundation_market(
    s: &mut OperationSession,
    period: &str,
    orders: &[MarketOrder],
    priorities: &[String],
) -> OpResult<MarketClearResult> {
    use crate::storage::{
        consume_storage, release_storage, reserve_storage, StorageBatch, StorageRequest,
    };
    let result = (|| {
        let start = s.operations().len();
        let mut batch = StorageBatch::new(s)?;
        let mut accepted = Vec::new();
        let mut reserved = Vec::new();
        let mut ids = HashSet::new();
        for o in orders {
            if !ids.insert(&o.id) {
                return Err(error("Duplicate foundation order ID"));
            }
            if o.consume {
                accepted.push(o.clone());
                continue;
            }
            let id: ExternalId = format!("market-storage:{period}:{}", o.id).into();
            let outcome = if let Some(facility) = &o.facility_id {
                let unit = batch
                    .availability(facility, &o.goods_id)
                    .unit
                    .unwrap_or(Value::Null);
                Some(reserve_storage(
                    s,
                    &mut batch,
                    &StorageRequest {
                        id: id.clone(),
                        owner_id: o.owner_id.clone(),
                        facility_id: facility.clone(),
                        goods_id: o.goods_id.clone(),
                        unit,
                        quantity: o.quantity,
                    },
                    period,
                )?)
            } else {
                None
            };
            if let Some(row) = outcome.as_ref().and_then(|r| r.reservation.as_ref()) {
                let mut order = o.clone();
                order.quantity = row.quantity;
                order.facility_id = Some(row.facility_id.clone());
                accepted.push(order);
                reserved.push((o.id.clone(), id));
            } else {
                let reason = outcome
                    .as_ref()
                    .and_then(|r| r.reason.as_deref())
                    .unwrap_or("storage_parameters_unknown");
                s.stage(&metadata(format!("market-blocked:{period}:{}",o.id).into(),OperationKind::Blocked,json!({"reason":reason,"province_id":o.province_id.0,"goods_id":o.goods_id,"order_id":o.id})))?;
            }
        }
        let mut view = MarketView::from_session(s)?;
        s.purchase_policy = Some(view.policy.clone());
        let mut market = plan_local_orders_in_place(&mut view, period, &accepted, priorities)?;
        apply_market_result(s, &market)?;
        for (order, id) in reserved {
            let q = market.fulfilled.get(&order).copied().unwrap_or(0.0);
            consume_storage(s, &mut batch, &id, q)?;
            if s.storage_reservation(&id)
                .is_some_and(|r| r.status == "held")
            {
                release_storage(
                    s,
                    &mut batch,
                    &id,
                    &format!(
                        "market-release:{period}:{}",
                        crate::needs::js_string(&id.json())
                    )
                    .into(),
                )?;
            }
        }
        market.operations = s.operations()[start..].to_vec();
        s.purchase_policy = None;
        Ok(market)
    })();
    if result.is_err() {
        s.poison();
    }
    result
}

#[cfg(test)]
#[path = "../tests/internal/perf_market.rs"]
mod perf_market_tests;
