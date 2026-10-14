//! Private paid dispatch. One session shares cash, stock, warehouse, fleet and weekly capacity.
use kingmaker_core::world_read::WorldRead;
use crate::{
    budget_funding::error,
    calendar::{add_days, serial_date},
    market_policy::protected_artisan_inputs,
    money_math::{multiply_money, rate_pay},
    operation_session::{OpResult, OperationSession, PreparedChanges},
    shipping_operations::stage_shipping_operations,
    storage::{approved, reserve_storage, StorageBatch, StorageRequest},
    transport_validation::{actor, province, RouteValidation},
};
use kingmaker_core::{calendar::GameDate, market::SaleOffer, operations::*, *};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ShipmentOrder {
    pub id: ExternalId,
    pub buyer_id: ExternalId,
    pub purpose: String,
    pub offer_id: ExternalId,
    pub route_id: ExternalId,
    pub destination_facility_id: ExternalId,
    pub quantity: f64,
    pub inventory_purpose: Option<String>,
    pub pop_id: Option<ExternalId>,
}
#[derive(Debug, Clone)]
pub(crate) struct DispatchOutcome {
    pub quantity: f64,
    pub reason: Option<String>,
    pub replayed: bool,
}
pub(crate) struct DispatchPlan {
    pub changes: PreparedChanges,
    pub shipments: Vec<Value>,
    pub route_reservations: Vec<Value>,
    pub routes: Vec<Value>,
    pub operations: Vec<EconomicOperation>,
}
type CapacityKey = (ExternalId, u8, ExternalId);
pub(crate) struct DispatchSession<'a> {
    pub session: OperationSession<'a>,
    pub storage: StorageBatch<'a>,
    period: String,
    ships: Vec<Value>,
    reservations: Vec<Value>,
    routes: Vec<Value>,
    offers: HashMap<ExternalId, &'a SaleOffer>,
    busy: HashMap<ExternalId, f64>,
    used: HashMap<CapacityKey, f64>,
    held: HashMap<ExternalId, f64>,
    parts: HashMap<ExternalId, HashMap<ExternalId, f64>>,
    validation: RouteValidation<'a>,
}
fn qty(v: &Value) -> OpResult<f64> {
    v.as_f64()
        .filter(|n| n.is_finite() && *n >= 0.)
        .ok_or_else(|| error("Invalid physical dispatch quantity"))
}
fn money(v: &Value) -> OpResult<Money> {
    let n: Money = serde_json::from_value(v.clone())?;
    if n.0 < 0 {
        return Err(error("Negative dispatch quote"));
    }
    Ok(n)
}
fn rejected(reason: &str) -> DispatchOutcome {
    DispatchOutcome {
        quantity: 0.,
        reason: Some(reason.into()),
        replayed: false,
    }
}
fn terminals(route: &Value) -> OpResult<[CapacityKey; 3]> {
    let unit = actor(&route["unit"])?;
    let terminal = |key: &str, place: &str| {
        route
            .get(key)
            .filter(|v| crate::needs::js_truthy(v))
            .map(actor)
            .unwrap_or_else(|| {
                Ok(format!("province:{}", crate::needs::js_string(&route[place])).into())
            })
    };
    Ok([
        (unit.clone(), 0, actor(&route["id"])?),
        (
            unit.clone(),
            1,
            terminal("source_terminal_id", "from_province_id")?,
        ),
        (
            unit,
            2,
            terminal("destination_terminal_id", "to_province_id")?,
        ),
    ])
}
impl<'a> DispatchSession<'a> {
    pub(crate) fn new(w: &'a dyn WorldRead, period: &str) -> OpResult<Self> {
        if period.is_empty() {
            return Err(error("Dispatch period missing"));
        }
        let session = OperationSession::new(w)?;
        let storage = StorageBatch::new(&session)?;
        let today = serial_date(GameDate::from(w.campaign()))?;
        let mut busy = HashMap::<ExternalId, f64>::new();
        for c in w.shipments().values() {
            if qty(&c["remaining_quantity"])? > 0.
                || serial_date(serde_json::from_value(c["carrier_release_date"].clone())?)? > today
            {
                *busy.entry(actor(&c["asset_id"])?).or_default() +=
                    qty(&c["shipped_quantity"])? * qty(&c["space_per_unit"])?;
            }
        }
        let mut held = protected_artisan_inputs(w)?;
        for r in w.goods_reservations().values() {
            if r.status == "held" {
                *held.entry(r.stock_id.clone()).or_default() += r.quantity;
            }
        }
        let mut used = HashMap::<CapacityKey, f64>::new();
        for r in w.route_reservations().values() {
            if r["period"] == json!(period) {
                let unit = actor(&r["unit"])?;
                let q = qty(&r["space"])?;
                for (kind, key) in [
                    (0, "route_id"),
                    (1, "source_terminal_id"),
                    (2, "destination_terminal_id"),
                ] {
                    *used
                        .entry((unit.clone(), kind, actor(&r[key])?))
                        .or_default() += q;
                }
            }
        }
        let mut parts = HashMap::<ExternalId, HashMap<ExternalId, f64>>::new();
        for p in crate::stock_provenance::capture_stock_provenance(w)? {
            parts
                .entry(p.stock_id)
                .or_default()
                .insert(p.producer_id, p.quantity);
        }
        let offers = w.sale_offers().values()
            .filter_map(|o| o.id.clone().map(|id| (id, o)))
            .collect::<HashMap<_, _>>();
        Ok(Self {
            session,
            storage,
            period: period.into(),
            ships: w.shipments().values().cloned().collect::<Vec<_>>(),
            reservations: w.route_reservations().values().cloned().collect::<Vec<_>>(),
            routes: w.supply_routes().values().cloned().collect::<Vec<_>>(),
            offers,
            busy,
            used,
            held,
            parts,
            validation: RouteValidation::new(w)?,
        })
    }
    pub(crate) fn add_route(&mut self, route: Value) -> OpResult<()> {
        if let Some(old) = self.routes.iter().find(|r| r["id"] == route["id"]) {
            if *old != route {
                return Err(error("Adapted route identity conflict"));
            }
            return Ok(());
        }
        self.routes.push(route);
        Ok(())
    }
    pub(crate) fn dispatch(&mut self, order: &ShipmentOrder) -> OpResult<DispatchOutcome> {
        let result = self.dispatch_inner(order);
        if result.is_err() {
            self.session.poison();
        }
        result
    }
    fn dispatch_inner(&mut self, o: &ShipmentOrder) -> OpResult<DispatchOutcome> {
        if o.purpose.is_empty() || !o.quantity.is_finite() || o.quantity <= 0. {
            return Err(error("Invalid physical supply order"));
        }
        let identity = serde_json::to_string(&json!([
            o.buyer_id,
            o.purpose,
            o.offer_id,
            o.route_id,
            o.destination_facility_id,
            crate::needs::js_quantity_value(o.quantity)
        ]))?;
        if let Some(old) = self.ships.iter().find(|s| s["id"] == o.id.json()) {
            if old["order_identity"] != json!(identity) {
                return Err(error("Supply order identity mismatch"));
            }
            return Ok(DispatchOutcome {
                quantity: qty(&old["shipped_quantity"])?,
                reason: None,
                replayed: true,
            });
        }
        let Some(offer) = self.offers.get(&o.offer_id).copied() else {
            return Ok(rejected("seller_offer_unknown"));
        };
        if !approved(
            offer.extra.get("status").and_then(Value::as_str),
            offer.extra.get("source"),
            offer.extra.get("approval_ref"),
        ) || offer
            .extra
            .get("price_minor_per_unit")
            .is_none_or(Value::is_null)
            || offer.extra.get("currency_id").is_none_or(Value::is_null)
            || offer.seller_purpose.is_none()
        {
            return Ok(rejected("seller_offer_unknown"));
        }
        let Some(route) = self
            .routes
            .iter()
            .find(|r| r["id"] == o.route_id.json())
            .cloned()
        else {
            return Ok(rejected("transport_parameters_unknown"));
        };
        if let Some(reason) = self.validation.reason(&route, offer) {
            return Ok(rejected(reason));
        }
        let Some(stock) = self.session.stock(&offer.stock_id).cloned() else {
            return Ok(rejected("supply_stock_or_destination_mismatch"));
        };
        let Some(facility) = self
            .storage
            .facilities
            .get(&o.destination_facility_id)
            .cloned()
        else {
            return Ok(rejected("supply_stock_or_destination_mismatch"));
        };
        if stock.owner_id != offer.seller_id
            || stock.goods.external_id() != offer.goods_id
            || stock.unit.as_ref() != offer.extra.get("unit")
            || stock.province_id != province(&route["from_province_id"])?
            || facility.province_id != province(&route["to_province_id"])?
        {
            return Ok(rejected("supply_stock_or_destination_mismatch"));
        }
        let available = self.storage.availability(&facility.id, &offer.goods_id);
        let Some(free) = available.free else {
            return Ok(rejected("destination_storage_unknown"));
        };
        let currency = actor(offer.extra.get("currency_id").unwrap())?;
        let lease = facility.access_contracts.iter().find(|c| {
            c.owner_id.as_ref() == Some(&o.buyer_id)
                && approved(
                    c.status.as_deref(),
                    c.source.as_ref(),
                    c.approval_ref.as_ref(),
                )
        });
        if facility.owner_id != o.buyer_id
            && lease.is_none_or(|l| {
                l.currency_id.as_ref() != Some(&currency)
                    || l.purpose.as_deref() != Some(o.purpose.as_str())
                    || l.price_minor_per_space.is_none()
            })
        {
            return Ok(rejected("destination_lease_unavailable"));
        }
        let mapping = &route["goods"][crate::needs::js_string(&offer.goods_id.json())];
        let space = qty(&mapping["space_per_unit"])?;
        let price = money(offer.extra.get("price_minor_per_unit").unwrap())?;
        let rate = offer
            .extra
            .get("price_minor_rate")
            .filter(|v| !v.is_null())
            .map(|v| serde_json::from_value::<kingmaker_core::production::MoneyRate>(v.clone()))
            .transpose()?;
        let freight = money(
            mapping
                .get("freight_minor_per_space")
                .filter(|v| !v.is_null())
                .unwrap_or(&route["freight_minor_per_space"]),
        )?;
        let rent = lease
            .and_then(|l| l.price_minor_per_space)
            .unwrap_or(Money::ZERO);
        if rent.0 < 0 {
            return Err(error("Negative supply rent quote"));
        }
        let asset = actor(&route["asset_id"])?;
        let capacity = self
            .session
            .base.transport_assets().values()
            .find(|a| a["id"] == asset.json())
            .ok_or_else(|| error("Carrier asset missing"))?;
        let keys = terminals(&route)?;
        let limits = [
            qty(&route["capacity_per_week"])?,
            qty(&route["source_handling_per_week"])?,
            qty(&route["destination_handling_per_week"])?,
        ];
        let mut q = o
            .quantity
            .min(
                (stock.quantity - self.held.get(&stock.external_id).copied().unwrap_or(0.)).max(0.),
            )
            .min(free)
            .min(
                (qty(&capacity["capacity"])? - self.busy.get(&asset).copied().unwrap_or(0.))
                    .max(0.)
                    / space,
            );
        for (key, limit) in keys.iter().zip(limits) {
            q = q.min((limit - self.used.get(key).copied().unwrap_or(0.)).max(0.) / space);
        }
        let goods_invoice = |q: f64| -> OpResult<Money> {
            Ok(if let Some(rate) = &rate {
                rate_pay(rate, q, 1.)?
            } else {
                multiply_money(price, q)?
            })
        };
        let cost = |q: f64| -> OpResult<Money> {
            let goods = goods_invoice(q)?.0;
            let transit = multiply_money(freight, q * space)?.0;
            let warehouse = multiply_money(
                rent,
                q * facility.goods[&crate::needs::js_string(&offer.goods_id.json())].space_per_unit,
            )?
            .0;
            Ok(Money(
                goods
                    .checked_add(transit)
                    .and_then(|n| n.checked_add(warehouse))
                    .ok_or_else(|| error("Supply total cost overflow"))?,
            ))
        };
        let cash = self.session.cash(&o.buyer_id, &currency, &o.purpose);
        // The autonomous source reducer never attempts another buy after purpose cash reaches zero.
        if cash.0 <= 0 {
            return Ok(rejected("supply_stock_capacity_or_cash_unavailable"));
        }
        if cost(q)? > cash {
            let mut low = 0.;
            let mut high = q;
            for _ in 0..64 {
                let mid = low + (high - low) / 2.;
                if mid == low || mid == high {
                    break;
                }
                if cost(mid)? <= cash {
                    low = mid
                } else {
                    high = mid
                }
            }
            q = low;
            if cost(q)? > cash {
                return Err(error("Supply affordability invariant failed"));
            }
        }
        if q <= 0. {
            return Ok(rejected("supply_stock_capacity_or_cash_unavailable"));
        }
        let reservation_id = ExternalId::from(format!(
            "shipment:{}:storage",
            crate::needs::js_string(&o.id.json())
        ));
        let reserved = reserve_storage(
            &mut self.session,
            &mut self.storage,
            &StorageRequest {
                id: reservation_id.clone(),
                owner_id: o.buyer_id.clone(),
                facility_id: facility.id.clone(),
                goods_id: offer.goods_id.clone(),
                unit: offer.extra["unit"].clone(),
                quantity: q,
            },
            &self.period,
        )?;
        let Some(reserved) = reserved.reservation else {
            return Ok(rejected(
                reserved
                    .reason
                    .as_deref()
                    .unwrap_or("destination_storage_blocked"),
            ));
        };
        if (reserved.quantity - q).abs() > 1e-8 {
            return Err(error("Supply preflight/storage reservation mismatch"));
        }
        let goods_price = goods_invoice(q)?;
        let freight_paid = multiply_money(freight, q * space)?;
        let today = GameDate::from(self.session.base.campaign());
        let days = route["travel_days"]
            .as_i64()
            .ok_or_else(|| error("Invalid route travel days"))?;
        let returns = route["return_days"]
            .as_i64()
            .ok_or_else(|| error("Invalid carrier return days"))?;
        let mut shipment = json!({"id":o.id,"order_identity":identity,"buyer_id":o.buyer_id,"buyer_purpose":o.purpose,"seller_id":offer.seller_id,"seller_purpose":offer.seller_purpose,"stock_id":stock.external_id,"goods_id":offer.goods_id,"unit":offer.extra["unit"],"currency_id":currency,"route_id":route["id"],"asset_id":asset,"carrier_id":route["carrier_id"],"from_province_id":route["from_province_id"],"to_province_id":route["to_province_id"],"facility_id":facility.id,"storage_reservation_id":reservation_id,"shipped_quantity":q,"remaining_quantity":q,"arrived_quantity":0,"space_per_unit":space,"goods_price_minor":goods_price.0.to_string(),"paid_to_seller_minor":"0","freight_paid_minor":freight_paid.0.to_string(),"departed_date":today,"arrival_date":add_days(today,days)?,"carrier_release_date":add_days(today,days+returns)?,"return_days":returns,"status":"in_transit","provenance":offer.extra["source"],"goods_origin":stock.inventory_origin.as_deref().unwrap_or("unresolved"),"arrival_receipts":[]});
        if let Some(purpose) = &o.inventory_purpose {
            shipment["inventory_purpose"] = json!(purpose);
            shipment["beneficiary_pop_id"] = json!(o.pop_id);
        }
        if let Some(parts) = self.parts.get_mut(&stock.external_id) {
            let mut contributors = vec![];
            for (id, amount) in parts {
                let moved = *amount * q / stock.quantity;
                *amount -= moved;
                if moved > 0. {
                    contributors.push(json!({"producer_id":id,"quantity":moved}));
                }
            }
            contributors.sort_by(|a, b| {
                crate::ordering::utf16_compare(
                    &crate::needs::js_string(&a["producer_id"]),
                    &crate::needs::js_string(&b["producer_id"]),
                )
            });
            shipment["producer_contributions"] = json!(contributors);
        }
        let mut op = json!({"event_id":format!("supply:{}:{}",self.session.base.campaign().campaign_id,crate::needs::js_string(&o.id.json())),"type":"shipment_dispatch","shipment_id":o.id,"debits":[],"credits":[],"stock_changes":[{"stock_id":stock.external_id,"delta":-q}],"seller_income_recognized":false});
        let payment = goods_price
            .0
            .checked_add(freight_paid.0)
            .ok_or_else(|| error("Dispatch invoice overflow"))?;
        if payment > 0 {
            op["debits"] = json!([{"owner_id":o.buyer_id,"currency_id":currency,"purpose":o.purpose,"amount_minor":payment.to_string()}]);
        }
        let mut credits = vec![];
        if goods_price.0 > 0 {
            credits.push(json!({"owner_id":format!("shipment:{}",crate::needs::js_string(&o.id.json())),"currency_id":currency,"purpose":"trade_escrow","amount_minor":goods_price.0.to_string()}));
        }
        if freight_paid.0 > 0 {
            credits.push(json!({"owner_id":route["carrier_id"],"currency_id":currency,"purpose":"operating_capital","amount_minor":freight_paid.0.to_string()}));
        }
        op["credits"] = json!(credits);
        let op: EconomicOperation = serde_json::from_value(op)?;
        stage_shipping_operations(&mut self.session, std::slice::from_ref(&op))?;
        if let Some(f) = stock.facility_id.as_ref() {
            if let Some(mapping) = self.storage.facilities.get(f).and_then(|f| {
                f.goods
                    .get(&crate::needs::js_string(&offer.goods_id.json()))
            }) {
                self.storage.adjust(f, -q * mapping.space_per_unit)?;
            }
        }
        self.ships.push(shipment);
        *self.busy.entry(asset).or_default() += q * space;
        for key in keys {
            *self.used.entry(key).or_default() += q * space;
        }
        self.reservations.push(json!({"id":o.id,"period":self.period,"route_id":route["id"],"unit":route["unit"],"space":q*space,"source_terminal_id":keys_terminal(&route,"source_terminal_id","from_province_id"),"destination_terminal_id":keys_terminal(&route,"destination_terminal_id","to_province_id")}));
        Ok(DispatchOutcome {
            quantity: q,
            reason: None,
            replayed: false,
        })
    }
    pub(crate) fn finish(self) -> OpResult<DispatchPlan> {
        let operations = self.session.operations().to_vec();
        let changes = self.session.finish()?;
        Ok(DispatchPlan {
            changes,
            shipments: self.ships,
            route_reservations: self.reservations,
            routes: self.routes,
            operations,
        })
    }
}
fn keys_terminal(route: &Value, key: &str, province: &str) -> Value {
    route
        .get(key)
        .filter(|v| crate::needs::js_truthy(v))
        .cloned()
        .unwrap_or_else(|| {
            json!(format!(
                "province:{}",
                crate::needs::js_string(&route[province])
            ))
        })
}
