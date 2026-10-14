//! Real stock standing offers; authored manual rows and private input rights stay intact.
use kingmaker_core::world_read::WorldRead;
use crate::{
    budget_funding::error,
    market_operations::invoice,
    money_math::decimal_ratio,
    needs::{js_string, js_truthy},
    operation_session::OpResult,
};
use kingmaker_core::{market::SaleOffer, *};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
pub fn plan_trade_offers(w: &dyn WorldRead, period: &str) -> OpResult<Vec<SaleOffer>> {
    if period.is_empty() {
        return Err(error("Standing offers require a period"));
    }
    let pops = w.pops().values()
        .map(|p| &p.external_id)
        .collect::<HashSet<_>>();
    let methods = w.production_catalog()
        .as_ref()
        .into_iter()
        .flat_map(|c| &c.methods)
        .map(|m| (&m.method_id, m))
        .collect::<HashMap<_, _>>();
    let mut allowed = HashSet::new();
    let mut raw = HashSet::new();
    let mut inputs = HashSet::new();
    let mut purpose = HashMap::new();
    let public = w.storage_facilities().values()
        .filter(|f| {
            f.extra.get("state_raw_warehouse").is_some_and(js_truthy)
                && js_string(&f.owner_id.json()).starts_with("treasury:")
        })
        .map(|f| (&f.id, &f.owner_id))
        .collect::<HashMap<_, _>>();
    for owner in public.values() {
        purpose.insert((*owner).clone(), "operating_capital".to_owned());
    }
    for e in w.resource_operations().values()
        .map(|e| &e.base)
        .chain(w.factories().values().map(|e| &e.base))
        .chain(w.artisans().values().map(|e| &e.base))
    {
        let Some(owner) = &e.owner_id else { continue };
        let Some(m) = methods.get(&e.method_id) else {
            continue;
        };
        let k = (owner.clone(), e.province_id, m.output_goods_id.clone());
        allowed.insert(k.clone());
        if m.kind == "rgo" {
            raw.insert(k);
        }
        if let Some((food, _)) = crate::food_conversion::recipe(&m.output_goods_id) {
            allowed.insert((owner.clone(), e.province_id, food.into()));
        }
        for i in &m.inputs {
            inputs.insert((owner.clone(), e.province_id, i.goods_id.clone()));
        }
        purpose.insert(
            owner.clone(),
            e.payment_purpose
                .clone()
                .filter(|p| !p.is_empty())
                .unwrap_or_else(|| {
                    if m.kind == "artisan" {
                        "operating_capital".into()
                    } else {
                        "employer_operating_capital".into()
                    }
                }),
        );
    }
    let mut offers = w.sale_offers().values()
        .filter(|o| !o.extra.get("automatic").is_some_and(js_truthy))
        .cloned()
        .collect::<Vec<_>>();
    let mut ids = offers
        .iter()
        .filter_map(|o| o.id.clone())
        .collect::<HashSet<_>>();
    for s in w.stockpiles().values() {
        let good = s.goods.external_id();
        let k = (s.owner_id.clone(), s.province_id, good.clone());
        if s.purpose == "food_reserve"
            || js_string(&s.owner_id.json()).starts_with("treasury:")
                && s.facility_id
                    .as_ref()
                    .and_then(|id| public.get(id))
                    .copied()
                    != Some(&s.owner_id)
            || !s.quantity.is_finite()
            || s.quantity <= 0.
            || pops.contains(&s.owner_id) && s.inventory_origin.as_deref() != Some("produced")
            || !allowed.contains(&k)
                && !(purpose.contains_key(&s.owner_id)
                    && (!pops.contains(&s.owner_id)
                        || s.inventory_origin.as_deref() == Some("produced")))
            || inputs.contains(&k) && !raw.contains(&k)
        {
            continue;
        }
        let Some(currency) = s.currency_id.as_ref().filter(|c| js_truthy(&c.json())) else {
            continue;
        };
        let Some(unit) = s.unit.as_ref().filter(|v| js_truthy(v)) else {
            continue;
        };
        let Some(quote) = w.price_calibration()
            .as_ref()
            .and_then(|p| p.prices_by_currency.get(&js_string(&currency.json())))
            .and_then(|p| p.get(&js_string(&good.json())))
            .and_then(Value::as_f64)
            .filter(|q| q.is_finite() && *q > 0.)
        else {
            continue;
        };
        let id: ExternalId = format!("standing-sale:{}", js_string(&s.external_id.json())).into();
        if !ids.insert(id.clone()) {
            return Err(error(
                "Conflicting standing-sale identity; stock IDs must not alias",
            ));
        }
        let (n, d) = decimal_ratio(&js_string(&json!(quote)))?;
        offers.push(serde_json::from_value(json!({"id":id,"automatic":true,"status":"approved","approval_ref":"preview-migration-not-live","source":"retained enterprise owner standing output offer:Victoria2 adaptation","stock_id":s.external_id,"seller_id":s.owner_id,"seller_purpose":purpose.get(&s.owner_id),"goods_id":good,"unit":unit,"currency_id":currency,"price_minor_per_unit":invoice(1.,quote)?.to_string(),"price_minor_rate":{"numerator":(n*100u32).to_string(),"denominator":d.to_string()},"period":period}))?);
    }
    Ok(offers)
}
