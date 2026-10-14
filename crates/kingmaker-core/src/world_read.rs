use crate::{row_store::{BorrowedRows, RowView}, WorldState};

macro_rules! define_read {
    (rows { $(($name:ident, $put:ident, $remove:ident, $replace:ident, $key:ty, $ty:ty, [$($field:ident),+], |$i:ident, $r:ident| $identity:expr),)* }
     singletons { $(($single:ident, $set:ident, $sty:ty, [$($sfield:ident),+]),)* }) => {
        pub trait WorldRead {
            fn production_data(&self)->crate::world_groups::ProductionView<'_> {
                crate::world_groups::ProductionView {
                    resource_operations: self.resource_operations(),
                    factories: self.factories(),
                    artisans: self.artisans(),
                    contracts: self.contracts(),
                    expansions: self.expansions(),
                    construction_projects: self.construction_projects(),
                    completed_construction: self.completed_construction(),
                    storage_facilities: self.storage_facilities(),
                    storage_reservations: self.storage_reservations(),
                    goods_reservations: self.goods_reservations(),
                    money_reservations: self.money_reservations(),
                    obligations: self.obligations(),
                    committed_operations: self.committed_operations(),
                    phase_receipts: self.production_phase_receipts(),
                    policies: self.production_policies(),
                    catalog: self.production_catalog(),
                }
            }
            fn market_data(&self)->crate::world_groups::MarketView<'_> {
                crate::world_groups::MarketView {
                    territory: self.territory(),
                    maintenance_agreements: self.maintenance_agreements(),
                    sale_offers: self.sale_offers(),
                    institutions: self.institutions(),
                    construction_catalog: self.market_construction_catalog(),
                    phase_receipts: self.market_phase_receipts(),
                    needs: self.needs(),
                    price_calibration: self.price_calibration(),
                    parameters: self.market_parameters(),
                    consumer_facility_by_province: self.consumer_facility_by_province(),
                }
            }
            fn budget_data(&self)->crate::world_groups::BudgetView<'_> {
                crate::world_groups::BudgetView {
                    employment: self.employment(),
                    adventurer_income_rates: self.adventurer_income_rates(),
                    jobs: self.jobs(),
                    phase_receipts: self.budget_phase_receipts(),
                    opening_receipts: self.budget_opening_receipts(),
                    profile_id: self.budget_profile_id(),
                    foundation_budget_policy: self.foundation_budget_policy(),
                }
            }
            fn labor_data(&self)->crate::world_groups::LaborView<'_> {
                crate::world_groups::LaborView {
                    profiles: self.profiles(),
                    species: self.species(),
                    phase_receipts: self.labor_phase_receipts(),
                    institution_rules: self.institution_rules(),
                    pop_mobility: self.pop_mobility(),
                }
            }
            fn mobility_data(&self)->crate::world_groups::MobilityView<'_> {
                crate::world_groups::MobilityView {
                    capitalist_entities: self.capitalist_entities(),
                    project_investments: self.project_investments(),
                    production_beneficiaries: self.production_beneficiaries(),
                    factory_beneficiaries: self.factory_beneficiaries(),
                    shipments: self.shipments(),
                    history_receipts: self.mobility_history_receipts(),
                    opening_receipts: self.mobility_opening_receipts(),
                    migration_receipts: self.mobility_migration_receipts(),
                    profession_receipts: self.mobility_profession_receipts(),
                    cohort_receipts: self.mobility_cohort_receipts(),
                    reference: self.mobility_reference(),
                }
            }
            fn transport_data(&self)->crate::world_groups::TransportView<'_> {
                crate::world_groups::TransportView {
                    transport_assets: self.transport_assets(),
                    transport_terminals: self.transport_terminals(),
                    supply_routes: self.supply_routes(),
                    route_reservations: self.route_reservations(),
                    navigation_geography: self.navigation_geography(),
                    shipment_recovery_plans: self.shipment_recovery_plans(),
                    phase_receipts: self.transport_phase_receipts(),
                    automatic_land_paths: self.automatic_land_paths(),
                }
            }
            fn income_data(&self)->crate::world_groups::IncomeView<'_> {
                crate::world_groups::IncomeView {
                    stock_contributions: self.stock_contributions(),
                    producer_income: self.producer_income(),
                    phase_receipts: self.income_phase_receipts(),
                }
            }
            fn construction_data(&self)->crate::world_groups::ConstructionView<'_> {
                crate::world_groups::ConstructionView {
                    investment_opportunities: self.investment_opportunities(),
                    construction_catalog: self.construction_construction_catalog(),
                    building_site_grants: self.building_site_grants(),
                    phase_receipts: self.construction_phase_receipts(),
                    construction_catalog_by_currency: self.construction_catalog_by_currency(),
                }
            }
            fn week_data(&self)->crate::world_groups::WeekView<'_> {
                crate::world_groups::WeekView {
                    phase_receipts: self.week_phase_receipts(),
                }
            }
            fn all_committed_operations(&self)->crate::history_view::HistoryView<'_,crate::ExternalId,crate::operations::CommittedOperation> {crate::history_view::HistoryView::new(self.history(),"committed_operations",self.committed_operations())}
            fn all_production_phase_receipts(&self)->crate::history_view::HistoryView<'_,u64,crate::production::ProductionPhaseReceipt> {crate::history_view::HistoryView::new(self.history(),"production_phase_receipts",self.production_phase_receipts())}
            fn all_market_phase_receipts(&self)->crate::history_view::HistoryView<'_,u64,crate::market::MarketPhaseReceipt> {crate::history_view::HistoryView::new(self.history(),"market_phase_receipts",self.market_phase_receipts())}
            fn all_budget_phase_receipts(&self)->crate::history_view::HistoryView<'_,u64,crate::budget::BudgetPhaseReceipt> {crate::history_view::HistoryView::new(self.history(),"budget_phase_receipts",self.budget_phase_receipts())}
            fn all_budget_opening_receipts(&self)->crate::history_view::HistoryView<'_,u64,crate::budget::BudgetPhaseReceipt> {crate::history_view::HistoryView::new(self.history(),"budget_opening_receipts",self.budget_opening_receipts())}
            fn all_labor_phase_receipts(&self)->crate::history_view::HistoryView<'_,u64,crate::labor::LaborPhaseReceipt> {crate::history_view::HistoryView::new(self.history(),"labor_phase_receipts",self.labor_phase_receipts())}
            fn all_transport_phase_receipts(&self)->crate::history_view::HistoryView<'_,u64,crate::transport::TransportPhaseReceipt> {crate::history_view::HistoryView::new(self.history(),"transport_phase_receipts",self.transport_phase_receipts())}
            fn all_income_phase_receipts(&self)->crate::history_view::HistoryView<'_,u64,crate::income::IncomePhaseReceipt> {crate::history_view::HistoryView::new(self.history(),"income_phase_receipts",self.income_phase_receipts())}
            fn all_construction_phase_receipts(&self)->crate::history_view::HistoryView<'_,u64,crate::construction::ConstructionPhaseReceipt> {crate::history_view::HistoryView::new(self.history(),"construction_phase_receipts",self.construction_phase_receipts())}
            fn all_week_phase_receipts(&self)->crate::history_view::HistoryView<'_,u64,crate::week::NativeWeekReceipt> {crate::history_view::HistoryView::new(self.history(),"week_phase_receipts",self.week_phase_receipts())}
            fn all_mobility_history_receipts(&self)->crate::history_view::HistoryView<'_,u64,crate::mobility::LaborHistoryReceipt> {crate::history_view::HistoryView::new(self.history(),"mobility_history_receipts",self.mobility_history_receipts())}
            fn all_mobility_opening_receipts(&self)->crate::history_view::HistoryView<'_,u64,crate::mobility::MobilityOpeningReceipt> {crate::history_view::HistoryView::new(self.history(),"mobility_opening_receipts",self.mobility_opening_receipts())}
            fn all_mobility_migration_receipts(&self)->crate::history_view::HistoryView<'_,u64,crate::mobility::MigrationPhaseReceipt> {crate::history_view::HistoryView::new(self.history(),"mobility_migration_receipts",self.mobility_migration_receipts())}
            fn all_mobility_profession_receipts(&self)->crate::history_view::HistoryView<'_,u64,crate::mobility::ProfessionPhaseReceipt> {crate::history_view::HistoryView::new(self.history(),"mobility_profession_receipts",self.mobility_profession_receipts())}
            fn all_mobility_cohort_receipts(&self)->crate::history_view::HistoryView<'_,u64,crate::mobility::CohortSplitReceipt> {crate::history_view::HistoryView::new(self.history(),"mobility_cohort_receipts",self.mobility_cohort_receipts())}
            fn source(&self) -> &Option<std::sync::Arc<crate::SourceSnapshot>>;
            fn history(&self) -> &Option<crate::history::HistoryBacking>;
            $(fn $name(&self) -> RowView<'_, $key, $ty>;)*
            $(fn $single(&self) -> &$sty;)*
        }
        pub struct BorrowedWorld<'a> {
            world: &'a WorldState,
            $($name: BorrowedRows<'a, $key, $ty>,)*
        }
        impl<'a> BorrowedWorld<'a> {
            pub fn new(world: &'a WorldState) -> Result<Self, String> {
crate::diagnostics::measure("world.index", crate::diagnostics::Visibility::Boundary, || {
                Ok(Self { world,
                    $($name: BorrowedRows::new(world.$($field).+.iter().enumerate().map(|($i, $r)| {
                        let _ = &$i;
                        ($identity, $r)
                    }))?,)*
                })
            
})
}
        }
        impl WorldRead for BorrowedWorld<'_> {
            fn source(&self) -> &Option<std::sync::Arc<crate::SourceSnapshot>> { &self.world.source }
            fn history(&self) -> &Option<crate::history::HistoryBacking> { &self.world.history }
            $(fn $name(&self) -> RowView<'_, $key, $ty> { RowView::Indexed(&self.$name) })*
            $(fn $single(&self) -> &$sty { &self.world.$($sfield).+ })*
        }
        impl WorldRead for WorldState {
            fn source(&self) -> &Option<std::sync::Arc<crate::SourceSnapshot>> { &self.source }
            fn history(&self) -> &Option<crate::history::HistoryBacking> { &self.history }
            $(fn $name(&self) -> RowView<'_, $key, $ty> {
                RowView::Slice {rows:&self.$($field).+,key:|$i,$r| {let _=(&$i,&$r);$identity}}
            })*
            $(fn $single(&self) -> &$sty { &self.$($sfield).+ })*
        }
    }
}
crate::world_fields!(define_read);

/// Transitional bridge for unconverted nested phases. Removed from the weekly path during integration.


use crate::*;
use std::collections::{HashMap,HashSet};
impl dyn WorldRead + '_ {
    pub fn account_by_address(&self,key:&AccountAddress)->Option<&AccountBalance> {self.accounts().get(key)}
    pub fn derive_enterprise_views<'a>(
        &'a self,
        accounts: impl Iterator<Item = &'a AccountBalance>,
        factories: impl Iterator<Item = &'a crate::production::FactoryEnterprise>,
        extra_max: u64,
    ) -> Result<Vec<EnterpriseRecord>, String> {
        self.derive_enterprise_views_with_artisans(
            accounts,
            factories,
            self.artisans().iter(),
            extra_max,
        )
    }
    pub fn derive_enterprise_views_with_artisans<'a>(
        &'a self,
        accounts: impl Iterator<Item = &'a AccountBalance>,
        factories: impl Iterator<Item = &'a crate::production::FactoryEnterprise>,
        artisans: impl Iterator<Item = &'a crate::production::ArtisanEnterprise>,
        extra_max: u64,
    ) -> Result<Vec<EnterpriseRecord>, String> {
        let accounts: Vec<_> = accounts.collect();
        let old: HashMap<_, _> = self.enterprises()
            .iter()
            .map(|e| (e.external_id.clone(), e.id))
            .collect();
        let used_max = self.enterprises()
            .iter()
            .map(|e| u64::from(e.id.0))
            .chain(accounts.iter().map(|a| u64::from(a.id.0)))
            .chain(std::iter::once(extra_max))
            .chain(self.stockpiles().iter().map(|s| u64::from(s.id)))
            .chain(self.pops().iter().map(|p| u64::from(p.id.0)))
            .chain(
                self.source()
                    .iter()
                    .flat_map(|s| s.dictionary.keys().copied()),
            )
            .max()
            .unwrap_or(0);
        let mut next = used_max;
        let mut views = Vec::new();
        let mut by_owner: HashMap<&ExternalId, Vec<&AccountBalance>> = HashMap::new();
        for account in accounts {
            by_owner.entry(&account.owner_id).or_default().push(account);
        }
        let producers = factories
            .map(|f| {
                (
                    &f.base,
                    false,
                    f.staff_count,
                    f.base.extra.get("working_capital"),
                )
            })
            .chain(artisans.map(|a| (&a.base, true, None, a.working_capital.as_ref())));
        for (base, is_artisan, staff, capital) in producers {
            let id = if let Some(id) = old.get(&base.id) {
                *id
            } else {
                next = next.checked_add(1).ok_or("Internal entity ID overflow")?;
                crate::EnterpriseId(
                    u32::try_from(next).map_err(|_| "Internal entity ID exceeds u32")?,
                )
            };
            let owner = base
                .owner_id
                .as_ref()
                .ok_or_else(|| format!("Enterprise {:?}: owner missing", base.id))?;
            let owned = by_owner.get(owner).map(Vec::as_slice).unwrap_or(&[]);
            let mut account_ids: Vec<_> = owned.iter().map(|a| a.id).collect();
            account_ids.sort();
            let mut eligible: Vec<_> = owned
                .iter()
                .filter(|a| {
                    base.payment_purpose
                        .as_ref()
                        .is_none_or(|p| &a.purpose == p)
                })
                .map(|a| a.id)
                .collect();
            eligible.sort();
            let level = base
                .extra
                .get("level")
                .map(|v| {
                    v.as_u64()
                        .and_then(|n| u32::try_from(n).ok())
                        .ok_or("Invalid enterprise level")
                })
                .transpose()?
                .unwrap_or(1);
            let operating_capital = capital
                .and_then(|c| c.get("minimum_minor"))
                .map(|v| {
                    serde_json::from_value::<Money>(v.clone())
                        .map(|m| m.units())
                        .map_err(|e| e.to_string())
                })
                .transpose()?;
            views.push(EnterpriseRecord {
                id,
                external_id: base.id.clone(),
                owner_id: owner.clone(),
                province_id: base.province_id,
                recipe_id: Some(base.method_id.clone()),
                level,
                account_id: if eligible.len() == 1 {
                    Some(eligible[0])
                } else {
                    None
                },
                account_ids,
                payment_purpose: base.payment_purpose.clone(),
                employees: Some(staff.unwrap_or_else(|| {
                    base.assignments
                        .iter()
                        .map(|a| a.workers_count * a.time_fraction)
                        .sum()
                })),
                is_artisan,
                operating_capital,
            });
        }
        Ok(views)
    }
    pub fn validate_links(&self) -> ValidationReport {
        let mut out = ValidationReport::default();
        let provinces: HashSet<_> = self.provinces().iter().map(|p| p.id).collect();
        if provinces.len() != self.provinces().len() {
            out.errors.push("Duplicate map province IDs".into());
        }
        let accounts: HashMap<_, _> = self.accounts().iter().map(|a| (a.id, a)).collect();
        if accounts.len() != self.accounts().len() {
            out.errors.push("Duplicate account IDs".into());
        }
        let pops: HashSet<_> = self.pops().iter().map(|p| &p.external_id).collect();
        if pops.len() != self.pops().len() {
            out.errors.push("Duplicate POP IDs".into());
        }
        for p in &self.pops() {
            if !provinces.contains(&p.province_id) {
                out.errors
                    .push(format!("POP {:?}: unknown province", p.external_id));
            }
            if let Some(id) = p.account_id {
                match accounts.get(&id) {
                    Some(a)
                        if a.owner_id == p.external_id
                            && Some(&a.currency_id) == p.currency_id.as_ref()
                            && a.purpose == "personal_savings" =>
                    {
                        ()
                    }
                    _ => out.errors.push(format!(
                        "POP {:?}: wrong personal savings account",
                        p.external_id
                    )),
                }
            }
            if ![p.workforce, p.employed, p.unemployed]
                .iter()
                .all(|v| v.is_finite() && *v >= 0.0)
                || p.employed > p.workforce + 1e-8
                || p.workforce > f64::from(p.size) + 1e-8
            {
                out.errors
                    .push(format!("POP {:?}: invalid labor quantities", p.external_id));
            }
        }
        for e in &self.enterprises() {
            if !provinces.contains(&e.province_id) {
                out.errors
                    .push(format!("Enterprise {:?}: unknown province", e.external_id));
            }
            for id in &e.account_ids {
                if !accounts.get(id).is_some_and(|a| a.owner_id == e.owner_id) {
                    out.errors.push(format!(
                        "Enterprise {:?}: wrong account {id:?}",
                        e.external_id
                    ));
                }
            }
        }
        let mut stocks = HashSet::new();
        for s in &self.stockpiles() {
            if !stocks.insert(s.id) {
                out.errors.push(format!("Duplicate stock ID {}", s.id));
            }
            if !provinces.contains(&s.province_id) {
                out.errors
                    .push(format!("Stock {:?}: unknown province", s.external_id));
            }
            if !s.quantity.is_finite() || s.quantity < 0.0 {
                out.errors
                    .push(format!("Stock {:?}: invalid quantity", s.external_id));
            }
            if s.beneficiary_pop_id
                .as_ref()
                .is_some_and(|id| !pops.contains(id))
            {
                out.errors
                    .push(format!("Stock {:?}: unknown beneficiary", s.external_id));
            }
        }
        for p in &self.provinces() {
            let (start, end) = p.pop_range;
            if start > end
                || end as usize > self.pops().len()
                || self.pops().values().skip(start as usize).take((end-start) as usize)
                    .any(|pop| pop.province_id != p.id)
            {
                out.errors
                    .push(format!("Province {:?}: invalid POP range", p.id));
            }
        }
        out.not_measured.push("Read-only link inspection does not measure complete employment assignments, warehouse capacity/reservations or legal-title operation conservation; use the closed economic phase audits".into());
        out
    }
}
