// Frozen phase_clear from Git blob 8f2e0a2470bb2b395a6be4b15d7f1221501efffe.
// Test-only oracle: retain this implementation when changing the production helper.
fn reference_phase_clear(
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
        let allocations = allocate_big_budget(
            &rows
                .iter()
                .map(|b| (ExternalId::from(b.order.id.clone()), b.requested.clone()))
                .collect::<Vec<_>>(),
            view.cash.get(&addr).copied().unwrap_or(Money(0)),
        )?;
        for a in allocations {
            let mut b = rows
                .iter()
                .find(|b| b.order.id.as_str() == a.id.as_str().unwrap())
                .unwrap()
                .clone();
            b.budget = a.paid;
            eligible.push(b);
        }
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
        let mut allocated: Vec<(usize, f64)> = Vec::new();
        for (i, q) in b.allocations {
            if let Some((_, n)) = allocated.iter_mut().find(|(s, _)| s == &i) {
                *n += q;
            } else {
                allocated.push((i, q));
            }
        }
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
