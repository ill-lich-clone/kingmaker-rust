use kingmaker_core::world_read::WorldRead;
use crate::needs::{js_string, js_truthy, undefined};
use crate::operation_session::OperationSession;
use crate::operation_session::{OpResult, OperationError};
use kingmaker_core::{
    market::{MarketOrder, SaleOffer},
    production::ProductionMethod,
    StockpileRecord,
};
use kingmaker_core::{ExternalId, ProvinceId};
use std::collections::{HashMap, HashSet};
type InputKey = (ExternalId, ProvinceId, ExternalId);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MarketLocation {
    Province(ProvinceId),
    Region(ExternalId),
}
#[derive(Debug, Clone)]
pub struct MarketScope {
    regions: HashMap<ProvinceId, Option<ExternalId>>,
}
impl MarketScope {
    pub fn from_world(w: &dyn WorldRead) -> OpResult<Self> {
        let mut regions = HashMap::new();
        if w.market_parameters()
            .as_ref()
            .and_then(|p| p.regional_market)
            == Some(true)
        {
            for row in w.territory().values() {
                if let Some(old) = regions.insert(row.province_id, row.region_id.clone()) {
                    if old != row.region_id {
                        return Err(OperationError("Conflicting province market region".into()));
                    }
                }
            }
        }
        Ok(Self { regions })
    }
    pub fn key(&self, id: ProvinceId) -> MarketLocation {
        self.regions
            .get(&id)
            .and_then(Option::as_ref)
            .map(|r| MarketLocation::Region(r.clone()))
            .unwrap_or(MarketLocation::Province(id))
    }
    pub fn same(&self, a: ProvinceId, b: ProvinceId) -> bool {
        self.key(a) == self.key(b)
    }
}

fn error(s: &str) -> OperationError {
    OperationError(s.into())
}
fn method<'a>(w: &'a dyn WorldRead, id: &ExternalId) -> OpResult<&'a ProductionMethod> {
    w.production_catalog()
        .as_ref()
        .and_then(|c| c.methods.iter().find(|m| &m.method_id == id))
        .ok_or_else(|| error("Unknown market producer method"))
}
fn protected_from_stocks(
    w: &dyn WorldRead,
    stocks: &[&StockpileRecord],
) -> OpResult<HashMap<ExternalId, f64>> {
    let mut required: HashMap<InputKey, f64> = HashMap::new();
    let mut held = HashMap::new();
    let mut result = HashMap::new();
    for r in w.goods_reservations().values() {
        if r.status == "held" {
            *held.entry(r.stock_id.clone()).or_insert(0.0) += r.quantity;
        }
    }
    for e in w.artisans().values() {
        if e.working_capital.is_none() {
            continue;
        }
        let pending = e
            .pending_retooling
            .as_ref()
            .and_then(|p| p.get("method_id"))
            .filter(|v| js_truthy(v))
            .map(|v| ExternalId::from_json(v.clone()).map_err(OperationError))
            .transpose()?;
        let id = pending.as_ref().unwrap_or(&e.base.method_id);
        let Some(m) = w.production_catalog()
            .as_ref()
            .and_then(|c| c.methods.iter().find(|m| &m.method_id == id))
            .filter(|m| m.status == "ready")
        else {
            continue;
        };
        let Some(owner) = &e.base.owner_id else {
            continue;
        };
        let productivity = m
            .weekly_productivity
            .ok_or_else(|| error("Unknown ready artisan productivity"))?;
        let fte = e
            .base
            .assignments
            .iter()
            .map(|a| a.workers_count * a.time_fraction)
            .sum::<f64>();
        if !fte.is_finite() || fte < 0.0 || !productivity.is_finite() || productivity <= 0.0 {
            return Err(error("Invalid protected artisan input demand"));
        }
        for i in &m.inputs {
            let q = fte * productivity * i.amount;
            if !q.is_finite() || q < 0.0 {
                return Err(error("Nonfinite protected input demand"));
            }
            *required
                .entry((owner.clone(), e.base.province_id, i.goods_id.clone()))
                .or_default() += q;
        }
    }
    for s in stocks {
        if s.purpose == "food_reserve" {
            continue;
        }
        let k = (s.owner_id.clone(), s.province_id, s.goods.external_id());
        let wanted = required.get(&k).copied().unwrap_or(0.0);
        let q =
            wanted.min((s.quantity - held.get(&s.external_id).copied().unwrap_or(0.0)).max(0.0));
        if q > 0.0 {
            result.insert(s.external_id.clone(), q);
            required.insert(k, wanted - q);
        }
    }
    Ok(result)
}
pub fn protected_artisan_inputs(w: &dyn WorldRead) -> OpResult<HashMap<ExternalId, f64>> {
    protected_from_stocks(w, &w.stockpiles().values().collect::<Vec<_>>())
}

#[derive(Debug, Clone)]
pub struct MarketPolicy {
    pub scope: MarketScope,
    pub held: HashMap<ExternalId, f64>,
    private_inputs: HashSet<InputKey>,
    seller_purposes: HashMap<ExternalId, String>,
    pop_owners: HashSet<ExternalId>,
    offers: HashMap<ExternalId, SaleOffer>,
    account_purposes: HashMap<ExternalId, String>,
    conversion_sources: HashMap<ExternalId, StockpileRecord>,
    pub(crate) procurement_sources: Option<HashMap<ExternalId, ExternalId>>,
}
impl MarketPolicy {
    pub fn from_session(s: &OperationSession) -> OpResult<Self> {
        let w = s.base;
        let mut held: HashMap<ExternalId, f64> = HashMap::new();
        for r in w.goods_reservations().values() {
            if r.status == "held" {
                *held.entry(r.stock_id.clone()).or_default() += r.quantity;
            }
        }
        let stocks = s.stocks().collect::<Vec<_>>();
        for (id, q) in protected_from_stocks(w, &stocks)? {
            *held.entry(id).or_default() += q;
        }
        let mut private_inputs = HashSet::new();
        let mut seller_purposes = HashMap::new();
        for e in w.factories().values()
            .map(|f| &f.base)
            .chain(w.artisans().values().map(|a| &a.base))
        {
            let m = method(w, &e.method_id)?;
            let Some(owner) = &e.owner_id else { continue };
            for i in &m.inputs {
                private_inputs.insert((owner.clone(), e.province_id, i.goods_id.clone()));
            }
            seller_purposes.insert(
                owner.clone(),
                e.payment_purpose.clone().unwrap_or_else(|| {
                    if m.kind == "artisan" {
                        "operating_capital".into()
                    } else {
                        "employer_operating_capital".into()
                    }
                }),
            );
        }
        for p in w.construction_projects().values() {
            if matches!(p.status.as_str(), "completed" | "cancelled") {
                continue;
            }
            if let Some(materials) = p
                .extra
                .get("construction")
                .and_then(|c| c.get("materials"))
                .and_then(|v| v.as_array())
            {
                for m in materials {
                    if let (Some(owner), Some(province), Some(good)) = (
                        p.extra.get("owner_id"),
                        p.extra.get("province_id"),
                        m.get("goods_id"),
                    ) {
                        private_inputs.insert((
                            ExternalId::from_json(owner.clone()).map_err(OperationError)?,
                            ProvinceId::from_external(
                                &ExternalId::from_json(province.clone()).map_err(OperationError)?,
                            )
                            .map_err(OperationError)?,
                            ExternalId::from_json(good.clone()).map_err(OperationError)?,
                        ));
                    }
                }
            }
        }
        let mut offers = HashMap::new();
        for o in w.sale_offers().values() {
            let status = o.extra.get("status").and_then(|v| v.as_str());
            let source = o.extra.get("source").is_some_and(js_truthy);
            let approval = o.extra.get("approval_ref").is_some_and(js_truthy);
            if matches!(status, Some("approved" | "source"))
                && source
                && (status != Some("approved") || approval)
            {
                offers.insert(o.stock_id.clone(), o.clone());
            }
        }
        let mut account_purposes = HashMap::new();
        for a in s.accounts() {
            if matches!(
                a.purpose.as_str(),
                "operating_capital" | "employer_operating_capital"
            ) {
                account_purposes
                    .entry(a.owner_id.clone())
                    .or_insert_with(|| a.purpose.clone());
            }
        }
        Ok(Self {
            scope: MarketScope::from_world(w)?,
            held,
            private_inputs,
            seller_purposes,
            pop_owners: w.pops().values().map(|p| p.external_id.clone()).collect(),
            offers,
            account_purposes,
            conversion_sources: HashMap::new(),
            procurement_sources: s.procurement_sources.clone(),
        })
    }
    pub fn free(&self, s: &StockpileRecord) -> f64 {
        (s.quantity - self.held.get(&s.external_id).copied().unwrap_or(0.0)).max(0.0)
    }
    fn approved_offer(&self, s: &StockpileRecord) -> Option<&SaleOffer> {
        self.offers.get(&s.external_id).filter(|o| {
            o.seller_id == s.owner_id
                && o.goods_id == s.goods.external_id()
                && o.seller_purpose.as_ref().is_some_and(|p| !p.is_empty())
        })
    }
    pub fn can_sell(&self, s: &StockpileRecord, o: &MarketOrder) -> bool {
        if self.procurement_sources.as_ref().is_some_and(|allowed| {
            allowed.get(&s.external_id) != Some(&o.owner_id) || self.approved_offer(s).is_none()
        }) {
            return false;
        }
        let title = self.conversion_sources.get(&s.external_id).unwrap_or(s);
        if title.purpose == "food_reserve" {
            return o.consume
                && title.owner_id == o.owner_id
                && (title.beneficiary_pop_id.is_none() || title.beneficiary_pop_id == o.pop_id);
        }
        s.owner_id == o.owner_id
            || self.approved_offer(s).is_some()
            || (!self.pop_owners.contains(&s.owner_id)
                && !self.private_inputs.contains(&(
                    s.owner_id.clone(),
                    s.province_id,
                    s.goods.external_id(),
                )))
    }
    pub(crate) fn derive_food_offer(&mut self, source: &StockpileRecord, output: &StockpileRecord) {
        if let Some(mut offer) = self.approved_offer(source).cloned() {
            offer.stock_id = output.external_id.clone();
            offer.goods_id = output.goods.external_id();
            self.offers.insert(output.external_id.clone(), offer);
        }
        self.conversion_sources
            .insert(output.external_id.clone(), source.clone());
    }
    pub fn seller_purpose(&self, s: &StockpileRecord) -> String {
        self.approved_offer(s)
            .and_then(|o| o.seller_purpose.clone())
            .or_else(|| {
                self.pop_owners
                    .contains(&s.owner_id)
                    .then(|| "personal_savings".into())
            })
            .or_else(|| self.seller_purposes.get(&s.owner_id).cloned())
            .or_else(|| self.account_purposes.get(&s.owner_id).cloned())
            .unwrap_or_else(|| "employer_operating_capital".into())
    }
    pub fn quote(w: &dyn WorldRead, o: &MarketOrder) -> Option<f64> {
        let currency = o.currency_id.as_ref()?;
        let p = w.price_calibration()
            .as_ref()?
            .prices_by_currency
            .get(&js_string(&currency.json()))?
            .get(&js_string(&o.goods_id.json()))?;
        if undefined(p) {
            return None;
        }
        p.as_f64().filter(|p| p.is_finite() && *p > 0.0)
    }
}
