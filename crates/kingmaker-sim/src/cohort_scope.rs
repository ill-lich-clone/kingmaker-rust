//! Phase-local owner/reference indexes. Only a source POP's rows enter a scalar split preview.
use kingmaker_core::row_store::ReadSequence;
use kingmaker_core::world_read::WorldRead;
use crate::{
    budget_funding::error,
    cohort_assets::text,
    labor_roster::{build_labor_roster, LaborEntry},
    operation_session::OpResult,
};
use kingmaker_core::*;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
type OwnerRows = HashMap<ExternalId, Vec<usize>>;
fn add(map: &mut OwnerRows, id: &ExternalId, i: usize) {
    map.entry(id.clone()).or_default().push(i);
}
fn references(row: &Value) -> HashSet<ExternalId> {
    let mut ids = HashSet::new();
    for key in [
        "owner_id",
        "owner_entity_id",
        "employer_entity_id",
        "payer_id",
        "payee_id",
        "sponsor_id",
        "investor_id",
        "beneficiary_id",
        "beneficiary_pop_id",
        "pop_id",
        "buyer_id",
        "seller_id",
        "accepted_buyer_id",
        "accepted_seller_id",
        "accepted_carrier_id",
    ] {
        if let Some(v) = row.get(key) {
            if let Ok(id) = ExternalId::from_json(v.clone()) {
                ids.insert(id);
            }
        }
    }
    for key in ["beneficiaries", "access_contracts"] {
        if let Some(rows) = row.get(key).and_then(Value::as_array) {
            for row in rows.iter() {
                ids.extend(references(row));
            }
        }
    }
    ids
}
fn reference_index<T: Serialize>(rows: &(impl ReadSequence<T> + ?Sized)) -> OpResult<OwnerRows> {
    let mut out = OwnerRows::new();
    for (i, row) in rows.iter().enumerate() {
        for id in references(&serde_json::to_value(row)?) {
            add(&mut out, &id, i);
        }
    }
    Ok(out)
}
fn select<T: Clone>(rows: &(impl ReadSequence<T> + ?Sized), map: &OwnerRows, id: &ExternalId) -> Vec<T> {
    map.get(id)
        .into_iter()
        .flatten()
        .map(|&i| rows.get(i).expect("validated row position").clone())
        .collect()
}
pub struct CohortScope {
    pops: HashMap<ExternalId, usize>,
    accounts: OwnerRows,
    stocks: OwnerRows,
    claims: OwnerRows,
    money: OwnerRows,
    tables: HashMap<&'static str, OwnerRows>,
    goods: OwnerRows,
    refs: OwnerRows,
    financial_lengths: (usize, usize, usize),
    pub(crate) labor: HashMap<ExternalId, Vec<LaborEntry>>,
}
impl CohortScope {
    /// Install one accepted split into the phase's private world. Original positions stay stable.
    pub(crate) fn commit_registry(&self,w:&mut crate::world_transaction::WorldTransaction<'_>,parent:&ExternalId,split:&dyn WorldRead)->OpResult<()> {
        for p in split.pops().iter() {w.put_pops(p.external_id.clone(),p.clone())?;}
        macro_rules! rows {
            ($name:ident,$put:ident,$positions:expr) => {{
                let positions=$positions.get(parent).map(Vec::as_slice).unwrap_or(&[]);
                if positions.len()!=split.$name().len() {return Err(error("Cohort scoped row count changed"));}
                let changes=positions.iter().zip(split.$name().iter()).map(|(&i,row)| {
                    let key=w.$name().keyed().nth(i).ok_or_else(||error("Missing scoped row"))?.0;Ok((key,row.clone()))
                }).collect::<OpResult<Vec<_>>>()?;
                for (key,row) in changes {w.$put(key,row)?;}
            }};
        }
        rows!(storage_facilities,put_storage_facilities,self.tables["facilities"]);
        rows!(storage_reservations,put_storage_reservations,self.tables["storage"]);
        rows!(contracts,put_contracts,self.tables["contracts"]);
        rows!(jobs,put_jobs,self.tables["jobs"]);
        rows!(artisans,put_artisans,self.tables["artisans"]);
        rows!(sale_offers,put_sale_offers,self.tables["sales"]);
        rows!(maintenance_agreements,put_maintenance_agreements,self.tables["maintenance"]);
        rows!(shipments,put_shipments,self.tables["shipments"]);
        rows!(shipment_recovery_plans,put_shipment_recovery_plans,self.tables["recovery_plans"]);
        rows!(goods_reservations,put_goods_reservations,self.goods);
        if let (Some(dst), Some(src)) = (w.edit_needs(), &split.needs()) {
            let positions = self.refs.get(parent).map(Vec::as_slice).unwrap_or(&[]);
            if src.profile_refs.len() != positions.len() * 2 {
                return Err(error("Cohort profile split count changed"));
            }
            for (&i, pair) in positions.iter().zip(src.profile_refs.chunks_exact(2)) {
                dst.profile_refs[i] = pair[0].clone();
                dst.profile_refs.push(pair[1].clone());
            }
        }
        if let Some(src) = &split.pop_mobility() {
            let dst = w.edit_pop_mobility().get_or_insert_with(|| json!({}));
            if dst.get("unemployment").is_none() {
                dst["unemployment"] = json!({})
            }
            if let Some(rows) = src.get("unemployment").expect("validated row position").as_object() {
                for (k, v) in rows.iter() {
                    dst["unemployment"][k] = v.clone();
                }
            }
        }
        for row in split.mobility_cohort_receipts().iter() {
            let key=w.mobility_cohort_receipts().next_key().map_err(crate::operation_session::OperationError)?;
            w.put_mobility_cohort_receipts(key,row.clone())?;
        }
        Ok(())
    }
    /// Read-only preview for a new phase event. Assigned labor comes from the full-world index.
    pub fn preview(
        &mut self,
        w: &dyn WorldRead,
        r: &kingmaker_core::mobility::CohortSplitRequest,
    ) -> OpResult<(WorldState, kingmaker_core::mobility::CohortSplitReceipt)> {
        if w.committed_operations().values().any(|op| {
            op.event_id.as_str().is_some_and(|id| {
                id == r.event_id || id.starts_with(&format!("{}:debt:", r.event_id))
            })
        }) {
            return Err(error("Cohort preview event already exists"));
        }
        let mut scoped = self.select(w, &r.pop_id)?;
        let receipt = crate::cohort_cycle::execute_indexed_split(
            &mut scoped,
            r,
            self.labor.get(&r.pop_id).map(Vec::as_slice).unwrap_or(&[]),
        )?;
        if receipt
            .child_id
            .as_ref()
            .is_some_and(|id| self.pops.contains_key(id))
        {
            return Err(error("Cohort child exists outside scope"));
        }
        Ok((scoped, receipt))
    }
    pub fn new(w: &dyn WorldRead) -> OpResult<Self> {
        let mut accounts = OwnerRows::new();
        for (i, a) in w.accounts().values().enumerate() {
            add(&mut accounts, &a.owner_id, i)
        }
        let mut stocks = OwnerRows::new();
        let mut stockowners = HashMap::new();
        for (i, s) in w.stockpiles().values().enumerate() {
            add(&mut stocks, &s.owner_id, i);
            stockowners.insert(&s.external_id, &s.owner_id);
        }
        let mut goods = reference_index(&w.goods_reservations())?;
        for (i, g) in w.goods_reservations().values().enumerate() {
            if let Some(owner) = stockowners.get(&g.stock_id) {
                let rows = goods.entry((*owner).clone()).or_default();
                if !rows.contains(&i) {
                    rows.push(i)
                }
            }
        }
        let mut tables = HashMap::new();
        macro_rules! table {
            ($key:literal,$rows:expr) => {
                tables.insert($key, reference_index($rows)?);
            };
        }
        table!("facilities", &w.storage_facilities());
        table!("storage", &w.storage_reservations());
        table!("contracts", &w.contracts());
        table!("jobs", &w.jobs());
        table!("artisans", &w.artisans());
        table!("sales", &w.sale_offers());
        table!("maintenance", &w.maintenance_agreements());
        table!("shipments", &w.shipments());
        table!("recovery_plans", &w.shipment_recovery_plans());
        table!("capitalists", &w.capitalist_entities());
        table!("investments", &w.project_investments());
        table!(
            "production_rights",
            &w.production_beneficiaries()
        );
        table!("factory_rights", &w.factory_beneficiaries());
        let mut labor = HashMap::<_, Vec<_>>::new();
        for e in build_labor_roster(w)?.by_employment.into_values() {
            labor.entry(e.pop_id.clone()).or_default().push(e)
        }
        let pops = w.pops().values()
            .enumerate()
            .map(|(i, p)| (p.external_id.clone(), i))
            .collect::<HashMap<_, _>>();
        if pops.len() != w.pops().len() {
            return Err(error("Duplicate cohort scope POP"));
        }
        Ok(Self {
            pops,
            accounts,
            stocks,
            claims: reference_index(&w.obligations())?,
            money: reference_index(&w.money_reservations())?,
            tables,
            goods,
            refs: reference_index(
                w.needs()
                    .as_ref()
                    .map(|n| n.profile_refs.as_slice())
                    .unwrap_or(&[]),
            )?,
            financial_lengths: (
                w.accounts().len(),
                w.stockpiles().len(),
                w.obligations().len(),
            ),
            labor,
        })
    }
    fn sync_finance(&mut self, w: &dyn WorldRead) -> OpResult<()> {
        if w.accounts().len() < self.financial_lengths.0
            || w.stockpiles().len() < self.financial_lengths.1
            || w.obligations().len() < self.financial_lengths.2
        {
            return Err(error("Cohort scope financial rows removed"));
        }
        for (i, a) in w.accounts().values().enumerate().skip(self.financial_lengths.0) {
            add(&mut self.accounts, &a.owner_id, i)
        }
        for (i, s) in w.stockpiles().values()
            .enumerate()
            .skip(self.financial_lengths.1)
        {
            add(&mut self.stocks, &s.owner_id, i)
        }
        for (i, d) in w.obligations().values()
            .enumerate()
            .skip(self.financial_lengths.2)
        {
            for id in references(&serde_json::to_value(d)?) {
                add(&mut self.claims, &id, i)
            }
        }
        self.financial_lengths = (
            w.accounts().len(),
            w.stockpiles().len(),
            w.obligations().len(),
        );
        Ok(())
    }
    /// Original source POPs split at most once per phase. New claims are indexed for other original parties.
    pub fn select(&mut self, w: &dyn WorldRead, id: &ExternalId) -> OpResult<WorldState> {
        self.sync_finance(w)?;
        let p = w.pops()
            .at(
                *self
                    .pops
                    .get(id)
                    .ok_or_else(|| error("Unknown cohort scope POP"))?,
            )
            .filter(|p| &p.external_id == id)
            .ok_or_else(|| error("Cohort scope POP order changed"))?;
        let mut out = WorldState::default();
        out.campaign = w.campaign().clone();
        out.pops = vec![p.clone()];
        out.provinces = w.provinces().values()
            .filter(|province| province.id == p.province_id)
            .cloned()
            .collect();
        out.accounts = select(&w.accounts(), &self.accounts, id);
        out.stockpiles = select(&w.stockpiles(), &self.stocks, id);
        out.production_data.obligations = select(&w.obligations(), &self.claims, id);
        out.production_data.money_reservations =
            select(&w.money_reservations(), &self.money, id);
        out.production_data.catalog = w.production_catalog().clone();
        out.production_data.policies = w.production_policies().clone();
        macro_rules! rows {
            ($dst:expr,$src:expr,$key:literal) => {
                $dst = select($src, &self.tables[$key], id);
            };
        }
        rows!(
            out.production_data.storage_facilities,
            &w.storage_facilities(),
            "facilities"
        );
        rows!(
            out.production_data.storage_reservations,
            &w.storage_reservations(),
            "storage"
        );
        rows!(
            out.production_data.contracts,
            &w.contracts(),
            "contracts"
        );
        rows!(out.budget_data.jobs, &w.jobs(), "jobs");
        rows!(
            out.production_data.artisans,
            &w.artisans(),
            "artisans"
        );
        out.production_data.goods_reservations =
            select(&w.goods_reservations(), &self.goods, id);
        rows!(
            out.market_data.sale_offers,
            &w.sale_offers(),
            "sales"
        );
        rows!(
            out.market_data.maintenance_agreements,
            &w.maintenance_agreements(),
            "maintenance"
        );
        rows!(
            out.mobility_data.shipments,
            &w.shipments(),
            "shipments"
        );
        rows!(
            out.transport_data.shipment_recovery_plans,
            &w.shipment_recovery_plans(),
            "recovery_plans"
        );
        rows!(
            out.mobility_data.capitalist_entities,
            &w.capitalist_entities(),
            "capitalists"
        );
        rows!(
            out.mobility_data.project_investments,
            &w.project_investments(),
            "investments"
        );
        rows!(
            out.mobility_data.production_beneficiaries,
            &w.production_beneficiaries(),
            "production_rights"
        );
        rows!(
            out.mobility_data.factory_beneficiaries,
            &w.factory_beneficiaries(),
            "factory_rights"
        );
        if let Some(needs) = &w.needs() {
            out.market_data.needs = Some(kingmaker_core::market::NeedsData {
                schema_version: needs.schema_version.clone(),
                profile_refs: select(&needs.profile_refs, &self.refs, id),
                profiles: needs.profiles.clone(),
                resolver: needs.resolver.clone(),
                extra: needs.extra.clone(),
            });
        }
        out.market_data.territory = w.territory().values()
            .filter(|t| t.province_id == p.province_id)
            .cloned()
            .collect();
        if let Some(mobility) = &w.pop_mobility() {
            if !mobility.is_object() {
                return Err(error("Invalid cohort scope mobility"));
            }
            out.labor_data.pop_mobility = Some(
                json!({"unemployment":{text(id):mobility["unemployment"].get(text(id)).cloned().unwrap_or_else(||json!([]))}}),
            );
        }
        out.rebuild_account_index()
            .map_err(crate::operation_session::OperationError)?;
        Ok(out)
    }
}
