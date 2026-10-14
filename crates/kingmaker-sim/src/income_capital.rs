//! Source-shaped shared working-capital forecast. It never buys or publishes anything.
use kingmaker_core::world_read::WorldRead;
use crate::{
    budget_funding::error,
    calendar::serial_date,
    market_operations::invoice,
    market_policy::{MarketLocation, MarketScope},
    money_math::multiply_money,
    needs::{js_string, js_truthy, undefined},
    operation_session::OpResult,
    storage::approved,
    transport_validation::{actor, province, RouteValidation},
};
use kingmaker_core::{
    calendar::GameDate, market::SaleOffer, ExternalId, Money, ProvinceId,
};
use num_traits::ToPrimitive;
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
type StockKey = (ExternalId, ProvinceId, ExternalId);
type CapacityKey = (ExternalId, u8, ExternalId);
type InputPlan<'a> = (
    &'a kingmaker_core::production::RecipeInput,
    StockKey,
    f64,
    f64,
    Vec<Lot>,
);

fn route_limits(r: &Value) -> OpResult<[(CapacityKey, f64); 3]> {
    let unit = actor(&r["unit"])?;
    let terminal = |key: &str, province_key: &str| -> OpResult<ExternalId> {
        r.get(key)
            .filter(|v| js_truthy(v))
            .map(actor)
            .unwrap_or_else(|| Ok(format!("province:{}", js_string(&r[province_key])).into()))
    };
    Ok([
        (
            (unit.clone(), 0, actor(&r["id"])?),
            quantity(&r["capacity_per_week"])?,
        ),
        (
            (
                unit.clone(),
                1,
                terminal("source_terminal_id", "from_province_id")?,
            ),
            quantity(&r["source_handling_per_week"])?,
        ),
        (
            (
                unit,
                2,
                terminal("destination_terminal_id", "to_province_id")?,
            ),
            quantity(&r["destination_handling_per_week"])?,
        ),
    ])
}

/// A small trial journal: all recipe ingredients see the same physical resources.
fn jointly_available(
    w: &dyn WorldRead,
    plans: &[InputPlan<'_>],
    output: f64,
    remaining: &HashMap<ExternalId, f64>,
    fleet: &HashMap<ExternalId, f64>,
    capacities: &HashMap<CapacityKey, f64>,
) -> OpResult<bool> {
    let mut source_used = HashMap::<ExternalId, f64>::new();
    let mut fleet_used = HashMap::<ExternalId, f64>::new();
    let mut capacity_used = HashMap::<CapacityKey, f64>::new();
    for (ingredient, _, own, inbound, lots) in plans {
        let mut wanted = (output * ingredient.amount - own - inbound).max(0.);
        for lot in lots {
            if wanted <= 0. {
                break;
            }
            let stock = w.stockpiles().at(lot.stock).expect("validated row position");
            let mut q = wanted.min(lot.quantity).min(
                (remaining.get(&stock.external_id).copied().unwrap_or(0.)
                    - source_used.get(&stock.external_id).copied().unwrap_or(0.))
                .max(0.),
            );
            if let Some(r) = &lot.route {
                let asset = actor(&r["asset_id"])?;
                let space = quantity(
                    &r["goods"][js_string(&ingredient.goods_id.json())]["space_per_unit"],
                )?;
                q = q.min(
                    (fleet.get(&asset).copied().unwrap_or(0.)
                        - fleet_used.get(&asset).copied().unwrap_or(0.))
                    .max(0.)
                        / space,
                );
                let limits = route_limits(r)?;
                for (key, limit) in &limits {
                    q = q.min(
                        (limit
                            - capacities.get(key).copied().unwrap_or(0.)
                            - capacity_used.get(key).copied().unwrap_or(0.))
                        .max(0.)
                            / space,
                    );
                }
                *fleet_used.entry(asset).or_default() += q * space;
                for (key, _) in limits {
                    *capacity_used.entry(key).or_default() += q * space;
                }
            }
            *source_used.entry(stock.external_id.clone()).or_default() += q;
            wanted -= q;
        }
        if wanted > 0. {
            return Ok(false);
        }
    }
    Ok(true)
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CapitalEntity {
    pub id: ExternalId,
    pub owner_id: ExternalId,
    pub currency_id: ExternalId,
    pub minimum_minor: Money,
    pub requested_minor: Money,
    pub possible: bool,
    pub active: bool,
    pub feasible_output: f64,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CapitalGroup {
    pub owner_id: ExternalId,
    pub currency_id: ExternalId,
    pub minimum_minor: Money,
    pub ids: Vec<ExternalId>,
    pub active: bool,
    pub personal_funded_minor: Money,
    pub retained_profit_minor: Money,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CapitalForecast {
    pub entities: Vec<CapitalEntity>,
    pub groups: Vec<CapitalGroup>,
}
pub struct InputSupplyIndex {
    pub available: HashMap<(MarketLocation, Option<ExternalId>, ExternalId), f64>,
    pub scope: MarketScope,
    pub eligible_indices: Vec<usize>,
    held: HashMap<ExternalId, f64>,
}
fn offer_allowed(o: &SaleOffer) -> bool {
    approved(
        o.extra.get("status").and_then(Value::as_str),
        o.extra.get("source"),
        o.extra.get("approval_ref"),
    )
}
pub fn build_input_supply_index(w: &dyn WorldRead) -> OpResult<InputSupplyIndex> {
    let scope = MarketScope::from_world(w)?;
    let owners: HashSet<_> = w.pops().values().map(|p| &p.external_id).collect();
    let offered: HashSet<_> = w.sale_offers().values()
        .filter(|o| offer_allowed(o))
        .map(|o| &o.stock_id)
        .collect();
    let mut held = HashMap::<_, f64>::new();
    for h in w.goods_reservations().values() {
        if h.status == "held" {
            if !h.quantity.is_finite() || h.quantity < 0. {
                return Err(error("Invalid held capital input"));
            }
            *held.entry(h.stock_id.clone()).or_default() += h.quantity;
        }
    }
    let methods = w.production_catalog()
        .as_ref()
        .map(|c| {
            c.methods
                .iter()
                .map(|m| (&m.method_id, m))
                .collect::<HashMap<_, _>>()
        })
        .unwrap_or_default();
    let mut protected = HashSet::new();
    for e in w.factories().values()
        .map(|e| &e.base)
        .chain(w.artisans().values().map(|e| &e.base))
    {
        if let (Some(owner), Some(m)) = (&e.owner_id, methods.get(&e.method_id)) {
            for input in &m.inputs {
                protected.insert((owner.clone(), e.province_id, input.goods_id.clone()));
            }
        }
    }
    let mut available = HashMap::new();
    let mut eligible_indices = vec![];
    for (i, s) in w.stockpiles().values().enumerate() {
        if !s.quantity.is_finite() || s.quantity < 0. {
            return Err(error("Invalid capital stock quantity"));
        }
        if s.purpose != "food_reserve"
            && (offered.contains(&s.external_id)
                || !owners.contains(&s.owner_id)
                    && !protected.contains(&(
                        s.owner_id.clone(),
                        s.province_id,
                        s.goods.external_id().clone(),
                    )))
        {
            *available
                .entry((
                    scope.key(s.province_id),
                    s.currency_id.clone(),
                    s.goods.external_id().clone(),
                ))
                .or_insert(0.) +=
                (s.quantity - held.get(&s.external_id).copied().unwrap_or(0.)).max(0.);
            eligible_indices.push(i);
        }
    }
    Ok(InputSupplyIndex {
        available,
        scope,
        eligible_indices,
        held,
    })
}
fn money(v: Option<&Value>) -> OpResult<Money> {
    let value = v.filter(|v| js_truthy(v));
    let amount = if let Some(v) = value {
        serde_json::from_value::<Money>(v.clone())?
    } else {
        Money::ZERO
    };
    if amount.0 < 0 {
        return Err(error("Negative artisan capital/principal"));
    }
    Ok(amount)
}
fn plus(a: Money, b: Money) -> OpResult<Money> {
    a.0.checked_add(b.0)
        .map(Money)
        .ok_or_else(|| error("Capital money overflow"))
}
fn bill(q: f64, p: f64) -> OpResult<Money> {
    invoice(q, p)?
        .to_i64()
        .map(Money)
        .ok_or_else(|| error("Capital invoice overflow"))
}
fn quantity(v: &Value) -> OpResult<f64> {
    v.as_f64()
        .filter(|q| q.is_finite() && *q >= 0.)
        .ok_or_else(|| error("Invalid forecast quantity"))
}
fn price(w: &dyn WorldRead, c: &ExternalId, g: &ExternalId) -> Option<f64> {
    w.price_calibration()
        .as_ref()?
        .prices_by_currency
        .get(&js_string(&c.json()))?
        .get(&js_string(&g.json()))?
        .as_f64()
        .filter(|p| p.is_finite() && *p > 0.)
}
#[derive(Clone)]
struct Lot {
    stock: usize,
    quantity: f64,
    route: Option<Value>,
}
enum Undo {
    Owned(StockKey, f64),
    Incoming(StockKey, f64),
    Source(ExternalId, f64),
    Fleet(ExternalId, f64),
    Capacity(CapacityKey, f64),
}
struct Routes<'a> {
    w: &'a dyn WorldRead,
    scope: &'a MarketScope,
    offers: HashMap<&'a ExternalId, &'a SaleOffer>,
    by_source: HashMap<ProvinceId, Vec<&'a Value>>,
    cache: HashMap<(ProvinceId, ProvinceId, ExternalId, ExternalId, ExternalId), Option<Value>>,
    validation: RouteValidation<'a>,
}
impl<'a> Routes<'a> {
    fn new(w: &'a dyn WorldRead, scope: &'a MarketScope) -> OpResult<Self> {
        let offers = w.sale_offers().values()
            .filter(|o| offer_allowed(o))
            .map(|o| (&o.stock_id, o))
            .collect();
        let mut by_source = HashMap::<_, Vec<_>>::new();
        for route in w.supply_routes().values() {
            if let Ok(id) = province(&route["from_province_id"]) {
                by_source.entry(id).or_default().push(route);
            }
        }
        Ok(Self {
            w,
            scope,
            offers,
            by_source,
            cache: HashMap::new(),
            validation: RouteValidation::new(w)?,
        })
    }
    fn route(&mut self, stock: usize, to: ProvinceId, c: &ExternalId) -> OpResult<Option<Value>> {
        let s = self.w.stockpiles().at(stock).expect("validated stock position");
        if self.scope.same(s.province_id, to) {
            return Ok(None);
        }
        let key = (
            s.province_id,
            to,
            c.clone(),
            s.goods.external_id().clone(),
            s.external_id.clone(),
        );
        if let Some(found) = self.cache.get(&key) {
            return Ok(found.clone());
        }
        let rows = self
            .by_source
            .get(&s.province_id)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let offer = self.offers.get(&s.external_id);
        let mut route = None;
        if offer.is_some() {
            let mut candidates = rows
                .iter()
                .filter(|r| {
                    province(&r["to_province_id"]).ok() == Some(to)
                        && r.get("currency_id") == Some(&c.json())
                        && r["goods"]
                            .get(js_string(&s.goods.external_id().json()))
                            .is_some_and(js_truthy)
                })
                .copied()
                .collect::<Vec<_>>();
            candidates.sort_by(|a, b| {
                a["travel_days"]
                    .as_f64()
                    .unwrap_or(f64::NAN)
                    .partial_cmp(&b["travel_days"].as_f64().unwrap_or(f64::NAN))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            route = candidates.first().map(|r| (*r).clone());
            if route.is_none()
                && self
                    .w
                    .automatic_land_paths()
                    .as_ref()
                    .is_some_and(js_truthy)
            {
                if let Some(template) = rows.iter().find(|r| {
                    r["mode"].as_str() == Some("land")
                        && r.get("currency_id") == Some(&c.json())
                        && r["goods"]
                            .get(js_string(&s.goods.external_id().json()))
                            .is_some_and(js_truthy)
                }) {
                    route = self.validation.adapt_land(template, s.province_id, to)?;
                }
            }
        }
        if let (Some(r), Some(o)) = (&route, offer) {
            if self.validation.reason(r, o).is_some() {
                route = None;
            }
        }
        self.cache.insert(key, route.clone());
        Ok(route)
    }
    fn lots(
        &mut self,
        owner: &ExternalId,
        to: ProvinceId,
        c: &ExternalId,
        g: &ExternalId,
        input: &InputSupplyIndex,
        remaining: &HashMap<ExternalId, f64>,
        fleet_free: &HashMap<ExternalId, f64>,
        capacities: &HashMap<CapacityKey, f64>,
    ) -> OpResult<Vec<Lot>> {
        let mut fleet = fleet_free.clone();
        let mut capacity_used = HashMap::<CapacityKey, f64>::new();
        let mut lots = vec![];
        for &i in &input.eligible_indices {
            let s = self.w.stockpiles().at(i).expect("validated stock position");
            if &s.owner_id == owner
                || s.goods.external_id() != *g
                || s.currency_id.as_ref() != Some(c)
            {
                continue;
            }
            let mut q = remaining.get(&s.external_id).copied().unwrap_or(0.);
            if q <= 0. {
                continue;
            }
            let route = self.route(i, to, c)?;
            if !self.scope.same(s.province_id, to) && route.is_none() {
                continue;
            }
            if let Some(r) = &route {
                let a = actor(&r["asset_id"])?;
                let space = quantity(&r["goods"][js_string(&g.json())]["space_per_unit"])?;
                q = q.min(fleet.get(&a).copied().unwrap_or(0.) / space);
                let limits = route_limits(r)?;
                for (key, limit) in &limits {
                    q = q.min(
                        (limit
                            - capacities.get(key).copied().unwrap_or(0.)
                            - capacity_used.get(key).copied().unwrap_or(0.))
                        .max(0.)
                            / space,
                    );
                }
                for (key, _) in limits {
                    *capacity_used.entry(key).or_default() += q * space;
                }
                fleet.insert(
                    a.clone(),
                    (fleet.get(&a).copied().unwrap_or(0.) - q * space).max(0.),
                );
            }
            if q > 0. {
                lots.push(Lot {
                    stock: i,
                    quantity: q,
                    route,
                });
            }
        }
        lots.sort_by(|a, b| {
            a.route
                .as_ref()
                .and_then(|r| r["travel_days"].as_f64())
                .unwrap_or(0.)
                .partial_cmp(
                    &b.route
                        .as_ref()
                        .and_then(|r| r["travel_days"].as_f64())
                        .unwrap_or(0.),
                )
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(lots)
    }
}
pub fn working_capital_requirements(w: &dyn WorldRead, quotes: &[Value]) -> OpResult<CapitalForecast> {
    let input = build_input_supply_index(w)?;
    let mut routes = Routes::new(w, &input.scope)?;
    let methods = w.production_catalog()
        .as_ref()
        .ok_or_else(|| error("Capital catalog missing"))?
        .methods
        .iter()
        .map(|m| (&m.method_id, m))
        .collect::<HashMap<_, _>>();
    let pops = w.pops().values()
        .map(|p| (&p.external_id, p))
        .collect::<HashMap<_, _>>();
    let mut currencies = HashMap::new();
    let mut cash = HashMap::<_, Money>::new();
    for a in w.accounts().values() {
        currencies.entry(&a.owner_id).or_insert(&a.currency_id);
        let key = (&a.owner_id, &a.currency_id, a.purpose.as_str());
        cash.insert(
            key,
            plus(cash.get(&key).copied().unwrap_or(Money::ZERO), a.balance)?,
        );
    }
    let mut owned = HashMap::<StockKey, f64>::new();
    for s in w.stockpiles().values() {
        let q = if s.purpose == "food_reserve" {
            0.
        } else {
            (s.quantity - input.held.get(&s.external_id).copied().unwrap_or(0.)).max(0.)
        };
        *owned
            .entry((
                s.owner_id.clone(),
                s.province_id,
                s.goods.external_id().clone(),
            ))
            .or_default() += q;
    }
    let today = serial_date(GameDate::from(w.campaign()))?;
    let mut incoming = HashMap::<StockKey, f64>::new();
    let mut fleet = HashMap::new();
    for a in w.transport_assets().values() {
        if let (Ok(id), Some(capacity)) = (
            actor(&a["id"]),
            a["capacity"].as_f64().filter(|q| q.is_finite() && *q >= 0.),
        ) {
            fleet.insert(id, capacity);
        }
    }
    for s in w.shipments().values() {
        let q = s["remaining_quantity"].as_f64().unwrap_or(0.);
        if q > 0.
            && s["status"].as_str() == Some("in_transit")
            && !js_truthy(&s["storage_delayed"])
            && js_truthy(&s["arrival_date"])
        {
            let date = serial_date(serde_json::from_value(s["arrival_date"].clone())?)?;
            if date > today && date <= today + 7 {
                *incoming
                    .entry((
                        actor(&s["buyer_id"])?,
                        province(&s["to_province_id"])?,
                        actor(&s["goods_id"])?,
                    ))
                    .or_default() += q;
            }
        }
        if let Ok(a) = actor(&s["asset_id"]) {
            if let Some(free) = fleet.get_mut(&a) {
                let busy = q > 0.
                    || s.get("carrier_release_date")
                        .filter(|v| !v.is_null() && !undefined(v))
                        .map(|v| {
                            serde_json::from_value(v.clone())
                                .map_err(crate::operation_session::OperationError::from)
                                .and_then(serial_date)
                        })
                        .transpose()?
                        .is_some_and(|date| date > today);
                if busy {
                    *free = (*free
                        - quantity(&s["shipped_quantity"])? * quantity(&s["space_per_unit"])?)
                    .max(0.);
                }
            }
        }
    }
    let mut remaining = input
        .eligible_indices
        .iter()
        .map(|i| {
            let s = w.stockpiles().at(*i).expect("validated row position");
            (
                s.external_id.clone(),
                (s.quantity - input.held.get(&s.external_id).copied().unwrap_or(0.)).max(0.),
            )
        })
        .collect::<HashMap<_, _>>();
    let mut quote_index = HashMap::new();
    for q in quotes {
        let place = if q.get("province_id").is_none_or(undefined) {
            None
        } else if q["province_id"].is_null() {
            continue;
        } else {
            Some(province(&q["province_id"])?)
        };
        quote_index.insert(
            (place, actor(&q["currency_id"])?, actor(&q["goods_id"])?),
            q,
        );
    }
    let mut result = CapitalForecast {
        entities: vec![],
        groups: vec![],
    };
    let mut group_index = HashMap::new();
    let mut capacities = HashMap::<CapacityKey, f64>::new();
    for e in w.artisans().values() {
        let Some(config) = e
            .base
            .extra
            .get("income_policy")
            .filter(|c| js_truthy(&c["source"]))
        else {
            continue;
        };
        let Some(owner) = &e.base.owner_id else {
            return Err(error("Artisan capital owner missing"));
        };
        let pop = pops.get(owner);
        if e.working_capital.is_none()
            && pop.is_none_or(|p| p.pop_type_id != ExternalId::from("artisans"))
        {
            continue;
        }
        let mid = e
            .pending_retooling
            .as_ref()
            .and_then(|r| r.get("method_id"))
            .filter(|v| js_truthy(v))
            .map(actor)
            .transpose()?
            .unwrap_or_else(|| e.base.method_id.clone());
        let Some(m) = methods.get(&mid) else { continue };
        let currency = e
            .working_capital
            .as_ref()
            .and_then(|c| c.get("currency_id"))
            .filter(|c| js_truthy(c))
            .map(actor)
            .transpose()?
            .or_else(|| pop.and_then(|p| p.currency_id.clone()))
            .or_else(|| currencies.get(owner).map(|c| (*c).clone()));
        let Some(currency) = currency else { continue };
        let mut fte = 0.;
        for a in &e.base.assignments {
            if !a.workers_count.is_finite()
                || !a.time_fraction.is_finite()
                || a.workers_count < 0.
                || a.time_fraction < 0.
            {
                return Err(error("Invalid artisan forecast labor"));
            }
            fte += a.workers_count * a.time_fraction;
        }
        // A deliberately unavailable recipe has no feasible production. Keep
        // its entity/group and authored cash reserve for funding and retooling.
        let mut output = if m.status == "ready" {
            fte * m.weekly_productivity.ok_or_else(|| {
                error(&format!(
                    "Capital productivity unresolved: artisan {}, method {}, province {}",
                    e.base.id.json(),
                    mid.json(),
                    e.base.province_id.0
                ))
            })?
        } else {
            0.
        };
        if !output.is_finite() || output < 0. {
            return Err(error("Invalid capital output"));
        }
        if let Some(sale) = quote_index
            .get(&(
                Some(e.base.province_id),
                currency.clone(),
                m.output_goods_id.clone(),
            ))
            .or_else(|| quote_index.get(&(None, currency.clone(), m.output_goods_id.clone())))
        {
            output = output.min(sale["demand"].as_f64().unwrap_or(0.).max(0.));
        }
        let mut possible = m.status == "ready" && fte > 0.;
        let mut requested = Money::ZERO;
        let mut margin = 0i128;
        let mut plans = vec![];
        for ingredient in &m.inputs {
            if !ingredient.amount.is_finite() || ingredient.amount <= 0. {
                return Err(error("Invalid capital recipe input"));
            }
            let id = (
                owner.clone(),
                e.base.province_id,
                ingredient.goods_id.clone(),
            );
            let own = owned.get(&id).copied().unwrap_or(0.);
            let inbound = incoming.get(&id).copied().unwrap_or(0.);
            let lots = routes.lots(
                owner,
                e.base.province_id,
                &currency,
                &ingredient.goods_id,
                &input,
                &remaining,
                &fleet,
                &capacities,
            )?;
            output = output.min(
                (own + inbound + lots.iter().map(|l| l.quantity).sum::<f64>()) / ingredient.amount,
            );
            plans.push((ingredient, id, own, inbound, lots));
        }
        if !jointly_available(w, &plans, output, &remaining, &fleet, &capacities)? {
            let (mut low, mut high) = (0., output);
            for _ in 0..64 {
                let mid = low + (high - low) / 2.;
                if mid == low || mid == high {
                    break;
                }
                if jointly_available(w, &plans, mid, &remaining, &fleet, &capacities)? {
                    low = mid;
                } else {
                    high = mid;
                }
            }
            output = low;
        }
        if let Some(p) = price(w, &currency, &m.output_goods_id).filter(|_| output > 0.) {
            margin = i128::from(bill(output, p)?.0)
        } else {
            possible = false;
        }
        let mut undo = vec![];
        for (ingredient, id, own, inbound, lots) in plans {
            let goal = output * ingredient.amount;
            let used_own = own.min(goal);
            let used_incoming = inbound.min((goal - used_own).max(0.));
            let mut wanted = (goal - used_own - used_incoming).max(0.);
            let Some(p) = price(w, &currency, &ingredient.goods_id) else {
                possible = false;
                continue;
            };
            margin -= i128::from(bill(goal, p)?.0);
            requested = plus(requested, bill(wanted, p)?)?;
            owned.insert(id.clone(), own - used_own);
            incoming.insert(id.clone(), inbound - used_incoming);
            undo.push(Undo::Owned(id.clone(), used_own));
            undo.push(Undo::Incoming(id, used_incoming));
            for lot in lots {
                if wanted <= 0. {
                    break;
                }
                let stock = w.stockpiles().at(lot.stock).expect("validated row position");
                let space = lot
                    .route
                    .as_ref()
                    .map(|r| {
                        quantity(
                            &r["goods"][js_string(&ingredient.goods_id.json())]["space_per_unit"],
                        )
                    })
                    .transpose()?
                    .unwrap_or(0.);
                let a = lot
                    .route
                    .as_ref()
                    .map(|r| actor(&r["asset_id"]))
                    .transpose()?;
                let mut q = wanted
                    .min(lot.quantity)
                    .min(remaining.get(&stock.external_id).copied().unwrap_or(0.))
                    .min(
                        a.as_ref()
                            .map(|a| fleet.get(a).copied().unwrap_or(0.) / space)
                            .unwrap_or(f64::INFINITY),
                    );
                if let Some(r) = &lot.route {
                    for (key, limit) in route_limits(r)? {
                        q = q.min(
                            (limit - capacities.get(&key).copied().unwrap_or(0.)).max(0.) / space,
                        );
                    }
                }
                remaining.insert(
                    stock.external_id.clone(),
                    remaining.get(&stock.external_id).copied().unwrap_or(0.) - q,
                );
                undo.push(Undo::Source(stock.external_id.clone(), q));
                if let (Some(r), Some(a)) = (&lot.route, a) {
                    for (key, _) in route_limits(r)? {
                        *capacities.entry(key.clone()).or_default() += q * space;
                        undo.push(Undo::Capacity(key, q * space));
                    }
                    fleet.insert(
                        a.clone(),
                        (fleet.get(&a).copied().unwrap_or(0.) - q * space).max(0.),
                    );
                    undo.push(Undo::Fleet(a, q * space));
                    let mapping = &r["goods"][js_string(&ingredient.goods_id.json())];
                    let rate = money(
                        mapping
                            .get("freight_minor_per_space")
                            .filter(|v| !v.is_null() && !undefined(v))
                            .or_else(|| r.get("freight_minor_per_space")),
                    )?;
                    let cost = multiply_money(rate, q * space)?;
                    requested = plus(requested, cost)?;
                    margin -= i128::from(cost.0);
                }
                wanted -= q;
            }
            if wanted > 1e-9 {
                possible = false;
            }
        }
        if margin <= 0 {
            possible = false;
        }
        let available = plus(
            cash.get(&(owner, &currency, "personal_savings"))
                .copied()
                .unwrap_or(Money::ZERO),
            cash.get(&(owner, &currency, "operating_capital"))
                .copied()
                .unwrap_or(Money::ZERO),
        )?;
        let fundable = if possible && requested.0 > 0 {
            (available.0 as f64 / requested.0 as f64).min(1.)
        } else if possible {
            1.
        } else {
            0.
        };
        if fundable < 1. {
            for u in undo {
                match u {
                    Undo::Owned(id, q) => *owned.entry(id).or_default() += q * (1. - fundable),
                    Undo::Incoming(id, q) => {
                        *incoming.entry(id).or_default() += q * (1. - fundable)
                    }
                    Undo::Source(id, q) => *remaining.entry(id).or_default() += q * (1. - fundable),
                    Undo::Fleet(id, q) => *fleet.entry(id).or_default() += q * (1. - fundable),
                    Undo::Capacity(id, q) => {
                        *capacities.entry(id).or_default() -= q * (1. - fundable)
                    }
                }
            }
        }
        let authored = money(config.get("cash_reserve_minor"))?;
        let minimum = if possible {
            Money(authored.0.max(requested.0))
        } else {
            authored
        };
        result.entities.push(CapitalEntity {
            id: e.base.id.clone(),
            owner_id: owner.clone(),
            currency_id: currency.clone(),
            minimum_minor: minimum,
            requested_minor: requested,
            possible,
            active: fte > 0.,
            feasible_output: output,
        });
        let key = (owner.clone(), currency.clone());
        let index = if let Some(index) = group_index.get(&key) {
            *index
        } else {
            let i = result.groups.len();
            result.groups.push(CapitalGroup {
                owner_id: owner.clone(),
                currency_id: currency.clone(),
                minimum_minor: Money::ZERO,
                ids: vec![],
                active: false,
                personal_funded_minor: Money::ZERO,
                retained_profit_minor: Money::ZERO,
            });
            group_index.insert(key, i);
            i
        };
        let g = &mut result.groups[index];
        g.minimum_minor = plus(g.minimum_minor, minimum)?;
        g.ids.push(e.base.id.clone());
        g.active |= fte > 0.;
        g.personal_funded_minor = plus(
            g.personal_funded_minor,
            money(
                e.working_capital
                    .as_ref()
                    .and_then(|c| c.get("personal_funded_minor")),
            )?,
        )?;
        g.retained_profit_minor = plus(
            g.retained_profit_minor,
            money(e.base.extra.get("undistributed_profit_minor"))?,
        )?;
    }
    Ok(result)
}
