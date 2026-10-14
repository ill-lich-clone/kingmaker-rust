//! Private first-week incremental production reservation. Never issues stock or payments.
use kingmaker_core::world_read::WorldRead;
use crate::{
    budget_funding::error,
    labor_money::LaborCash,
    labor_release::apply_staff_change,
    money_math::multiply_money,
    operation_session::{external_text, OpResult, OperationSession},
    production_activities::{factory_activity, rgo_capacity},
    storage::{approved, truthy, StorageBatch},
};
use kingmaker_core::{production::*, *};
use std::collections::HashMap;
type StockKey = (ExternalId, ProvinceId, ExternalId);
#[derive(Clone)]
enum Asset {
    Rgo(ResourceOperation),
    Factory(FactoryEnterprise),
    Artisan(ArtisanEnterprise),
}
impl Asset {
    fn base(&self) -> &Producer {
        match self {
            Self::Rgo(e) => &e.base,
            Self::Factory(e) => &e.base,
            Self::Artisan(e) => &e.base,
        }
    }
    fn base_mut(&mut self) -> &mut Producer {
        match self {
            Self::Rgo(e) => &mut e.base,
            Self::Factory(e) => &mut e.base,
            Self::Artisan(e) => &mut e.base,
        }
    }
}
enum Undo {
    Pop(ExternalId, Option<PopRecord>),
    Asset(ExternalId, Option<Asset>),
    Output(ExternalId, Option<f64>),
    Stock(StockKey, Option<f64>),
    Space(ExternalId, Option<f64>),
    Cash(AccountAddress, Option<Money>),
}
#[derive(Clone)]
pub struct HiringDemands {
    pub inputs: Vec<(StockKey, f64)>,
    pub space: f64,
    pub facility: ExternalId,
    pub rent: Option<(AccountAddress, Money)>,
}
struct Trial {
    asset: Asset,
    output: f64,
    demands: HiringDemands,
}
pub struct HiringForecast<'a> {
    world: &'a dyn WorldRead,
    methods: HashMap<ExternalId, &'a ProductionMethod>,
    assets: HashMap<ExternalId, Asset>,
    pops: HashMap<ExternalId, PopRecord>,
    outputs: HashMap<ExternalId, f64>,
    stock: HashMap<StockKey, f64>,
    spaces: HashMap<ExternalId, f64>,
    storage: StorageBatch<'a>,
    pub cash: LaborCash,
    undo: Option<Vec<Undo>>,
}
impl<'a> HiringForecast<'a> {
    pub fn new(w: &'a dyn WorldRead, cash: LaborCash) -> OpResult<Self> {
        let s = OperationSession::new(w)?;
        let storage = StorageBatch::new(&s)?;
        let methods = w.production_catalog()
            .as_ref()
            .ok_or_else(|| error("Hiring production catalog unresolved"))?
            .methods
            .iter()
            .map(|m| (m.method_id.clone(), m))
            .collect();
        let mut held = HashMap::new();
        for r in w.goods_reservations().values() {
            if r.status == "held" {
                *held.entry(&r.stock_id).or_insert(0.) += r.quantity;
            }
        }
        let mut stock = HashMap::new();
        for s in w.stockpiles().values() {
            *stock
                .entry((s.owner_id.clone(), s.province_id, s.goods.external_id()))
                .or_insert(0.) +=
                (s.quantity - held.get(&s.external_id).copied().unwrap_or(0.)).max(0.);
        }
        let mut f = Self {
            world: w,
            methods,
            assets: HashMap::new(),
            pops: w.pops().values()
                .map(|p| (p.external_id.clone(), p.clone()))
                .collect(),
            outputs: HashMap::new(),
            stock,
            spaces: HashMap::new(),
            storage,
            cash,
            undo: None,
        };
        let rows = w.resource_operations().values()
            .cloned()
            .map(Asset::Rgo)
            .chain(
                w.factories().values()
                    .cloned()
                    .map(Asset::Factory),
            )
            .chain(
                w.artisans().values()
                    .cloned()
                    .map(Asset::Artisan),
            );
        for a in rows {
            let id = a.base().id.clone();
            let output = f.potential(&a)?;
            if f.assets.insert(id.clone(), a.clone()).is_some() {
                return Err(error("Duplicate hiring producer"));
            }
            f.outputs.insert(id, output);
            if output > 0. {
                if let Some(d) = f.demands(&a, output)? {
                    f.reserve(&d)?;
                }
            }
        }
        Ok(f)
    }
    fn potential(&self, a: &Asset) -> OpResult<f64> {
        let Some(m) = self.methods.get(&a.base().method_id) else {
            return Ok(0.);
        };
        if m.status != "ready" {
            return Ok(0.);
        }
        let productivity = m
            .weekly_productivity
            .filter(|n| n.is_finite() && *n > 0.)
            .ok_or_else(|| error("Invalid hiring productivity"))?;
        match a {
            Asset::Factory(f) => {
                if !matches!(f.status.as_deref(), Some("operating" | "awaiting_staff")) {
                    return Ok(0.);
                }
                let mut f = f.clone();
                f.status = Some("operating".into());
                let pops = f
                    .base
                    .assignments
                    .iter()
                    .filter_map(|a| self.pops.get_key_value(&a.pop_id))
                    .collect();
                Ok(factory_activity(self.world, &f, &pops, "hire-forecast")?.output)
            }
            Asset::Artisan(e)
                if e.pending_retooling.as_ref().is_some_and(truthy)
                    || m.technology
                        .iter()
                        .any(|t| !e.implemented_technologies.contains(t)) =>
            {
                Ok(0.)
            }
            _ => Ok(a
                .base()
                .assignments
                .iter()
                .map(|x| {
                    x.workers_count
                        * x.time_fraction
                        * if m.kind == "rgo" {
                            x.productivity_multiplier.unwrap_or(1.)
                        } else {
                            1.
                        }
                })
                .sum::<f64>()
                * productivity),
        }
    }
    fn demands(&mut self, a: &Asset, amount: f64) -> OpResult<Option<HiringDemands>> {
        let base = a.base();
        let Some(m) = self.methods.get(&base.method_id) else {
            return Ok(None);
        };
        let Some(id) = &base.facility_id else {
            return Ok(None);
        };
        let available = self.storage.availability(id, &m.output_goods_id);
        if available.status == "unknown" {
            return Ok(None);
        }
        let Some(f) = self.storage.facilities.get(id) else {
            return Ok(None);
        };
        let mapping = f
            .goods
            .get(&external_text(&m.output_goods_id))
            .ok_or_else(|| error("Hiring output storage mapping unresolved"))?;
        if available.unit.as_ref() != Some(&m.output_unit) {
            return Ok(None);
        }
        let space = amount * mapping.space_per_unit;
        let owner = base
            .owner_id
            .as_ref()
            .ok_or_else(|| error("Hiring producer owner unresolved"))?;
        let rent = if &f.owner_id != owner {
            let Some(lease) = f.access_contracts.iter().find(|l| {
                l.owner_id.as_ref() == Some(owner)
                    && approved(
                        l.status.as_deref(),
                        l.source.as_ref(),
                        l.approval_ref.as_ref(),
                    )
            }) else {
                return Ok(None);
            };
            let (Some(price), Some(currency), Some(purpose)) = (
                lease.price_minor_per_space,
                lease.currency_id.as_ref(),
                lease.purpose.as_ref(),
            ) else {
                return Ok(None);
            };
            Some((
                AccountAddress {
                    owner_id: owner.clone(),
                    currency_id: currency.clone(),
                    purpose: purpose.clone(),
                },
                multiply_money(price, space)?,
            ))
        } else {
            None
        };
        let inputs = m
            .inputs
            .iter()
            .map(|i| {
                (
                    (owner.clone(), base.province_id, i.goods_id.clone()),
                    amount * i.amount,
                )
            })
            .collect();
        if !self.spaces.contains_key(id) {
            self.set_space(id.clone(), available.free_space.unwrap());
        }
        Ok(Some(HiringDemands {
            inputs,
            space,
            facility: id.clone(),
            rent,
        }))
    }
    fn reserve(&mut self, d: &HiringDemands) -> OpResult<()> {
        for (k, n) in &d.inputs {
            let v = self.stock.get(k).copied().unwrap_or(0.) - n;
            self.set_stock(k.clone(), v);
        }
        self.set_space(
            d.facility.clone(),
            self.spaces.get(&d.facility).copied().unwrap_or(0.) - d.space,
        );
        if let Some((k, n)) = &d.rent {
            let v = self
                .cash
                .balances
                .get(k)
                .copied()
                .unwrap_or(Money::ZERO)
                .0
                .checked_sub(n.0)
                .ok_or_else(|| error("Hiring rent overflow"))?;
            self.set_cash(k.clone(), Money(v));
        }
        Ok(())
    }
    fn trial(&mut self, id: &ExternalId, p: &PopRecord, count: u64) -> OpResult<Option<Trial>> {
        if count == 0 {
            return Ok(None);
        }
        self.set_pop(p.external_id.clone(), p.clone());
        let a = self
            .assets
            .get(id)
            .ok_or_else(|| error("Unknown hiring producer"))?
            .clone();
        let Some(m) = self.methods.get(&a.base().method_id) else {
            return Ok(None);
        };
        let cap = match &a {
            Asset::Rgo(r) => rgo_capacity(r, &self.world.expansions())?,
            Asset::Factory(f) => f.people_slots.unwrap_or(0.),
            Asset::Artisan(e) => e.people_slots.unwrap_or(0.),
        };
        if a.base()
            .assignments
            .iter()
            .map(|a| a.workers_count)
            .sum::<f64>()
            + count as f64
            > cap
        {
            return Ok(None);
        }
        if let Asset::Factory(f) = &a {
            if p.pop_type_id.as_str() != Some("craftsmen") {
                let mut direct = 0.;
                let mut support = 0.;
                for a in &f.base.assignments {
                    if self
                        .pops
                        .get(&a.pop_id)
                        .is_some_and(|p| p.pop_type_id.as_str() == Some("craftsmen"))
                    {
                        direct += a.workers_count * a.time_fraction;
                    } else {
                        support += a.workers_count * a.time_fraction;
                    }
                }
                let ratio = f
                    .parameters
                    .as_ref()
                    .and_then(|p| p.clerk_ratio)
                    .unwrap_or(0.);
                if direct <= 0. || ratio <= 0. || support + count as f64 > direct * ratio + 1e-9 {
                    return Ok(None);
                }
            }
        }
        let with_workers = |n: u64| -> OpResult<Asset> {
            let mut next = a.clone();
            if let Asset::Factory(f) = &mut next {
                *f = apply_staff_change(f, 0, n)?;
            }
            next.base_mut().assignments.push(LaborAssignment {
                pop_id: p.external_id.clone(),
                workers_count: n as f64,
                time_fraction: 1.,
                fte: None,
                employment_id: Some(format!("forecast:{}", external_text(&p.external_id)).into()),
                contract_id: None,
                worker_group_id: None,
                productivity_multiplier: Some(
                    p.extra
                        .get("species_labor_coefficient")
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(1.),
                ),
                productivity_status: None,
                extra: Default::default(),
            });
            Ok(next)
        };
        let next = with_workers(count)?;
        let output = match self.potential(&next) {
            Ok(n) => n,
            Err(_) => return Ok(None),
        };
        let increment = output - self.outputs.get(id).copied().unwrap_or(0.);
        if !(increment > 0.)
            || count > 1 && !(output > self.potential(&with_workers(count - 1)?)?)
        {
            return Ok(None);
        }
        if m.kind
            != match a {
                Asset::Rgo(_) => "rgo",
                Asset::Factory(_) => "factory",
                Asset::Artisan(_) => "artisan",
            }
        {
            return Err(error("Hiring method kind mismatch"));
        }
        let Some(d) = self.demands(&a, increment)? else {
            return Ok(None);
        };
        if d.inputs
            .iter()
            .any(|(k, n)| *n > self.stock.get(k).copied().unwrap_or(0.) + 1e-9)
            || d.space > self.spaces.get(&d.facility).copied().unwrap_or(0.) + 1e-9
            || d.rent.as_ref().is_some_and(|(k, n)| {
                n.0 > self.cash.balances.get(k).copied().unwrap_or(Money::ZERO).0
            })
        {
            return Ok(None);
        }
        Ok(Some(Trial {
            asset: next,
            output,
            demands: d,
        }))
    }
    pub fn limit(
        &mut self,
        id: &ExternalId,
        p: &PopRecord,
        requested: u64,
        mut affordable: impl FnMut(&HiringDemands, u64) -> OpResult<bool>,
    ) -> OpResult<u64> {
        let (mut lo, mut hi) = (0, requested);
        while lo < hi {
            let mid = lo + (hi - lo + 1) / 2;
            let ok = match self.trial(id, p, mid)? {
                Some(t) => affordable(&t.demands, mid)?,
                None => false,
            };
            if ok {
                lo = mid
            } else {
                hi = mid - 1
            }
        }
        Ok(lo)
    }
    pub fn accept(&mut self, id: &ExternalId, p: &PopRecord, count: u64) -> OpResult<()> {
        let t = self
            .trial(id, p, count)?
            .ok_or_else(|| error("Unfunded production hiring forecast"))?;
        self.reserve(&t.demands)?;
        self.set_asset(id.clone(), t.asset);
        self.set_output(id.clone(), t.output);
        Ok(())
    }
    pub fn preview<T>(&mut self, action: impl FnOnce(&mut Self) -> OpResult<T>) -> OpResult<T> {
        if self.undo.is_some() {
            return Err(error("Nested production hiring preview"));
        }
        self.undo = Some(vec![]);
        let result = action(self);
        for change in self.undo.take().unwrap().into_iter().rev() {
            match change {
                Undo::Pop(k, v) => restore(&mut self.pops, k, v),
                Undo::Asset(k, v) => restore(&mut self.assets, k, v),
                Undo::Output(k, v) => restore(&mut self.outputs, k, v),
                Undo::Stock(k, v) => restore(&mut self.stock, k, v),
                Undo::Space(k, v) => restore(&mut self.spaces, k, v),
                Undo::Cash(k, v) => restore(&mut self.cash.balances, k, v),
            }
        }
        result
    }
    pub fn reserve_wages(
        &mut self,
        pool: &AccountAddress,
        source: &AccountAddress,
        expense: Money,
    ) -> OpResult<bool> {
        if self.undo.is_none() {
            return Err(error("Wage quote requires preview"));
        }
        let local = self
            .cash
            .balances
            .get(pool)
            .copied()
            .unwrap_or(Money::ZERO)
            .0;
        let general = if source != pool {
            self.cash
                .balances
                .get(source)
                .copied()
                .unwrap_or(Money::ZERO)
                .0
        } else {
            0
        };
        if expense.0 < 0 || expense.0 as i128 > local as i128 + general as i128 {
            return Ok(false);
        }
        let used = expense.0.min(local);
        self.set_cash(pool.clone(), Money(local - used));
        if source != pool {
            self.set_cash(source.clone(), Money(general - (expense.0 - used)));
        }
        Ok(true)
    }
    fn set_pop(&mut self, k: ExternalId, v: PopRecord) {
        let old = self.pops.insert(k.clone(), v);
        if let Some(u) = &mut self.undo {
            u.push(Undo::Pop(k, old));
        }
    }
    fn set_asset(&mut self, k: ExternalId, v: Asset) {
        let old = self.assets.insert(k.clone(), v);
        if let Some(u) = &mut self.undo {
            u.push(Undo::Asset(k, old));
        }
    }
    fn set_output(&mut self, k: ExternalId, v: f64) {
        let old = self.outputs.insert(k.clone(), v);
        if let Some(u) = &mut self.undo {
            u.push(Undo::Output(k, old));
        }
    }
    fn set_stock(&mut self, k: StockKey, v: f64) {
        let old = self.stock.insert(k.clone(), v);
        if let Some(u) = &mut self.undo {
            u.push(Undo::Stock(k, old));
        }
    }
    fn set_space(&mut self, k: ExternalId, v: f64) {
        let old = self.spaces.insert(k.clone(), v);
        if let Some(u) = &mut self.undo {
            u.push(Undo::Space(k, old));
        }
    }
    pub(crate) fn set_cash(&mut self, k: AccountAddress, v: Money) {
        let old = self.cash.balances.insert(k.clone(), v);
        if let Some(u) = &mut self.undo {
            u.push(Undo::Cash(k, old));
        }
    }
}
fn restore<K: std::hash::Hash + Eq, V>(map: &mut HashMap<K, V>, k: K, v: Option<V>) {
    if let Some(v) = v {
        map.insert(k, v);
    } else {
        map.remove(&k);
    }
}
