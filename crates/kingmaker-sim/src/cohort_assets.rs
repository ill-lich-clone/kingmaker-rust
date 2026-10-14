//! Sparse title changes; a profession change never moves physical goods.
use kingmaker_core::row_store::ReadSequence;
use kingmaker_core::world_read::WorldRead;
use crate::{budget_funding::error, operation_session::OpResult};
use kingmaker_core::{market::*, production::*, storage::*, *};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

const REFERENCES: [&str; 14] = [
    "owner_id",
    "owner_entity_id",
    "buyer_id",
    "seller_id",
    "pop_id",
    "payer_id",
    "payee_id",
    "beneficiary_id",
    "beneficiary_pop_id",
    "employer_entity_id",
    "sponsor_id",
    "accepted_buyer_id",
    "accepted_seller_id",
    "accepted_carrier_id",
];
pub(crate) fn text(id: &ExternalId) -> String {
    id.as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| id.json().to_string())
}
fn owns(v: &Value, id: &ExternalId) -> bool {
    [
        "owner_id",
        "owner_entity_id",
        "investor_id",
        "beneficiary_id",
        "pop_id",
    ]
    .iter()
    .any(|key| v.get(key) == Some(&id.json()))
        || v.get("beneficiaries")
            .and_then(Value::as_array)
            .is_some_and(|rows| rows.iter().any(|v| owns(v, id)))
}
pub(crate) fn remap(
    v: &mut Value,
    parent: &ExternalId,
    child: &ExternalId,
    stocks: &HashMap<ExternalId, ExternalId>,
) -> bool {
    let mut changed = false;
    match v {
        Value::Object(row) => {
            for (key, value) in row {
                if REFERENCES.contains(&key.as_str()) && *value == parent.json() {
                    *value = child.json();
                    changed = true;
                } else if key == "stock_id" {
                    if let Ok(id) = ExternalId::from_json(value.clone()) {
                        if let Some(new) = stocks.get(&id) {
                            *value = new.json();
                            changed = true;
                        }
                    }
                } else {
                    changed |= remap(value, parent, child, stocks);
                }
            }
        }
        Value::Array(rows) => {
            for value in rows.iter_mut() {
                changed |= remap(value, parent, child, stocks);
            }
        }
        _ => {}
    }
    changed
}
/// Only changed rows are copied. Their original row is retained for closed publication checks.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RowChange<T> {
    pub position: usize,
    pub before: T,
    pub after: T,
}
fn changes<T: Serialize + DeserializeOwned + Clone>(
    rows: &(impl ReadSequence<T> + ?Sized),
    parent: &ExternalId,
    child: &ExternalId,
    stocks: &HashMap<ExternalId, ExternalId>,
) -> OpResult<Vec<RowChange<T>>> {
    let mut result = vec![];
    for (position, row) in rows.iter().enumerate() {
        let mut v = serde_json::to_value(row)?;
        if remap(&mut v, parent, child, stocks) {
            result.push(RowChange {
                position,
                before: row.clone(),
                after: serde_json::from_value(v)?,
            });
        }
    }
    Ok(result)
}
pub(crate) fn install<T>(rows: &mut [T], changes: Vec<RowChange<T>>) {
    for c in changes {
        rows[c.position] = c.after;
    }
}
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct AssetPlan {
    pub blocked: Option<String>,
    pub held: HashMap<ExternalId, f64>,
    pub retained: HashSet<ExternalId>,
    pub facilities: Vec<RowChange<StorageFacility>>,
    pub storage: Vec<RowChange<StorageReservation>>,
    pub goods: Vec<RowChange<GoodsReservation>>,
    pub shipments: Vec<RowChange<Value>>,
    pub recovery_plans: Vec<RowChange<Value>>,
    pub artisans: Vec<RowChange<ArtisanEnterprise>>,
    pub contracts: Vec<RowChange<LaborContract>>,
    pub jobs: Vec<RowChange<Value>>,
    pub sales: Vec<RowChange<SaleOffer>>,
    pub maintenance: Vec<RowChange<MaintenanceAgreement>>,
}
impl AssetPlan {
    fn blocked(reason: &str) -> Self {
        Self {
            blocked: Some(reason.into()),
            ..Default::default()
        }
    }
    pub fn install(self, w: &mut WorldState) {
        install(&mut w.production_data.storage_facilities, self.facilities);
        install(&mut w.production_data.storage_reservations, self.storage);
        install(&mut w.production_data.goods_reservations, self.goods);
        install(&mut w.mobility_data.shipments, self.shipments);
        install(
            &mut w.transport_data.shipment_recovery_plans,
            self.recovery_plans,
        );
        install(&mut w.production_data.artisans, self.artisans);
        install(&mut w.production_data.contracts, self.contracts);
        install(&mut w.budget_data.jobs, self.jobs);
        install(&mut w.market_data.sale_offers, self.sales);
        install(&mut w.market_data.maintenance_agreements, self.maintenance);
    }
    pub(crate) fn install_in(self,w:&mut crate::world_transaction::WorldTransaction<'_>)->OpResult<()> {
        macro_rules! install {
            ($rows:ident,$put:ident,$changes:expr) => {for c in $changes {
                let key=w.$rows().keyed().nth(c.position).ok_or_else(||error("Missing title row"))?.0;
                w.$put(key,c.after)?;
            }};
        }
        install!(storage_facilities,put_storage_facilities,self.facilities);
        install!(storage_reservations,put_storage_reservations,self.storage);
        install!(goods_reservations,put_goods_reservations,self.goods);
        install!(shipments,put_shipments,self.shipments);
        install!(shipment_recovery_plans,put_shipment_recovery_plans,self.recovery_plans);
        install!(artisans,put_artisans,self.artisans);
        install!(contracts,put_contracts,self.contracts);
        install!(jobs,put_jobs,self.jobs);
        install!(sale_offers,put_sale_offers,self.sales);
        install!(maintenance_agreements,put_maintenance_agreements,self.maintenance);
        Ok(())
    }

}
pub(crate) fn plan_assets(
    w: &dyn WorldRead,
    parent: &PopRecord,
    child: &ExternalId,
    full: bool,
) -> OpResult<AssetPlan> {
    let id = &parent.external_id;
    if w.money_reservations().values()
        .any(|r| &r.owner_id == id)
    {
        return Ok(AssetPlan::blocked("asset_reservation"));
    }
    for s in w.shipments().values() {
        let party = ["owner_id", "buyer_id", "seller_id", "pop_id"]
            .iter()
            .any(|key| s.get(key) == Some(&id.json()));
        if party
            && !matches!(
                s["status"].as_str(),
                Some("delivered" | "cancelled" | "returned" | "lost" | "closed" | "arrived")
            )
        {
            if !s.get("currency_id").is_some_and(crate::needs::js_truthy)
                || s.get("goods_price_minor")
                    .is_none_or(|v| v.is_null() || crate::needs::undefined(v))
                || !s["remaining_quantity"].as_f64().is_some_and(f64::is_finite)
                || !s
                    .get("storage_reservation_id")
                    .is_some_and(crate::needs::js_truthy)
            {
                return Ok(AssetPlan::blocked("pending_shipment"));
            }
        }
    }
    let mut out = AssetPlan::default();
    let owned: HashSet<_> = w.stockpiles().values()
        .filter(|s| &s.owner_id == id)
        .map(|s| &s.external_id)
        .collect();
    for g in w.goods_reservations().values() {
        if g.status == "held" && owned.contains(&g.stock_id) {
            if !g.quantity.is_finite() || g.quantity < 0. {
                return Err(error("Invalid cohort goods reservation"));
            }
            *out.held.entry(g.stock_id.clone()).or_default() += g.quantity;
        }
    }
    let enterprises: Vec<_> = w.artisans().values()
        .filter(|a| a.base.owner_id.as_ref() == Some(id))
        .collect();
    for s in w.stockpiles().values().filter(|s| &s.owner_id == id) {
        if !s.quantity.is_finite() || s.quantity < 0. {
            return Err(error("Invalid cohort stock"));
        }
        if out.held.get(&s.external_id).copied().unwrap_or(0.) > s.quantity + 1e-9 {
            return Ok(AssetPlan::blocked("goods_reservation"));
        }
        if !full
            && s.purpose != "food_reserve"
            && enterprises.iter().any(|a| {
                a.base.province_id == s.province_id
                    && (s.inventory_origin.as_deref() == Some("produced")
                        || w.production_catalog().as_ref().is_some_and(|c| {
                            c.methods.iter().any(|m| {
                                m.method_id == a.base.method_id
                                    && m.inputs.iter().any(|i| i.goods_id == s.goods.external_id())
                            })
                        }))
            })
        {
            out.retained.insert(s.external_id.clone());
        }
    }
    for rows in [
        &w.capitalist_entities(),
        &w.project_investments(),
        &w.production_beneficiaries(),
        &w.factory_beneficiaries(),
    ] {
        if rows.iter().any(|v| owns(v, id)) {
            return Ok(AssetPlan::blocked("linked_property_rights"));
        }
    }
    for f in w.storage_facilities().values() {
        for c in f
            .access_contracts
            .iter()
            .filter(|c| c.owner_id.as_ref() == Some(id))
        {
            if c.price_minor_per_space.is_none()
                || c.currency_id.is_none()
                || c.purpose.is_none()
                || c.source.is_none()
                || !matches!(c.status.as_deref(), Some("approved" | "source"))
                || c.status.as_deref() == Some("approved") && c.approval_ref.is_none()
            {
                return Ok(AssetPlan::blocked("linked_storage_rights"));
            }
            if ["fixed_fee_minor", "monthly_fee_minor"].iter().any(|key| {
                c.extra
                    .get(*key)
                    .is_some_and(|v| !v.is_null() && !crate::needs::undefined(v))
            }) {
                return Ok(AssetPlan::blocked("indivisible_storage_contract"));
            }
        }
    }
    if full {
        let stocks = w.stockpiles().values()
            .filter(|s| &s.owner_id == id)
            .map(|s| {
                (
                    s.external_id.clone(),
                    format!("{}:{}", text(&s.external_id), text(child)).into(),
                )
            })
            .collect();
        out.facilities = changes(&w.storage_facilities(), id, child, &stocks)?;
        out.storage = changes(&w.storage_reservations(), id, child, &stocks)?;
        out.goods = changes(&w.goods_reservations(), id, child, &stocks)?;
        out.shipments = changes(&w.shipments(), id, child, &stocks)?;
        out.recovery_plans = changes(
            &w.shipment_recovery_plans(),
            id,
            child,
            &stocks,
        )?;
        // destination_stock_id has a shipment-specific default in the source rules.
        for (position, s) in w.shipments().values().enumerate() {
            let stock = s
                .get("destination_stock_id")
                .filter(|v| crate::needs::js_truthy(v))
                .cloned()
                .unwrap_or_else(|| {
                    json!(format!(
                        "shipment:{}:destination",
                        s["id"]
                            .as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| s["id"].to_string())
                    ))
                });
            if let Ok(stock) = ExternalId::from_json(stock) {
                if let Some(new) = stocks.get(&stock) {
                    if let Some(c) = out.shipments.iter_mut().find(|c| c.position == position) {
                        c.after["destination_stock_id"] = new.json();
                    } else {
                        let mut after = s.clone();
                        after["destination_stock_id"] = new.json();
                        out.shipments.push(RowChange {
                            position,
                            before: s.clone(),
                            after,
                        });
                    }
                }
            }
        }
        out.artisans = changes(&w.artisans(), id, child, &stocks)?;
        out.contracts = changes(&w.contracts(), id, child, &stocks)?;
        out.jobs = changes(&w.jobs(), id, child, &stocks)?;
        out.sales = changes(&w.sale_offers(), id, child, &stocks)?;
        out.maintenance = changes(&w.maintenance_agreements(), id, child, &stocks)?;
    } else {
        for (position, f) in w.storage_facilities().values().enumerate() {
            let mut after = f.clone();
            if &f.owner_id == id
                && !f
                    .access_contracts
                    .iter()
                    .any(|c| c.owner_id.as_ref() == Some(child))
            {
                after.access_contracts.push(serde_json::from_value(json!({"owner_id":child,"status":"approved","source":"approved cohort warehouse access","approval_ref":"economic-recovery:2026-10-12","currency_id":parent.currency_id,"purpose":"personal_savings","price_minor_per_space":"0","cohort_source_owner":id}))?);
            } else {
                for c in f
                    .access_contracts
                    .iter()
                    .filter(|c| c.owner_id.as_ref() == Some(id))
                {
                    let mut next = c.clone();
                    next.owner_id = Some(child.clone());
                    next.extra.insert("cohort_source_owner".into(), id.json());
                    after.access_contracts.push(next);
                }
            }
            if after != *f {
                out.facilities.push(RowChange {
                    position,
                    before: f.clone(),
                    after,
                });
            }
        }
    }
    Ok(out)
}
