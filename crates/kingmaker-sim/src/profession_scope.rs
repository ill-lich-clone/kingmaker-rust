//! One phase index and small employer dependency closures; never a full-world preview per child.
use kingmaker_core::row_store::ReadSequence;
use kingmaker_core::world_read::WorldRead;
use crate::{
    budget_funding::{error, optional_id, payroll_purpose, wage_multiplier},
    labor_offers::JobOffer,
    labor_roster::{build_labor_roster, LaborRoster},
    local_hiring::HiringScope,
    money_math::rate_pay,
    operation_session::*,
};
use kingmaker_core::{production::*, *};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
type Asset = (u8, usize);
type IdRows = HashMap<ExternalId, Vec<usize>>;
pub(crate) struct FinanceIndex {
    accounts: HashMap<AccountAddress, usize>,
    stocks: HashMap<ExternalId, usize>,
    claims: HashMap<ExternalId, usize>,
    next: u32,
}
impl FinanceIndex {
    pub fn new(w: &dyn WorldRead) -> Self {
        Self {
            accounts: w.accounts().values()
                .enumerate()
                .map(|(i, a)| (a.address(), i))
                .collect(),
            stocks: index(&w.stockpiles(), |s| s.external_id.clone()),
            claims: index(&w.obligations(), |d| d.obligation_id.clone()),
            next: w.accounts().values()
                .map(|a| a.id.0)
                .chain(w.stockpiles().values().map(|s| s.id))
                .chain(w.pops().values().map(|p| p.id.0))
                .chain(w.enterprises().values().map(|e| e.id.0))
                .max()
                .unwrap_or(0),
        }
    }
    pub fn allocate(&mut self) -> OpResult<u32> {
        self.next = self
            .next
            .checked_add(1)
            .ok_or_else(|| error("Private profession ordinal overflow"))?;
        Ok(self.next)
    }
    pub fn commit(&mut self,w:&mut crate::world_transaction::WorldTransaction<'_>,selected:&dyn WorldRead)->OpResult<()> {
        for a in selected.accounts().iter() {
            let row=if let Some(old)=w.accounts().get(&a.address()) {let mut row=old.clone();row.balance=a.balance;row} else {
                let mut row=a.clone();row.id=AccountId(self.allocate()?);row.external_id=serde_json::to_string(&serde_json::json!([a.owner_id,a.currency_id,a.purpose]))?.into();
                self.accounts.insert(row.address(),w.accounts().len());row
            };w.put_accounts(row.address(),row)?;
        }
        for row in selected.stockpiles().iter() {
            let mut row=row.clone();
            if let Some(old)=w.stockpiles().get(&row.external_id) {row.id=old.id;} else {row.id=self.allocate()?;self.stocks.insert(row.external_id.clone(),w.stockpiles().len());}
            w.put_stockpiles(row.external_id.clone(),row)?;
        }
        for row in selected.obligations().iter() {
            if !self.claims.contains_key(&row.obligation_id) {self.claims.insert(row.obligation_id.clone(),w.obligations().len());}
            w.put_obligations(row.obligation_id.clone(),row.clone())?;
        }
        Ok(())
    }

}
fn add(map: &mut IdRows, id: ExternalId, i: usize) {
    map.entry(id).or_default().push(i)
}
fn index<T>(rows: &(impl ReadSequence<T> + ?Sized), id: impl Fn(&T) -> ExternalId) -> HashMap<ExternalId, usize> {
    rows.iter()
        .enumerate()
        .map(|(i, row)| (id(row), i))
        .collect()
}
fn base(w: &dyn WorldRead, key: Asset) -> &Producer {
    match key.0 {
        0 => &w.resource_operations().at(key.1).expect("validated row position").base,
        1 => &w.factories().at(key.1).expect("validated row position").base,
        _ => &w.artisans().at(key.1).expect("validated row position").base,
    }
}
fn extend<T: Clone>(target: &mut Vec<T>, source: &(impl ReadSequence<T> + ?Sized), rows: &IdRows, id: &ExternalId) {
    target.extend(
        rows.get(id)
            .into_iter()
            .flatten()
            .map(|&i| source.get(i).expect("validated row position").clone()),
    )
}
fn merge<T>(target: &mut Vec<T>, rows: impl IntoIterator<Item = T>, id: impl Fn(&T) -> ExternalId) {
    let mut ids = index(target, &id);
    for row in rows {
        let key = id(&row);
        if let Some(&i) = ids.get(&key) {
            target[i] = row
        } else {
            ids.insert(key, target.len());
            target.push(row)
        }
    }
}
fn job_id(v: &Value) -> OpResult<ExternalId> {
    optional_id(v.get("job_id"))?
        .or(optional_id(v.get("id"))?)
        .ok_or_else(|| error("Profession scope job ID missing"))
}
pub(crate) struct ProfessionIndex {
    assets: HashMap<ExternalId, Asset>,
    facility_assets: HashMap<ExternalId, Vec<Asset>>,
    input_assets: HashMap<(ExternalId, ProvinceId), Vec<Asset>>,
    rent_assets: HashMap<ExternalId, Vec<Asset>>,
    pops: HashMap<ExternalId, usize>,
    contracts: HashMap<ExternalId, usize>,
    employment: HashMap<ExternalId, usize>,
    jobs: HashMap<ExternalId, usize>,
    facilities: HashMap<ExternalId, usize>,
    building_jobs: IdRows,
    building_contracts: IdRows,
    building_employment: IdRows,
    refs: IdRows,
    accounts: IdRows,
    stocks: IdRows,
    stock_facilities: IdRows,
    goods: IdRows,
    storage: IdRows,
    debts: IdRows,
    reservations: IdRows,
    expansions: IdRows,
    finance_lengths: (usize, usize, usize),
    wage_reserves: HashMap<ExternalId, HashMap<AccountAddress, i128>>,
}
impl ProfessionIndex {
    pub fn commit_hiring(
        &mut self,
        w: &mut crate::world_transaction::WorldTransaction<'_>,
        selected: &dyn WorldRead,
        child: &ExternalId,
        policies: &ProductionPolicies,
        hires: &(impl ReadSequence<Value> + ?Sized),
    ) -> OpResult<()> {
        self.pops.insert(
            child.clone(),
            w.pops()
                .iter()
                .position(|p| &p.external_id == child)
                .ok_or_else(|| error("Accepted profession child missing"))?,
        );
        if let Some(p) = selected.pops().values().find(|p| &p.external_id == child) {
            w.put_pops(p.external_id.clone(),p.clone())?;
        }
        for c in selected.contracts().values() {
            if !self.contracts.contains_key(&c.contract_id) {self.contracts.insert(c.contract_id.clone(),w.contracts().len());}
            w.put_contracts(c.contract_id.clone(),c.clone())?;
        }
        for j in selected.jobs().values() {
            let id = job_id(j)?;
            let fresh = !self.jobs.contains_key(&id);
            let position = *self.jobs.get(&id).unwrap_or(&w.jobs().len());
            let key=if fresh {w.jobs().next_key().map_err(OperationError)?} else {w.jobs().keyed().nth(position).ok_or_else(||error("Missing job position"))?.0};
            self.jobs.insert(id,position);w.put_jobs(key,j.clone())?;
            if fresh {
                if let Some(b) = optional_id(j.get("building_id"))? {
                    add(&mut self.building_jobs, b, position)
                }
            }
        }
        let ids: HashSet<_> = hires
            .iter()
            .filter_map(|h| h.get("employment_id"))
            .map(|v| ExternalId::from_json(v.clone()).map_err(OperationError))
            .collect::<OpResult<_>>()?;
        for e in selected.employment().values()
            .filter(|e| ids.contains(&e.employment_id))
        {
            let fresh = !self.employment.contains_key(&e.employment_id);
            let position = *self
                .employment
                .get(&e.employment_id)
                .unwrap_or(&w.employment().len());
            self.employment.insert(e.employment_id.clone(),position);w.put_employment(e.employment_id.clone(),e.clone())?;
            if fresh {
                if let Some(b) = optional_id(e.extra.get("building_id"))? {
                    add(&mut self.building_employment, b, position)
                }
            }
        }
        for r in selected.resource_operations().values() {
            if let Some(&(0, _i)) = self.assets.get(&r.base.id) {
                w.put_resource_operations(r.base.id.clone(),r.clone())?;
            }
        }
        for f in selected.factories().values() {
            if let Some(&(1, _i)) = self.assets.get(&f.base.id) {
                w.put_factories(f.base.id.clone(),f.clone())?;
            }
        }
        let roster = build_labor_roster(selected)?;
        let mut session = OperationSession::new(selected)?;
        session.policies.wages.extend(policies.wages.clone());
        session.policies.extra.extend(policies.extra.clone());
        for id in ids {
            if let Some(e) = roster.by_employment.get(&id) {
                self.reserve(w, e, 1, &session)?
            }
        }
        Ok(())
    }
    pub fn new(
        w: &dyn WorldRead,
        policies: &ProductionPolicies,
        roster: &LaborRoster,
    ) -> OpResult<Self> {
        let facilities = index(&w.storage_facilities(), |f| f.id.clone());
        let mut out = Self {
            assets: HashMap::new(),
            facility_assets: HashMap::new(),
            input_assets: HashMap::new(),
            rent_assets: HashMap::new(),
            pops: index(&w.pops(), |p| p.external_id.clone()),
            contracts: index(&w.contracts(), |c| c.contract_id.clone()),
            employment: index(&w.employment(), |e| e.employment_id.clone()),
            jobs: HashMap::new(),
            facilities,
            building_jobs: IdRows::new(),
            building_contracts: IdRows::new(),
            building_employment: IdRows::new(),
            refs: IdRows::new(),
            accounts: IdRows::new(),
            stocks: IdRows::new(),
            stock_facilities: IdRows::new(),
            goods: IdRows::new(),
            storage: IdRows::new(),
            debts: IdRows::new(),
            reservations: IdRows::new(),
            expansions: IdRows::new(),
            finance_lengths: (0, 0, 0),
            wage_reserves: HashMap::new(),
        };
        for key in (0..w.resource_operations().len())
            .map(|i| (0, i))
            .chain((0..w.factories().len()).map(|i| (1, i)))
            .chain((0..w.artisans().len()).map(|i| (2, i)))
        {
            let e = base(w, key);
            out.assets.insert(e.id.clone(), key);
            if let Some(owner) = &e.owner_id {
                out.input_assets
                    .entry((owner.clone(), e.province_id))
                    .or_default()
                    .push(key);
                if let Some(id) = &e.facility_id {
                    out.facility_assets.entry(id.clone()).or_default().push(key);
                    if out.facilities.get(id).is_some_and(|&i| {
                        w.storage_facilities().at(i).expect("validated row position").owner_id != *owner
                    }) {
                        out.rent_assets.entry(owner.clone()).or_default().push(key)
                    }
                }
            }
        }
        for (i, j) in w.jobs().values().enumerate() {
            out.jobs.insert(job_id(j)?, i);
            if let Some(b) = optional_id(j.get("building_id"))? {
                add(&mut out.building_jobs, b, i)
            }
        }
        for (i, c) in w.contracts().values().enumerate() {
            if let Some(b) = optional_id(c.extra.get("building_id"))? {
                add(&mut out.building_contracts, b, i)
            }
        }
        for (i, e) in w.employment().values().enumerate() {
            if let Some(b) = optional_id(e.extra.get("building_id"))? {
                add(&mut out.building_employment, b, i)
            }
        }
        for (i, r) in w.needs()
            .iter()
            .flat_map(|n| n.profile_refs.iter())
            .enumerate()
        {
            add(&mut out.refs, r.pop_id.clone(), i)
        }
        for (i, r) in w.storage_reservations().values().enumerate() {
            add(&mut out.storage, r.facility_id.clone(), i)
        }
        for (i, r) in w.goods_reservations().values().enumerate() {
            add(&mut out.goods, r.stock_id.clone(), i)
        }
        for (i, r) in w.money_reservations().values().enumerate() {
            add(&mut out.reservations, r.owner_id.clone(), i)
        }
        for (i, e) in w.expansions().values().enumerate() {
            add(&mut out.expansions, e.rgo_id.clone(), i)
        }
        out.sync_finance(w);
        let mut session = OperationSession::new(w)?;
        session.policies.wages.extend(policies.wages.clone());
        session.policies.extra.extend(policies.extra.clone());
        for e in roster.by_employment.values() {
            out.reserve(w, e, 1, &session)?
        }
        Ok(out)
    }
    fn reserve(
        &mut self,
        w: &dyn WorldRead,
        e: &crate::labor_roster::LaborEntry,
        sign: i128,
        s: &OperationSession,
    ) -> OpResult<()> {
        let Some(c) = e
            .contract_id
            .as_ref()
            .and_then(|id| self.contracts.get(id))
            .map(|&i| w.contracts().at(i).expect("validated row position"))
        else {
            return Ok(());
        };
        let Some(rate) = &c.weekly_wage_rate else {
            return Ok(());
        };
        let Some(owner) = &c.employer_entity_id else {
            return Err(error("Profession wage employer missing"));
        };
        let Some(currency) = &c.currency_id else {
            return Err(error("Profession wage currency missing"));
        };
        let address = AccountAddress {
            owner_id: owner.clone(),
            currency_id: currency.clone(),
            purpose: payroll_purpose(c)?,
        };
        *self
            .wage_reserves
            .entry(owner.clone())
            .or_default()
            .entry(address)
            .or_default() +=
            sign * 2 * i128::from(rate_pay(rate, e.fte, wage_multiplier(s, c)?)?.0);
        Ok(())
    }
    fn sync_finance(&mut self, w: &dyn WorldRead) {
        for (i, a) in w.accounts().values().enumerate().skip(self.finance_lengths.0) {
            add(&mut self.accounts, a.owner_id.clone(), i)
        }
        for (i, s) in w.stockpiles().values().enumerate().skip(self.finance_lengths.1) {
            add(&mut self.stocks, s.owner_id.clone(), i);
            if let Some(f) = &s.facility_id {
                add(&mut self.stock_facilities, f.clone(), i)
            }
        }
        for (i, d) in w.obligations().values()
            .enumerate()
            .skip(self.finance_lengths.2)
        {
            add(&mut self.debts, d.payer_id.clone(), i)
        }
        self.finance_lengths = (
            w.accounts().len(),
            w.stockpiles().len(),
            w.obligations().len(),
        );
    }
    pub fn select(
        &mut self,
        w: &dyn WorldRead,
        offer: &JobOffer,
        split: &dyn WorldRead,
        child: &ExternalId,
        policies: &ProductionPolicies,
    ) -> OpResult<(WorldState, HiringScope)> {
        self.sync_finance(w);
        let mut owners = HashSet::from([offer.owner_id.clone()]);
        if w.profiles().values()
            .any(|p| p.get("state_id") == Some(&offer.owner_id.json()))
        {
            owners.insert(format!("treasury:{}", external_text(&offer.owner_id)).into());
        }
        let mut selected = HashSet::<Asset>::new();
        let mut queue = vec![];
        if let Some(key) = self.assets.get(&offer.entity_id) {
            queue.push(*key)
        }
        for owner in &owners {
            queue.extend(self.rent_assets.get(owner).into_iter().flatten().copied())
        }
        let mut cursor = 0;
        while cursor < queue.len() {
            let key = queue[cursor];
            cursor += 1;
            if !selected.insert(key) {
                continue;
            }
            let e = base(w, key);
            if let Some(owner) = &e.owner_id {
                owners.insert(owner.clone());
                queue.extend(
                    self.input_assets
                        .get(&(owner.clone(), e.province_id))
                        .into_iter()
                        .flatten()
                        .copied(),
                );
                queue.extend(self.rent_assets.get(owner).into_iter().flatten().copied());
            }
            if let Some(f) = &e.facility_id {
                queue.extend(self.facility_assets.get(f).into_iter().flatten().copied())
            }
        }
        let mut out = WorldState::default();
        out.campaign = w.campaign().clone();
        out.production_data.catalog = w.production_catalog().clone();
        out.production_data.policies = w.production_policies().clone();
        out.labor_data.institution_rules = w.institution_rules().clone();
        out.labor_data.profiles = w.profiles().values().cloned().collect::<Vec<_>>();
        out.labor_data.species = w.species().values().cloned().collect::<Vec<_>>();
        out.market_data.price_calibration = w.price_calibration().clone();
        out.budget_data.foundation_budget_policy = w.foundation_budget_policy().clone();
        let mut buildings = HashSet::new();
        let mut facility_ids = HashSet::new();
        let mut pop_ids = HashSet::new();
        let mut contract_ids = HashSet::new();
        let mut job_ids = HashSet::new();
        let mut employment_ids = HashSet::new();
        if offer.kind == "service" {
            let b = offer
                .job
                .as_ref()
                .and_then(|j| j.get("building_id"))
                .map(|v| ExternalId::from_json(v.clone()))
                .transpose()
                .map_err(OperationError)?
                .unwrap_or_else(|| offer.entity_id.clone());
            buildings.insert(b.clone());
            out.market_data.institutions = w.institutions().values()
                .filter(|v| v.get("building_id").or(v.get("id")) == Some(&b.json()))
                .cloned()
                .collect();
            if let Some(j) = &offer.job {
                job_ids.insert(job_id(j)?);
            }
        }
        let mut keys: Vec<_> = selected.into_iter().collect();
        keys.sort();
        for key in keys {
            let e = base(w, key);
            buildings.insert(e.id.clone());
            if let Some(f) = &e.facility_id {
                facility_ids.insert(f.clone());
            }
            for a in &e.assignments {
                pop_ids.insert(a.pop_id.clone());
                if let Some(id) = &a.contract_id {
                    contract_ids.insert(id.clone());
                }
                if let Some(id) = &a.employment_id {
                    employment_ids.insert(id.clone());
                }
            }
            extend(
                &mut out.production_data.expansions,
                &w.expansions(),
                &self.expansions,
                &e.id,
            );
            match key.0 {
                0 => out
                    .production_data
                    .resource_operations
                    .push(w.resource_operations().at(key.1).expect("validated row position").clone()),
                1 => out
                    .production_data
                    .factories
                    .push(w.factories().at(key.1).expect("validated row position").clone()),
                _ => out
                    .production_data
                    .artisans
                    .push(w.artisans().at(key.1).expect("validated row position").clone()),
            }
        }
        for building in &buildings {
            for &i in self.building_jobs.get(building).into_iter().flatten() {
                job_ids.insert(job_id(w.jobs().at(i).expect("validated row position"))?);
            }
            for &i in self.building_contracts.get(building).into_iter().flatten() {
                contract_ids.insert(w.contracts().at(i).expect("validated row position").contract_id.clone());
            }
            if offer.kind == "service" {
                for &i in self.building_employment.get(building).into_iter().flatten() {
                    employment_ids.insert(w.employment().at(i).expect("validated row position").employment_id.clone());
                }
            }
        }
        for id in employment_ids {
            if let Some(&i) = self.employment.get(&id) {
                let e = w.employment().at(i).expect("validated row position");
                pop_ids.insert(e.pop_id.clone());
                contract_ids.insert(e.contract_id.clone());
                if let Some(id) = optional_id(e.extra.get("job_id"))? {
                    job_ids.insert(id);
                }
                out.budget_data.employment.push(e.clone());
            }
        }
        for id in contract_ids {
            if let Some(&i) = self.contracts.get(&id) {
                let c = w.contracts().at(i).expect("validated row position");
                if let Some(id) = optional_id(c.extra.get("job_id"))? {
                    job_ids.insert(id);
                }
                out.production_data.contracts.push(c.clone());
            }
        }
        for id in job_ids {
            if let Some(&i) = self.jobs.get(&id) {
                out.budget_data.jobs.push(w.jobs().at(i).expect("validated row position").clone())
            }
        }
        for id in pop_ids {
            if let Some(&i) = self.pops.get(&id) {
                out.pops.push(w.pops().at(i).expect("validated row position").clone())
            }
        }
        merge(&mut out.pops, split.pops().values().cloned().collect::<Vec<_>>(), |p| p.external_id.clone());
        for p in split.pops().values() {
            owners.insert(p.external_id.clone());
        }
        for owner in &owners {
            extend(&mut out.accounts, &w.accounts(), &self.accounts, owner);
            extend(&mut out.stockpiles, &w.stockpiles(), &self.stocks, owner);
            extend(
                &mut out.production_data.money_reservations,
                &w.money_reservations(),
                &self.reservations,
                owner,
            );
            extend(
                &mut out.production_data.obligations,
                &w.obligations(),
                &self.debts,
                owner,
            );
        }
        for id in facility_ids {
            if let Some(&i) = self.facilities.get(&id) {
                out.production_data
                    .storage_facilities
                    .push(w.storage_facilities().at(i).expect("validated row position").clone())
            }
            extend(
                &mut out.production_data.storage_reservations,
                &w.storage_reservations(),
                &self.storage,
                &id,
            );
            extend(
                &mut out.stockpiles,
                &w.stockpiles(),
                &self.stock_facilities,
                &id,
            );
        }
        // Shared facilities may have selected the same stock twice.
        let mut seen = HashSet::new();
        out.stockpiles
            .retain(|s| seen.insert(s.external_id.clone()));
        for s in &out.stockpiles {
            extend(
                &mut out.production_data.goods_reservations,
                &w.goods_reservations(),
                &self.goods,
                &s.external_id,
            )
        }
        merge(&mut out.accounts, split.accounts().values().cloned().collect::<Vec<_>>(), |a| {
            a.external_id.clone()
        });
        merge(&mut out.stockpiles, split.stockpiles().values().cloned().collect::<Vec<_>>(), |s| {
            s.external_id.clone()
        });
        merge(
            &mut out.production_data.obligations,
            split.obligations().values().cloned().collect::<Vec<_>>(),
            |d| d.obligation_id.clone(),
        );
        merge(
            &mut out.production_data.storage_facilities,
            split.storage_facilities().values().cloned().collect::<Vec<_>>(),
            |f| f.id.clone(),
        );
        merge(
            &mut out.production_data.artisans,
            split.artisans().values().cloned().collect::<Vec<_>>(),
            |a| a.base.id.clone(),
        );
        merge(
            &mut out.production_data.contracts,
            split.contracts().values().cloned().collect::<Vec<_>>(),
            |c| c.contract_id.clone(),
        );
        if let Some(needs) = &w.needs() {
            let refs = out
                .pops
                .iter()
                .flat_map(|p| {
                    self.refs
                        .get(&p.external_id)
                        .into_iter()
                        .flatten()
                        .map(|&i| needs.profile_refs[i].clone())
                })
                .collect();
            let mut n = kingmaker_core::market::NeedsData {
                schema_version: needs.schema_version.clone(),
                profile_refs: refs,
                profiles: needs.profiles.clone(),
                resolver: needs.resolver.clone(),
                extra: needs.extra.clone(),
            };
            if let Some(s) = &split.needs() {
                merge(&mut n.profile_refs, s.profile_refs.clone(), |r| {
                    r.pop_id.clone()
                });
            }
            out.market_data.needs = Some(n)
        }
        let provinces: HashSet<_> = out.pops.iter().map(|p| p.province_id).collect();
        out.provinces = w.provinces().values()
            .filter(|p| provinces.contains(&p.id))
            .cloned()
            .collect();
        out.market_data.territory = w.territory().values()
            .filter(|p| provinces.contains(&p.province_id))
            .cloned()
            .collect();
        out.rebuild_account_index().map_err(OperationError)?;
        let mut external: HashMap<AccountAddress, i128> = owners
            .iter()
            .flat_map(|id| {
                self.wage_reserves
                    .get(id)
                    .into_iter()
                    .flat_map(|rows| rows.iter())
                    .map(|(k, n)| (k.clone(), *n))
            })
            .collect();
        let selected_roster = build_labor_roster(&out)?;
        let mut session = OperationSession::new(&out)?;
        session.policies.wages.extend(policies.wages.clone());
        session.policies.extra.extend(policies.extra.clone());
        for e in selected_roster.by_employment.values() {
            if let Some(c) = e.contract_id.as_ref().and_then(|id| {
                out.production_data
                    .contracts
                    .iter()
                    .find(|c| &c.contract_id == id)
            }) {
                if let (Some(rate), Some(owner), Some(currency)) =
                    (&c.weekly_wage_rate, &c.employer_entity_id, &c.currency_id)
                {
                    let address = AccountAddress {
                        owner_id: owner.clone(),
                        currency_id: currency.clone(),
                        purpose: payroll_purpose(c)?,
                    };
                    *external.entry(address).or_default() -=
                        2 * i128::from(rate_pay(rate, e.fte, wage_multiplier(&session, c)?)?.0);
                }
            }
        }
        let reservations = external
            .into_iter()
            .filter(|(a, n)| owners.contains(&a.owner_id) && *n != 0)
            .map(|(a, n)| {
                if n < 0 {
                    return Err(error("Profession external wages became negative"));
                }
                Ok((
                    a,
                    Money(
                        i64::try_from(n)
                            .map_err(|_| error("Profession external wages overflow"))?,
                    ),
                ))
            })
            .collect::<OpResult<Vec<_>>>()?;
        Ok((
            out,
            HiringScope {
                candidate_ids: Some(HashSet::from([child.clone()])),
                offer_ids: Some(HashSet::from([offer.id.clone()])),
                external_wage_reservations: reservations,
            },
        ))
    }
}
