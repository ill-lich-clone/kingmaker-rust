//! Owned row overlays; a poisoned phase cannot publish any of its earlier work.
use kingmaker_core::world_read::WorldRead;
use kingmaker_core::operations::*;
use kingmaker_core::production::*;
use kingmaker_core::storage::*;
use kingmaker_core::*;
use serde_json::{json, Value};
use std::collections::HashMap;
use thiserror::Error;

#[derive(Debug, Error)]
#[error("Economic operation rejected: {0}")]
pub struct OperationError(pub String);
pub type OpResult<T> = Result<T, OperationError>;
fn reject(message: impl Into<String>) -> OperationError {
    OperationError(message.into())
}
impl From<kingmaker_core::history::HistoryError> for OperationError {
    fn from(e:kingmaker_core::history::HistoryError)->Self {reject(e.to_string())}
}
impl From<serde_json::Error> for OperationError {
    fn from(e: serde_json::Error) -> Self {
        reject(e.to_string())
    }
}
impl From<crate::money_math::MoneyMathError> for OperationError {
    fn from(e: crate::money_math::MoneyMathError) -> Self {
        reject(e.to_string())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyOutcome {
    Applied,
    AlreadyApplied,
}

pub struct OperationSession<'a> {
    pub base: &'a dyn WorldRead,
    account_positions: HashMap<AccountAddress, &'a AccountBalance>,
    stock_positions: HashMap<ExternalId, &'a StockpileRecord>,
    accounts: HashMap<AccountAddress, AccountBalance>,
    stocks: HashMap<ExternalId, StockpileRecord>,
    pub(crate) storage_rows: HashMap<ExternalId, StorageReservation>,
    pub(crate) factory_rows: HashMap<ExternalId, FactoryEnterprise>,
    pops: HashMap<ExternalId, &'a PopRecord>,
    contracts: HashMap<ExternalId, &'a LaborContract>,
    employments: HashMap<ExternalId, Vec<(&'a Producer, &'a LaborAssignment)>>,
    debts: HashMap<ExternalId, Obligation>,
    cash_reservations: HashMap<AccountAddress, MoneyReservation>,
    operations: Vec<EconomicOperation>,
    known: HashMap<ExternalId, Value>,
    history_error: std::cell::RefCell<Option<String>>,
    production_authorizations: HashMap<ExternalId, EconomicOperation>,
    market_authorizations: HashMap<ExternalId, EconomicOperation>,
    budget_authorizations: HashMap<ExternalId, EconomicOperation>,
    cohort_authorizations: HashMap<ExternalId, EconomicOperation>,
    income_authorizations: HashMap<ExternalId, EconomicOperation>,
    construction_authorizations: HashMap<ExternalId, EconomicOperation>,
    shipping_authorizations: HashMap<ExternalId, EconomicOperation>,
    cohort_stock_templates: HashMap<ExternalId, StockpileRecord>,
    market_reads: Option<crate::market_operations::MarketReadSet>,
    pub(crate) market_request: Option<kingmaker_core::market::MarketRequest>,
    pub(crate) purchase_policy: Option<crate::market_policy::MarketPolicy>,
    pub(crate) procurement_sources: Option<HashMap<ExternalId, ExternalId>>,
    pub(crate) native_maintenance:
        std::cell::OnceCell<Option<crate::production_validation::NativeMaintenanceEvidence>>,
    active_labor: std::cell::OnceCell<std::collections::HashSet<ExternalId>>,
    pub policies: ProductionPolicies,
    pub period: Option<String>,
    next_id: u32,
    poisoned: bool,
}
pub struct PreparedChanges {
    campaign: CampaignHeader,
    stock_contributions: Vec<kingmaker_core::income::StockContribution>,
    accounts: Vec<(AccountAddress, Option<AccountBalance>, AccountBalance)>,
    stocks: Vec<(ExternalId, Option<StockpileRecord>, StockpileRecord)>,
    storage: Vec<(ExternalId, Option<StorageReservation>, StorageReservation)>,
    factories: Vec<(ExternalId, FactoryEnterprise, FactoryEnterprise)>,
    debts: Vec<(ExternalId, Option<Obligation>, Obligation)>,
    cash_reservations: Vec<(AccountAddress, MoneyReservation, MoneyReservation)>,
    pub(crate) operations: Vec<EconomicOperation>,
    pops: Vec<PopRecord>,
    contracts: Vec<LaborContract>,
    catalog: Option<ProductionCatalog>,
    policies: ProductionPolicies,
    market_reads: Option<crate::market_operations::MarketReadSet>,
}
impl<'a> OperationSession<'a> {
    pub fn new(base: &'a dyn WorldRead) -> OpResult<Self> {
        let mut accounts = HashMap::new();
        for a in base.accounts().values() {
            crate::run_control::work("session_accounts")?;
            if accounts.insert(a.address(), a).is_some() {
                return Err(reject("Duplicate financial address"));
            }
        }
        let mut stocks = HashMap::new();
        for s in base.stockpiles().values() {
            crate::run_control::work("session_stocks")?;
            if stocks.insert(s.external_id.clone(), s).is_some() {
                return Err(reject("Duplicate stock identity"));
            }
        }
        let max = base.accounts().values()
            .map(|a| u64::from(a.id.0))
            .chain(base.stockpiles().values().map(|s| u64::from(s.id)))
            .chain(base.pops().values().map(|p| u64::from(p.id.0)))
            .chain(base.enterprises().values().map(|e| u64::from(e.id.0)))
            .chain(
                base.source()
                    .iter()
                    .flat_map(|s| s.max_dictionary_id()),
            )
            .max()
            .unwrap_or(0);
        let mut identities = std::collections::HashSet::new();
        for op in base.committed_operations().values() {
            crate::run_control::work("session_history")?;
            if !identities.insert(&op.event_id) || base.history().as_ref().map(|h|h.reader.get(&kingmaker_core::history::HistoryKey{namespace:"committed_operations".into(),id:op.event_id.clone()})).transpose()?.flatten().is_some() {
                return Err(reject("Duplicate committed event"));
            }
        }
        let pops = base.pops().values()
            .map(|p| (p.external_id.clone(), p))
            .collect::<HashMap<_, _>>();
        let contracts = base.contracts().values()
            .map(|c| (c.contract_id.clone(), c))
            .collect::<HashMap<_, _>>();
        if pops.len() != base.pops().len() || contracts.len() != base.contracts().len()
        {
            return Err(reject("Duplicate POP/contract identity"));
        }
        let mut employments: HashMap<_, Vec<_>> = HashMap::new();
        for p in base.resource_operations().values()
            .map(|p| &p.base)
            .chain(base.factories().values().map(|p| &p.base))
        {
            for a in &p.assignments {
                if let Some(id) = &a.employment_id {
                    employments.entry(id.clone()).or_default().push((p, a));
                }
            }
        }
        Ok(Self {
            pops,
            contracts,
            employments,
            base,
            account_positions: accounts,
            stock_positions: stocks,
            accounts: HashMap::new(),
            stocks: HashMap::new(),
            storage_rows: HashMap::new(),
            factory_rows: HashMap::new(),
            debts: HashMap::new(),
            cash_reservations: HashMap::new(),
            operations: vec![],
            known:HashMap::new(),
            history_error: Default::default(),
            production_authorizations: HashMap::new(),
            market_authorizations: HashMap::new(),
            budget_authorizations: HashMap::new(),
            cohort_authorizations: HashMap::new(),
            income_authorizations: HashMap::new(),
            construction_authorizations: HashMap::new(),
            shipping_authorizations: HashMap::new(),
            cohort_stock_templates: HashMap::new(),
            market_reads: None,
            market_request: None,
            purchase_policy: None,
            procurement_sources: None,
            native_maintenance: std::cell::OnceCell::new(),
            active_labor: std::cell::OnceCell::new(),
            policies: base.production_policies().clone(),
            period: None,
            next_id: u32::try_from(max).map_err(|_| reject("Internal ordinal exceeds u32"))?,
            poisoned: false,
        })
    }
    pub fn account(&self, addr: &AccountAddress) -> Option<&AccountBalance> {
        self.accounts.get(addr).or_else(|| {
            self.account_positions
                .get(addr)
                .copied()
        })
    }
    pub fn pop(&self, id: &ExternalId) -> Option<&PopRecord> {
        self.pops.get(id).copied()
    }
    pub fn contract(&self, id: &ExternalId) -> Option<&LaborContract> {
        self.contracts
            .get(id)
            .copied()
    }
    pub fn configure(&mut self, request: &ProductionRequest) -> OpResult<()> {
        self.native_maintenance.take();
        if !self.operations.is_empty()
            || request.expected_revision != self.base.campaign().revision
            || request.period.is_empty()
        {
            return Err(reject("Invalid production request revision/period"));
        }
        self.policies.taxes.extend(request.policies.taxes.clone());
        self.policies.wages.extend(request.policies.wages.clone());
        if !request.policies.priorities.is_empty() {
            self.policies.priorities = request.policies.priorities.clone();
        }
        self.period = Some(request.period.clone());
        Ok(())
    }
    pub fn stock(&self, id: &ExternalId) -> Option<&StockpileRecord> {
        self.stocks.get(id).or_else(|| {
            self.stock_positions
                .get(id)
                .copied()
        })
    }
    pub fn accounts(&self) -> impl Iterator<Item = &AccountBalance> {
        self.base.accounts().values()
            .filter(|a| !self.accounts.contains_key(&a.address()))
            .chain(self.accounts.values())
    }
    pub fn stocks(&self) -> impl Iterator<Item = &StockpileRecord> {
        self.base.stockpiles().values()
            .filter(|s| !self.stocks.contains_key(&s.external_id))
            .chain(self.stocks.values())
    }
    pub fn cash(&self, owner: &ExternalId, currency: &ExternalId, purpose: &str) -> Money {
        self.account(&AccountAddress {
            owner_id: owner.clone(),
            currency_id: currency.clone(),
            purpose: purpose.into(),
        })
        .map(|a| a.balance)
        .unwrap_or(Money::ZERO)
    }
    pub fn debt(&self, id: &ExternalId) -> Option<&Obligation> {
        self.debts.get(id).or_else(|| {
            self.base.obligations().values()
                .find(|d| &d.obligation_id == id)
        })
    }
    pub(crate) fn obligations(&self) -> impl Iterator<Item = &Obligation> {
        self.base.obligations().values()
            .filter(|d| !self.debts.contains_key(&d.obligation_id))
            .chain(self.debts.values())
    }
    pub fn storage_reservation(&self, id: &ExternalId) -> Option<&StorageReservation> {
        self.storage_rows.get(id).or_else(|| {
            self.base.storage_reservations().values()
                .find(|r| &r.id == id)
        })
    }
    pub fn storage_reservations(&self) -> impl Iterator<Item = &StorageReservation> {
        self.base.storage_reservations().values()
            .filter(|r| !self.storage_rows.contains_key(&r.id))
            .chain(self.storage_rows.values())
    }
    pub fn factory(&self, id: &ExternalId) -> Option<&FactoryEnterprise> {
        self.factory_rows.get(id).or_else(|| {
            self.base.factories().values()
                .find(|f| &f.base.id == id)
        })
    }
    pub fn committed(&self, id: &ExternalId) -> Option<std::borrow::Cow<'_,Value>> {
        match self.lookup_committed(id) {
            Ok(value)=>value,
            Err(e)=>{*self.history_error.borrow_mut()=Some(e.to_string());None}
        }
    }
    fn lookup_committed(&self,id:&ExternalId)->OpResult<Option<std::borrow::Cow<'_,Value>>> {
        if let Some(value)=self.known.get(id) {return Ok(Some(std::borrow::Cow::Borrowed(value)));}
        self.base.all_committed_operations().get(id)?.map(|r| {
            if &r.event_id!=id {return Err(reject("Archived event identity mismatch"));}
            canonical_event(&r.value).map(std::borrow::Cow::Owned)
        }).transpose()
    }
    pub(crate) fn operations(&self) -> &[EconomicOperation] {
        &self.operations
    }
    pub(crate) fn registry_changes(&self) -> ProductionRegistryChanges {
        let mut result = ProductionRegistryChanges::default();
        for (id, row) in &self.storage_rows {
            result.storage.push(RecordTransition {
                before: self
                    .base.storage_reservations().values()
                    .find(|r| &r.id == id)
                    .cloned(),
                after: row.clone(),
            });
        }
        for (id, row) in &self.factory_rows {
            result.factories.push(RecordTransition {
                before: self
                    .base.factories().values()
                    .find(|r| &r.base.id == id)
                    .cloned(),
                after: row.clone(),
            });
        }
        for (id, row) in &self.debts {
            result.obligations.push(RecordTransition {
                before: self
                    .base.obligations().values()
                    .find(|r| &r.obligation_id == id)
                    .cloned(),
                after: row.clone(),
            });
        }
        for (address, row) in &self.cash_reservations {
            result.money_reservations.push(RecordTransition {
                before: self
                    .base.money_reservations().values()
                    .find(|r| {
                        r.owner_id == address.owner_id
                            && r.currency_id == address.currency_id
                            && r.purpose == address.purpose
                    })
                    .cloned(),
                after: row.clone(),
            });
        }
        result
            .storage
            .sort_by_key(|r| r.after.id.json().to_string());
        result
            .factories
            .sort_by_key(|r| r.after.base.id.json().to_string());
        result
            .obligations
            .sort_by_key(|r| r.after.obligation_id.json().to_string());
        result.money_reservations.sort_by_key(|r| {
            serde_json::json!([r.after.owner_id, r.after.currency_id, r.after.purpose]).to_string()
        });
        result
    }
    pub fn stage(&mut self, op: &EconomicOperation) -> OpResult<ApplyOutcome> {
        if self.poisoned {
            return Err(reject("Phase is already poisoned"));
        }
        match self.prepare_operation(op) {
            Ok(v) => Ok(v),
            Err(e) => {
                self.poisoned = true;
                Err(reject(format!("{:?}: {}", op.event_id, e.0)))
            }
        }
    }
    pub fn poison(&mut self) {
        self.poisoned = true;
    }
    pub(crate) fn active_labor_ids(&self) -> OpResult<&std::collections::HashSet<ExternalId>> {
        if self.active_labor.get().is_none() {
            let roster = crate::labor_roster::build_labor_roster(self.base)?;
            let _ = self
                .active_labor
                .set(roster.by_employment.into_keys().collect());
        }
        Ok(self.active_labor.get().unwrap())
    }
    pub(crate) fn stage_labor(&mut self, op: &EconomicOperation) -> OpResult<ApplyOutcome> {
        if self.committed(&op.event_id).is_some()
            || !matches!(
                op.kind,
                OperationKind::LaborHiringFunding | OperationKind::StateBudgetFunding
            )
            || op.obligation.is_some()
            || !op.stock_changes.is_empty()
        {
            self.poison();
            return Err(reject("Invalid private hiring funding or orphan history"));
        }
        self.budget_authorizations
            .insert(op.event_id.clone(), op.clone());
        self.stage(op)
    }
    pub(crate) fn next_ordinal(&self) -> OpResult<u32> {
        self.next_id
            .checked_add(1)
            .ok_or_else(|| reject("Internal cohort ID overflow"))
    }
    /// A cohort planner supplies exact cloned titles; arbitrary public operations cannot authorize them.
    pub(crate) fn stage_cohort(
        &mut self,
        op: &EconomicOperation,
        templates: &[(ExternalId, StockpileRecord)],
    ) -> OpResult<ApplyOutcome> {
        if self.committed(&op.event_id).is_some()
            || !matches!(
                op.kind,
                OperationKind::PopCohortSplit | OperationKind::PopCohortClaim
            )
        {
            self.poison();
            return Err(reject("Invalid cohort operation or orphan history"));
        }
        self.cohort_authorizations
            .insert(op.event_id.clone(), op.clone());
        self.cohort_stock_templates
            .extend(templates.iter().cloned());
        self.stage(op)
    }
    /// Only a closed native income/choice planner can grant this exact payment authority.
    pub(crate) fn stage_income(&mut self, op: &EconomicOperation) -> OpResult<ApplyOutcome> {
        if self.committed(&op.event_id).is_some()
            || !matches!(
                op.kind,
                OperationKind::EnterpriseIncome
                    | OperationKind::IncomePhaseBoundary
                    | OperationKind::EnterpriseDividend
                    | OperationKind::ArtisanRetooling
                    | OperationKind::ArtisanRetoolingComplete
                    | OperationKind::ArtisanCapitalFunding
                    | OperationKind::ArtisanCapitalRelease
            )
            || !op.stock_changes.is_empty()
            || op.obligation.is_some()
        {
            self.poison();
            return Err(reject("Invalid income operation or orphan history"));
        }
        self.income_authorizations
            .insert(op.event_id.clone(), op.clone());
        self.stage(op)
    }
    pub(crate) fn stage_construction(&mut self, op: &EconomicOperation) -> OpResult<ApplyOutcome> {
        if self.committed(&op.event_id).is_some()
            || (!crate::construction_operations::construction_kind(&op.kind)
                && op.kind != OperationKind::Payroll)
            || op.obligation.is_some()
            || (op.kind != OperationKind::ConstructionMaterials && !op.stock_changes.is_empty())
            || op
                .stock_changes
                .iter()
                .any(|s| !s.delta.is_finite() || s.delta >= 0. || s.create.is_some())
            || (op.kind == OperationKind::ConstructionPhaseBoundary
                && (!op.debits.is_empty() || !op.credits.is_empty()))
        {
            self.poison();
            return Err(reject("Invalid construction operation or orphan history"));
        }
        self.construction_authorizations
            .insert(op.event_id.clone(), op.clone());
        self.stage(op)
    }
    pub(crate) fn stage_shipping(&mut self, op: &EconomicOperation) -> OpResult<ApplyOutcome> {
        let financial_only = matches!(
            op.kind,
            OperationKind::ShipmentRecovery
                | OperationKind::ShipmentReturnDispatch
                | OperationKind::StorageRent
                | OperationKind::StorageRentRefund
        );
        let marker = matches!(
            op.kind,
            OperationKind::ShipmentInterrupted
                | OperationKind::SupplyPhaseBoundary
                | OperationKind::StateProcurementPeriod
        );
        let allowed = crate::shipping_operations::shipping_kind(&op.kind)
            || matches!(
                op.kind,
                OperationKind::StorageRent
                    | OperationKind::StorageRentRefund
                    | OperationKind::Purchase
            );
        let invalid_stock = op.stock_changes.iter().any(|s| {
            !s.delta.is_finite()
                || match op.kind {
                    OperationKind::ShipmentDispatch => s.delta >= 0. || s.create.is_some(),
                    OperationKind::ShipmentArrival => s.delta <= 0.,
                    OperationKind::ShipmentReturnArrival => s.delta <= 0. || s.create.is_some(),
                    OperationKind::Purchase => s.delta == 0.,
                    _ => true,
                }
        });
        if !allowed
            || self.committed(&op.event_id).is_some()
            || op.obligation.is_some()
            || invalid_stock
            || (matches!(
                op.kind,
                OperationKind::ShipmentDispatch
                    | OperationKind::ShipmentArrival
                    | OperationKind::ShipmentReturnArrival
            ) && op.stock_changes.len() != 1)
            || (financial_only && !op.stock_changes.is_empty())
            || (marker
                && (!op.debits.is_empty()
                    || !op.credits.is_empty()
                    || !op.stock_changes.is_empty()))
        {
            self.poison();
            return Err(reject("Invalid shipping operation or orphan history"));
        }
        self.shipping_authorizations
            .insert(op.event_id.clone(), op.clone());
        self.stage(op)
    }
    pub(crate) fn stage_market_boundary(
        &mut self,
        op: &EconomicOperation,
    ) -> OpResult<ApplyOutcome> {
        if op.kind != OperationKind::MarketPhaseBoundary
            || !op.debits.is_empty()
            || !op.credits.is_empty()
            || !op.stock_changes.is_empty()
            || op.obligation.is_some()
            || self.committed(&op.event_id).is_some()
        {
            self.poison();
            return Err(reject("Invalid native market boundary"));
        }
        self.market_authorizations
            .insert(op.event_id.clone(), op.clone());
        self.stage(op)
    }
    pub(crate) fn stage_migration_arrival(
        &mut self,
        op: &EconomicOperation,
    ) -> OpResult<ApplyOutcome> {
        if op.kind != OperationKind::PopMigrationArrival
            || !op.debits.is_empty()
            || !op.credits.is_empty()
            || !op.stock_changes.is_empty()
            || op.obligation.is_some()
            || self.committed(&op.event_id).is_some()
        {
            self.poison();
            return Err(reject("Invalid native migration arrival or orphan history"));
        }
        self.cohort_authorizations
            .insert(op.event_id.clone(), op.clone());
        self.stage(op)
    }
    /// Only native planners may authorize a precisely derived fiscal operation.
    pub(crate) fn stage_budget(&mut self, op: &EconomicOperation) -> OpResult<ApplyOutcome> {
        if self.committed(&op.event_id).is_some() {
            self.poison();
            return Err(reject("Budget event exists without complete phase receipt"));
        }
        if !matches!(
            op.kind,
            OperationKind::AdventurerIncome
                | OperationKind::StateBudgetFunding
                | OperationKind::BudgetFunding
                | OperationKind::Payroll
                | OperationKind::BudgetPhaseBoundary
                | OperationKind::Blocked
        ) {
            self.poison();
            return Err(reject("Invalid native budget operation kind"));
        }
        if op.kind == OperationKind::BudgetPhaseBoundary
            && (!op.debits.is_empty()
                || !op.credits.is_empty()
                || !op.stock_changes.is_empty()
                || op.obligation.is_some())
        {
            self.poison();
            return Err(reject("Budget boundary cannot change money, goods or debt"));
        }
        self.budget_authorizations
            .insert(op.event_id.clone(), op.clone());
        self.stage(op)
    }
    pub(crate) fn authorize_production(&mut self, op: &EconomicOperation) {
        self.production_authorizations
            .insert(op.event_id.clone(), op.clone());
    }
    pub(crate) fn stage_week_boundary(&mut self, op: &EconomicOperation) -> OpResult<ApplyOutcome> {
        if op.kind != OperationKind::NativeWeekBoundary
            || !op.debits.is_empty()
            || !op.credits.is_empty()
            || !op.stock_changes.is_empty()
            || op.obligation.is_some()
        {
            self.poison();
            return Err(reject("Invalid native week boundary"));
        }
        self.budget_authorizations
            .insert(op.event_id.clone(), op.clone());
        self.stage(op)
    }
    pub fn configure_market(
        &mut self,
        request: &kingmaker_core::market::MarketRequest,
    ) -> OpResult<()> {
        if self.poisoned
            || request.expected_revision != self.base.campaign().revision
            || request.period.is_empty()
            || request
                .profile
                .food_norm_multiplier
                .is_some_and(|m| !m.is_finite() || m < 0.0)
        {
            self.poison();
            return Err(reject("Invalid market revision/period/profile"));
        }
        self.period = Some(request.period.clone());
        self.market_request = Some(request.clone());
        self.market_reads = Some(crate::market_operations::MarketReadSet::capture(self.base));
        Ok(())
    }
    pub(crate) fn authorize_purchase(
        &mut self,
        op: &EconomicOperation,
        order: &kingmaker_core::market::MarketOrder,
        proof: &crate::market_operations::PurchaseEvidence,
    ) -> OpResult<()> {
        match crate::market_operations::validate_purchase(self, op, order, proof) {
            Ok(()) => {
                self.market_authorizations
                    .insert(op.event_id.clone(), op.clone());
                Ok(())
            }
            Err(e) => {
                self.poison();
                Err(e)
            }
        }
    }
    pub(crate) fn authorize_food_conversion(&mut self, op: &EconomicOperation) -> OpResult<()> {
        match crate::market_operations::validate_food_conversion(self, op) {
            Ok(()) => {
                self.market_authorizations
                    .insert(op.event_id.clone(), op.clone());
                Ok(())
            }
            Err(e) => {
                self.poison();
                Err(e)
            }
        }
    }
    fn prepare_operation(&mut self, op: &EconomicOperation) -> OpResult<ApplyOutcome> {
        crate::run_control::check("operation")?;
        crate::market_operations::validate_creation_headers(op)?;
        if [
            "event_id",
            "type",
            "debits",
            "credits",
            "stock_changes",
            "obligation",
        ]
        .iter()
        .any(|key| op.meta.contains_key(*key))
        {
            return Err(reject("Metadata shadows typed operation headers"));
        }
        if op.event_id.as_str() == Some("") {
            return Err(reject("Empty event ID"));
        }
        let value = canonical_operation(op)?;
        if let Some(old) = self.lookup_committed(&op.event_id)? {
            return if old.as_ref() == &value {
                Ok(ApplyOutcome::AlreadyApplied)
            } else {
                Err(reject("Conflicting event replay"))
            };
        }
        self.validate_operation(op)?;
        let mut totals: HashMap<ExternalId, i128> = HashMap::new();
        let mut debit: HashMap<AccountAddress, i128> = HashMap::new();
        let mut credit: HashMap<AccountAddress, i128> = HashMap::new();
        for leg in &op.debits {
            if leg.amount_minor.0 < 0 {
                return Err(reject("Negative debit"));
            }
            *totals.entry(leg.currency_id.clone()).or_default() -= i128::from(leg.amount_minor.0);
            *debit.entry(leg.address()).or_default() += i128::from(leg.amount_minor.0);
        }
        for leg in &op.credits {
            if leg.amount_minor.0 < 0 {
                return Err(reject("Negative credit"));
            }
            *totals.entry(leg.currency_id.clone()).or_default() += i128::from(leg.amount_minor.0);
            *credit.entry(leg.address()).or_default() += i128::from(leg.amount_minor.0);
        }
        let issuance = if op.kind == OperationKind::AdventurerIncome
            && self.budget_authorizations.get(&op.event_id) == Some(op)
        {
            let amount: Money = serde_json::from_value(
                op.meta
                    .get("issuance_minor")
                    .cloned()
                    .ok_or_else(|| reject("Missing authorized issuance"))?,
            )?;
            let currency = id_meta(op, "currency_id")?;
            if amount.0 < 0
                || !op.debits.is_empty()
                || !op.stock_changes.is_empty()
                || op.obligation.is_some()
            {
                return Err(reject("Invalid issuance shape"));
            }
            Some((currency, i128::from(amount.0)))
        } else {
            None
        };
        if totals.iter().any(|(currency, v)| {
            *v != issuance
                .as_ref()
                .filter(|(c, _)| c == currency)
                .map(|(_, n)| *n)
                .unwrap_or(0)
        }) || issuance
            .as_ref()
            .is_some_and(|(c, n)| *n > 0 && !totals.contains_key(c))
        {
            return Err(reject("Money is unbalanced by currency"));
        }
        let mut next = self.next_id;
        let mut account_changes = Vec::new();
        let mut keys: Vec<_> = debit.keys().chain(credit.keys()).cloned().collect();
        keys.sort_by_key(|a| serde_json::to_string(a).unwrap());
        keys.dedup();
        for key in keys {
            let old = self.account(&key);
            let spent = debit.get(&key).copied().unwrap_or(0);
            let income = credit.get(&key).copied().unwrap_or(0);
            if debit.contains_key(&key) && old.is_none() {
                return Err(reject(format!("Unknown debit account {key:?}")));
            }
            let old_cash = old.map(|a| i128::from(a.balance.0)).unwrap_or(0);
            if spent > old_cash {
                return Err(reject(format!("Insufficient purpose cash {key:?}")));
            }
            let balance = i64::try_from(old_cash - spent + income)
                .map_err(|_| reject("Recipient money overflow"))?;
            if old.is_none() && income == 0 && !credit.contains_key(&key) {
                continue;
            }
            let mut row = if let Some(a) = old {
                a.clone()
            } else {
                if !self.may_create(op, &key)? {
                    return Err(reject("Unauthorized account creation"));
                }
                next = next
                    .checked_add(1)
                    .ok_or_else(|| reject("Internal ID overflow"))?;
                AccountBalance {
                    id: AccountId(next),
                    external_id: serde_json::to_string(&json!([
                        key.owner_id,
                        key.currency_id,
                        key.purpose
                    ]))?
                    .into(),
                    owner_id: key.owner_id.clone(),
                    currency_id: key.currency_id.clone(),
                    purpose: key.purpose.clone(),
                    balance: Money::ZERO,
                }
            };
            row.balance = Money(balance);
            account_changes.push((key, row));
        }
        let mut stock_changes: HashMap<ExternalId, StockpileRecord> = HashMap::new();
        for change in &op.stock_changes {
            if !change.delta.is_finite() {
                return Err(reject("Nonfinite stock delta"));
            }
            let mut row = if let Some(s) = stock_changes
                .get(&change.stock_id)
                .or_else(|| self.stock(&change.stock_id))
            {
                s.clone()
            } else {
                let c = change
                    .create
                    .as_ref()
                    .ok_or_else(|| reject("Unknown stock without create terms"))?;
                next = next
                    .checked_add(1)
                    .ok_or_else(|| reject("Internal stock ID overflow"))?;
                if let Some(template) = self.cohort_stock_templates.get(&change.stock_id) {
                    let mut row = template.clone();
                    row.id = next;
                    row.external_id = change.stock_id.clone();
                    row.quantity = 0.;
                    row
                } else {
                    StockpileRecord {
                        extra: c
                            .extra
                            .iter()
                            .filter(|(key, _)| {
                                !matches!(*key, "inventory_purpose" | "beneficiary_pop_id")
                            })
                            .map(|(k, v)| (k.to_owned(), v.clone()))
                            .collect(),
                        id: next,
                        external_id: change.stock_id.clone(),
                        province_id: c.province_id,
                        goods: Goods::from_external(c.goods_id.clone()),
                        quantity: 0.0,
                        purpose: c
                            .extra
                            .get("inventory_purpose")
                            .and_then(Value::as_str)
                            .unwrap_or("commercial")
                            .into(),
                        owner_id: c.owner_id.clone(),
                        unit: Some(c.unit.clone()),
                        facility_id: c.facility_id.clone(),
                        currency_id: c.currency_id.clone(),
                        inventory_origin: Some(c.inventory_origin.clone()),
                        method_kind: c.method_kind.clone(),
                        beneficiary_pop_id: c
                            .extra
                            .get("beneficiary_pop_id")
                            .filter(|v| !v.is_null() && !crate::needs::undefined(v))
                            .map(|v| ExternalId::from_json(v.clone()).map_err(reject))
                            .transpose()?,
                    }
                }
            };
            row.quantity += change.delta;
            if !row.quantity.is_finite() || row.quantity < -1e-9 {
                return Err(reject("Negative or nonfinite stock"));
            }
            if row.quantity < 0.0 {
                row.quantity = 0.0;
            }
            stock_changes.insert(change.stock_id.clone(), row);
        }
        if let Some(d) = &op.obligation {
            if d.amount_minor.0 < 0 {
                return Err(reject("Negative obligation"));
            }
        }
        // No writes above this line: every participating row and overflow was checked.
        for (key, row) in account_changes {
            self.accounts.insert(key, row);
        }
        self.stocks.extend(stock_changes);
        self.next_id = next;
        for leg in &op.debits {
            let key = leg.address();
            if let Some(old) = self.cash_reservations.get(&key).or_else(|| {
                self.base.money_reservations().values()
                    .find(|r| {
                        r.owner_id == key.owner_id
                            && r.currency_id == key.currency_id
                            && r.purpose == key.purpose
                    })
            }) {
                let mut row = old.clone();
                row.amount_minor =
                    Money(row.amount_minor.0.saturating_sub(leg.amount_minor.0).max(0));
                self.cash_reservations.insert(key, row);
            }
        }
        if let Some(d) = &op.obligation {
            self.debts.insert(d.obligation_id.clone(), d.clone());
        }
        self.known.insert(op.event_id.clone(), value);
        self.operations.push(op.clone());
        Ok(ApplyOutcome::Applied)
    }
    fn validate_operation(&self, op: &EconomicOperation) -> OpResult<()> {
        if op.kind == OperationKind::MarketPhaseBoundary {
            return if self.market_authorizations.get(&op.event_id) == Some(op) {
                Ok(())
            } else {
                Err(reject("Market boundary requires native planner authority"))
            };
        }
        if crate::construction_operations::construction_kind(&op.kind)
            || self.construction_authorizations.contains_key(&op.event_id)
        {
            return if self.construction_authorizations.get(&op.event_id) == Some(op) {
                Ok(())
            } else {
                Err(reject(
                    "Construction requires private current-phase authorization",
                ))
            };
        }
        if crate::shipping_operations::shipping_kind(&op.kind)
            || self.shipping_authorizations.contains_key(&op.event_id)
        {
            return if self.shipping_authorizations.get(&op.event_id) == Some(op) {
                Ok(())
            } else {
                Err(reject(
                    "Transport operation requires native transport-phase authorization",
                ))
            };
        }
        if matches!(
            op.kind,
            OperationKind::EnterpriseIncome
                | OperationKind::IncomePhaseBoundary
                | OperationKind::EnterpriseDividend
                | OperationKind::ArtisanRetooling
                | OperationKind::ArtisanRetoolingComplete
                | OperationKind::ArtisanCapitalFunding
                | OperationKind::ArtisanCapitalRelease
        ) {
            return if self.income_authorizations.get(&op.event_id) == Some(op) {
                Ok(())
            } else {
                Err(reject(
                    "Income/retooling requires private current-phase authorization",
                ))
            };
        }
        if matches!(
            op.kind,
            OperationKind::PopCohortSplit
                | OperationKind::PopCohortClaim
                | OperationKind::PopMigrationArrival
        ) {
            return if self.cohort_authorizations.get(&op.event_id) == Some(op) {
                Ok(())
            } else {
                Err(reject(
                    "Cohort operations require private current-phase authorization",
                ))
            };
        }
        if matches!(
            op.kind,
            OperationKind::AdventurerIncome
                | OperationKind::StateBudgetFunding
                | OperationKind::BudgetFunding
                | OperationKind::LaborHiringFunding
                | OperationKind::BudgetPhaseBoundary
                | OperationKind::NativeWeekBoundary
        ) || (op.kind == OperationKind::Payroll
            && self.budget_authorizations.contains_key(&op.event_id))
        {
            if self.budget_authorizations.get(&op.event_id) != Some(op)
                || !op.stock_changes.is_empty()
            {
                return Err(reject(
                    "Budget operation requires private current-phase authorization",
                ));
            }
            return Ok(());
        }
        if matches!(
            op.kind,
            OperationKind::Purchase | OperationKind::FoodConversion
        ) {
            return if self.market_authorizations.get(&op.event_id) == Some(op) {
                Ok(())
            } else {
                Err(reject(
                    "Market operations require private current-phase authorization",
                ))
            };
        }
        if op.kind != OperationKind::Production && !op.stock_changes.is_empty() {
            return Err(reject(
                "Only validated production may change stocks in this stage",
            ));
        }
        if op.kind == OperationKind::Transfer {
            if op.debits.len() != 1
                || op.credits.len() != 1
                || op.debits[0].amount_minor != op.credits[0].amount_minor
                || op.debits[0].currency_id != op.credits[0].currency_id
                || op.obligation.is_some()
            {
                return Err(reject("Invalid transfer shape"));
            }
            if self.account(&op.credits[0].address()).is_none() {
                return Err(reject("Unknown transfer payee"));
            }
        } else if op.kind == OperationKind::Payroll {
            let receipt: PayrollReceipt = serde_json::from_value(serde_json::to_value(op)?)?;
            let pop = self
                .pop(&receipt.pop_id)
                .ok_or_else(|| reject("Unknown payroll POP"))?;
            let matches: Vec<_> = self
                .employments
                .get(&receipt.employment_id)
                .into_iter()
                .flatten()
                .map(|(_, a)| *a)
                .collect();
            if matches.len() != 1 {
                return Err(reject(
                    "Payroll requires unique actual production employment",
                ));
            }
            if matches[0].pop_id != receipt.pop_id || pop.pop_type_id.as_str() == Some("slaves") {
                return Err(reject("Wrong payroll worker"));
            }
            let contract = matches[0]
                .contract_id
                .as_ref()
                .and_then(|id| self.contract(id))
                .ok_or_else(|| reject("Unknown payroll contract"))?;
            if contract.employer_entity_id.as_ref() != Some(&receipt.employer_id)
                || contract.currency_id.as_ref() != Some(&receipt.currency_id)
            {
                return Err(reject("Payroll employer/currency mismatch"));
            }
            let rate = contract_rate(contract)?;
            let multiplier = self
                .policies
                .wages
                .get(&external_text(&receipt.employer_id))
                .copied()
                .unwrap_or(1.0);
            let working = crate::money_math::rate_pay(&rate, receipt.planned_fte, multiplier)?;
            let idle_max = if let Some(rate) = &contract.guaranteed_idle_weekly_rate {
                crate::money_math::rate_pay(
                    rate,
                    (matches[0].workers_count * matches[0].time_fraction - receipt.planned_fte)
                        .max(0.0),
                    1.0,
                )?
            } else {
                Money::ZERO
            };
            if i128::from(receipt.gross_minor.0)
                != i128::from(working.0) + i128::from(receipt.idle_guarantee_minor.0)
                || receipt.idle_guarantee_minor > idle_max
            {
                return Err(reject("Payroll gross/idle differs from authored rates"));
            }
            if receipt.gross_minor < working
                || receipt.idle_guarantee_minor.0 > receipt.gross_minor.0 - working.0
            {
                return Err(reject("Payroll cannot erase authored working wages"));
            }
            if receipt.gross_minor.0 < 0
                || receipt.paid_gross_minor.0 < 0
                || receipt.paid_gross_minor > receipt.gross_minor
                || receipt.idle_guarantee_minor.0 < 0
                || !receipt.planned_fte.is_finite()
                || receipt.planned_fte < 0.0
                || receipt.planned_fte > matches[0].workers_count * matches[0].time_fraction + 1e-7
                || !receipt.actual_fte.is_finite()
                || receipt.actual_fte < 0.0
            {
                return Err(reject("Invalid payroll amounts or FTE"));
            }
            if !receipt.tax_rate.is_finite() || !(0.0..=1.0).contains(&receipt.tax_rate) {
                return Err(reject("Invalid payroll tax rate"));
            }
            let expected_tax = if let Some(state) = &pop.state_id {
                self.policies
                    .taxes
                    .get(&external_text(state))
                    .copied()
                    .or_else(|| {
                        contract
                            .expected_payroll
                            .as_ref()
                            .and_then(|v| v.get("tax_rate"))
                            .and_then(Value::as_f64)
                    })
                    .unwrap_or(0.0)
            } else {
                0.0
            };
            if receipt.tax_rate != expected_tax {
                return Err(reject("Payroll tax differs from current policy/contract"));
            }
            let tax =
                crate::money_math::multiply_money(receipt.paid_gross_minor, receipt.tax_rate)?;
            let actual = if receipt.gross_minor.0 > 0 {
                receipt.planned_fte * (receipt.paid_gross_minor.0 as f64)
                    / (receipt.gross_minor.0 as f64)
            } else {
                receipt.planned_fte
            };
            if receipt.tax_minor != tax
                || receipt.net_minor.0 != receipt.paid_gross_minor.0 - tax.0
                || (receipt.actual_fte - actual).abs() > 1e-7
            {
                return Err(reject("Payroll tax/net/actual FTE not explained"));
            }
            let treasury = pop
                .state_id
                .as_ref()
                .map(|s| ExternalId::from(format!("treasury:{}", external_text(s))));
            if receipt.tax_rate > 0.0 && (treasury.is_none() || receipt.tax_owner_id != treasury) {
                return Err(reject("Payroll treasury does not belong to POP state"));
            }
            let purpose = contract
                .payment_purpose
                .as_deref()
                .unwrap_or("employer_operating_capital");
            for d in &op.debits {
                if d.owner_id != receipt.employer_id
                    || d.currency_id != receipt.currency_id
                    || d.purpose != purpose
                {
                    return Err(reject("Wrong payroll debit purpose"));
                }
            }
            let mut net = 0i128;
            let mut tax_paid = 0i128;
            for c in &op.credits {
                if c.currency_id != receipt.currency_id {
                    return Err(reject("Wrong payroll currency"));
                }
                if c.owner_id == receipt.pop_id && c.purpose == "personal_savings" {
                    net += i128::from(c.amount_minor.0);
                } else if Some(&c.owner_id) == treasury.as_ref() && c.purpose == "operating_capital"
                {
                    tax_paid += i128::from(c.amount_minor.0);
                } else {
                    return Err(reject("Payroll credit is not a worker or lawful treasury"));
                }
            }
            if net != i128::from(receipt.net_minor.0)
                || tax_paid != i128::from(tax.0)
                || op
                    .debits
                    .iter()
                    .map(|d| i128::from(d.amount_minor.0))
                    .sum::<i128>()
                    != i128::from(receipt.paid_gross_minor.0)
            {
                return Err(reject("Payroll legs disagree with receipt"));
            }
            let unpaid = receipt.gross_minor.0 - receipt.paid_gross_minor.0;
            if unpaid > 0 {
                let d = op
                    .obligation
                    .as_ref()
                    .ok_or_else(|| reject("Unpaid wages lost obligation"))?;
                if d.obligation_id
                    != ExternalId::from(format!("debt:{}", external_text(&op.event_id)))
                {
                    return Err(reject("Payroll debt identity mismatch"));
                }
                if d.amount_minor.0 != unpaid
                    || d.payer_id != receipt.employer_id
                    || d.payee_id != receipt.pop_id
                    || d.currency_id != receipt.currency_id
                    || d.purpose != purpose
                    || d.tax_rate != Some(receipt.tax_rate)
                    || d.tax_owner_id != receipt.tax_owner_id
                {
                    return Err(reject("Unpaid wage obligation mismatch"));
                }
            } else if op.obligation.is_some() {
                return Err(reject("Fully paid wage creates debt"));
            }
        } else if op.kind == OperationKind::Production {
            self.validate_production(op)?;
            if !op.debits.is_empty() || !op.credits.is_empty() || op.obligation.is_some() {
                return Err(reject("Production cannot issue wages or arbitrary money"));
            }
        } else if op.kind == OperationKind::Blocked {
            if !op.debits.is_empty() || !op.credits.is_empty() || op.obligation.is_some() {
                return Err(reject("Blocked event changes money"));
            }
        } else {
            self.validate_financial_transition(op)?;
        }
        Ok(())
    }
    fn validate_production(&self, op: &EconomicOperation) -> OpResult<()> {
        if self.production_authorizations.get(&op.event_id) != Some(op) {
            return Err(reject(
                "Production requires internally verified paid labor/storage authorization",
            ));
        }
        let subject = id_meta(op, "subject_id")?;
        let producer = self
            .base.resource_operations().values()
            .map(|r| &r.base)
            .chain(self.base.factories().values().map(|r| &r.base))
            .chain(self.base.artisans().values().map(|r| &r.base))
            .find(|r| r.id == subject)
            .ok_or_else(|| reject("Unknown producer"))?;
        let owner = producer
            .owner_id
            .as_ref()
            .ok_or_else(|| reject("Unknown production owner"))?;
        let method = self
            .base.production_catalog()
            .as_ref()
            .and_then(|c| c.methods.iter().find(|m| m.method_id == producer.method_id))
            .ok_or_else(|| reject("Unknown production method"))?;
        let quantity = number_meta(op, "output_quantity")?;
        if quantity <= 0.0
            || method.status != "ready"
            || id_meta(op, "goods_id")? != method.output_goods_id
        {
            return Err(reject("Invalid actual production method/output"));
        }
        let mut by_goods: HashMap<ExternalId, f64> = HashMap::new();
        for change in &op.stock_changes {
            let (o, p, g, u, facility) = if let Some(s) = self.stock(&change.stock_id) {
                (
                    &s.owner_id,
                    s.province_id,
                    s.goods.external_id(),
                    s.unit.as_ref(),
                    s.facility_id.as_ref(),
                )
            } else {
                let c = change
                    .create
                    .as_ref()
                    .ok_or_else(|| reject("Missing production stock creation"))?;
                if c.facility_id.is_none()
                    || c.currency_id.is_none()
                    || c.method_kind.as_deref() != Some(&method.kind)
                {
                    return Err(reject(
                        "Production output creation requires real facility/currency/method",
                    ));
                }
                (
                    &c.owner_id,
                    c.province_id,
                    c.goods_id.clone(),
                    Some(&c.unit),
                    c.facility_id.as_ref(),
                )
            };
            if o != owner || p != producer.province_id {
                return Err(reject("Production changed another owner/province stock"));
            }
            if change.delta > 0.0
                && (g != method.output_goods_id
                    || u != Some(&method.output_unit)
                    || facility != producer.facility_id.as_ref())
            {
                return Err(reject(
                    "Production output target does not belong to method/facility",
                ));
            }
            if change.delta < 0.0 {
                if u != Some(&crate::production_validation::goods_unit(self.base, &g)?) {
                    return Err(reject("Recipe input commodity unit mismatch"));
                }
                let s = self
                    .stock(&change.stock_id)
                    .ok_or_else(|| reject("Input missing"))?;
                let held: f64 = self
                    .base.goods_reservations().values()
                    .filter(|r| r.status == "held" && r.stock_id == s.external_id)
                    .map(|r| r.quantity)
                    .sum();
                if s.purpose == "food_reserve" || s.quantity + change.delta + 1e-8 < held {
                    return Err(reject("Production consumed held/food inputs"));
                }
            }
            *by_goods.entry(g).or_default() += change.delta;
        }
        let mut expected: HashMap<ExternalId, f64> =
            HashMap::from([(method.output_goods_id.clone(), quantity)]);
        for input in &method.inputs {
            *expected.entry(input.goods_id.clone()).or_default() -= input.amount * quantity;
        }
        if by_goods.len() != expected.len()
            || expected.iter().any(|(g, q)| {
                by_goods
                    .get(g)
                    .is_none_or(|actual| !actual.is_finite() || (actual - q).abs() > 1e-8)
            })
        {
            return Err(reject("Production stock changes do not implement recipe"));
        }
        Ok(())
    }
    fn validate_financial_transition(&self, op: &EconomicOperation) -> OpResult<()> {
        if op.kind == OperationKind::DebtPayment {
            let updated = op
                .obligation
                .as_ref()
                .ok_or_else(|| reject("Debt payment requires remaining obligation"))?;
            let old = self
                .debt(&updated.obligation_id)
                .ok_or_else(|| reject("Unknown debt"))?;
            if updated.payer_id != old.payer_id
                || updated.payee_id != old.payee_id
                || updated.currency_id != old.currency_id
                || updated.purpose != old.purpose
                || updated.amount_minor > old.amount_minor
            {
                return Err(reject("Debt identity or balance changed illegally"));
            }
            let mut unchanged = updated.clone();
            unchanged.amount_minor = old.amount_minor;
            if &unchanged != old {
                return Err(reject("Debt contract changed during payment"));
            }
            let paid = Money(old.amount_minor.0 - updated.amount_minor.0);
            let tax = crate::money_math::multiply_money(paid, old.tax_rate.unwrap_or(0.0))?;
            let expected_debits = if paid.0 > 0 {
                vec![MoneyLeg {
                    owner_id: old.payer_id.clone(),
                    currency_id: old.currency_id.clone(),
                    purpose: old.purpose.clone(),
                    amount_minor: paid,
                }]
            } else {
                vec![]
            };
            let mut expected_credits = vec![];
            if paid.0 > 0 {
                expected_credits.push(MoneyLeg {
                    owner_id: old.payee_id.clone(),
                    currency_id: old.currency_id.clone(),
                    purpose: old
                        .payee_purpose
                        .clone()
                        .unwrap_or("personal_savings".into()),
                    amount_minor: Money(paid.0 - tax.0),
                });
            }
            if tax.0 > 0 {
                expected_credits.push(MoneyLeg {
                    owner_id: old
                        .tax_owner_id
                        .clone()
                        .ok_or_else(|| reject("Taxed debt lacks treasury"))?,
                    currency_id: old.currency_id.clone(),
                    purpose: "operating_capital".into(),
                    amount_minor: tax,
                });
            }
            if op.debits != expected_debits || op.credits != expected_credits {
                return Err(reject("Debt payment legs do not explain remaining amount"));
            }
            for d in &op.debits {
                if d.owner_id != old.payer_id
                    || d.currency_id != old.currency_id
                    || d.purpose != old.purpose
                {
                    return Err(reject("Wrong debt payer"));
                }
            }
        } else {
            let reservation_id = id_meta(op, "reservation_id")?;
            let reservation = self
                .storage_reservation(&reservation_id)
                .ok_or_else(|| reject("Rent reservation missing"))?;
            let facility = reservation.facility_id.clone();
            let f = self
                .base.storage_facilities().values()
                .find(|f| f.id == facility)
                .ok_or_else(|| reject("Rent facility missing"))?;
            let renter = reservation.owner_id.clone();
            let terms = reservation
                .lease_terms
                .as_ref()
                .ok_or_else(|| reject("Rent terms missing"))?;
            let currency = terms
                .currency_id
                .as_ref()
                .ok_or_else(|| reject("Rent currency missing"))?;
            let purpose = terms
                .purpose
                .as_ref()
                .ok_or_else(|| reject("Rent purpose missing"))?;
            let (payer, payer_purpose, payee, payee_purpose) =
                if op.kind == OperationKind::StorageRent {
                    (&renter, purpose.as_str(), &f.owner_id, "operating_capital")
                } else {
                    (&f.owner_id, "operating_capital", &renter, purpose.as_str())
                };
            let due = if op.kind == OperationKind::StorageRent {
                reservation.fee_paid_minor.unwrap_or(Money::ZERO)
            } else {
                if terms.cancellation_policy.as_deref() != Some("refund_unused") {
                    return Err(reject("Lease forbids refund"));
                }
                crate::money_math::multiply_money(
                    reservation.fee_paid_minor.unwrap_or(Money::ZERO),
                    reservation.quantity
                        / reservation
                            .accepted_quantity
                            .ok_or_else(|| reject("Missing accepted rent quantity"))?,
                )?
            };
            let paid = if op.kind == OperationKind::StorageRent {
                due
            } else {
                due.min(self.cash(payer, currency, payer_purpose))
            };
            if op
                .debits
                .iter()
                .map(|l| i128::from(l.amount_minor.0))
                .sum::<i128>()
                != i128::from(paid.0)
                || op
                    .credits
                    .iter()
                    .map(|l| i128::from(l.amount_minor.0))
                    .sum::<i128>()
                    != i128::from(paid.0)
            {
                return Err(reject("Rent amount differs from saved lease"));
            }
            if due > paid {
                let d = op
                    .obligation
                    .as_ref()
                    .ok_or_else(|| reject("Rent refund lost unpaid debt"))?;
                if &d.payer_id != payer
                    || &d.payee_id != payee
                    || &d.currency_id != currency
                    || d.purpose != payer_purpose
                    || d.payee_purpose.as_deref() != Some(payee_purpose)
                    || d.amount_minor.0 != due.0 - paid.0
                    || d.tax_rate.unwrap_or(0.0) != 0.0
                {
                    return Err(reject("Wrong lease refund debt"));
                }
            } else if op.obligation.is_some() {
                return Err(reject("Unexpected lease debt"));
            }
            if op.debits.iter().any(|d| {
                &d.owner_id != payer || &d.currency_id != currency || d.purpose != payer_purpose
            }) || op.credits.iter().any(|c| {
                &c.owner_id != payee || &c.currency_id != currency || c.purpose != payee_purpose
            }) {
                return Err(reject("Rent legs do not implement lease"));
            }
        }
        Ok(())
    }
    fn may_create(&self, op: &EconomicOperation, key: &AccountAddress) -> OpResult<bool> {
        if self.shipping_authorizations.get(&op.event_id) == Some(op) {
            return Ok(op.credits.iter().any(|leg| leg.address() == *key));
        }
        if self.construction_authorizations.get(&op.event_id) == Some(op) {
            return Ok(op.credits.iter().any(|leg| leg.address() == *key));
        }
        if self.income_authorizations.get(&op.event_id) == Some(op) {
            return Ok(op.credits.iter().any(|leg| leg.address() == *key));
        }
        if self.cohort_authorizations.get(&op.event_id) == Some(op) {
            return Ok(op.credits.iter().any(|leg| leg.address() == *key));
        }
        if self.budget_authorizations.get(&op.event_id) == Some(op) {
            return Ok(op.credits.iter().any(|leg| leg.address() == *key));
        }
        Ok(match op.kind {
            OperationKind::Payroll => {
                let r: PayrollReceipt = serde_json::from_value(serde_json::to_value(op)?)?;
                key.currency_id == r.currency_id
                    && ((key.owner_id == r.pop_id && key.purpose == "personal_savings")
                        || (r.tax_owner_id.as_ref() == Some(&key.owner_id)
                            && key.purpose == "operating_capital"))
            }
            OperationKind::DebtPayment => op.obligation.as_ref().is_some_and(|d| {
                key.currency_id == d.currency_id
                    && ((key.owner_id == d.payee_id
                        && key.purpose == d.payee_purpose.as_deref().unwrap_or("personal_savings"))
                        || (d.tax_owner_id.as_ref() == Some(&key.owner_id)
                            && key.purpose == "operating_capital"))
            }),
            OperationKind::StorageRent | OperationKind::StorageRentRefund => true, // identities already checked against actual lease above
            OperationKind::Purchase => {
                self.market_authorizations.get(&op.event_id) == Some(op)
                    && op.credits.iter().any(|c| &c.address() == key)
            }
            _ => false,
        })
    }
    pub fn finish(self) -> OpResult<PreparedChanges> {
        if let Some(e)=self.history_error.borrow().as_ref() {return Err(reject(e.clone()));}
        if self.poisoned {
            return Err(reject("Poisoned phase cannot commit"));
        }
        let mut pop_ids = std::collections::HashSet::new();
        let mut contract_ids = std::collections::HashSet::new();
        for op in &self.operations {
            if op.kind == OperationKind::Payroll {
                let r: PayrollReceipt = serde_json::from_value(serde_json::to_value(op)?)?;
                pop_ids.insert(r.pop_id);
                if let Some(rows) = self.employments.get(&r.employment_id) {
                    for (_, a) in rows {
                        if let Some(id) = &a.contract_id {
                            contract_ids.insert(id.clone());
                        }
                    }
                }
            }
        }
        let pops = pop_ids
            .into_iter()
            .map(|id| self.pop(&id).expect("validated POP").clone())
            .collect();
        let contracts = contract_ids
            .into_iter()
            .map(|id| self.contract(&id).expect("validated contract").clone())
            .collect();
        let accounts = self
            .accounts
            .into_iter()
            .map(|(k, v)| {
                let old = self
                    .account_positions
                    .get(&k)
                    .map(|r| (**r).clone());
                (k, old, v)
            })
            .collect();
        let stocks = self
            .stocks
            .into_iter()
            .map(|(k, v)| {
                let old = self
                    .stock_positions
                    .get(&k)
                    .map(|r| (**r).clone());
                (k, old, v)
            })
            .collect();
        let storage = self
            .storage_rows
            .into_iter()
            .map(|(k, v)| {
                let old = self
                    .base.storage_reservations().values()
                    .find(|r| r.id == k)
                    .cloned();
                (k, old, v)
            })
            .collect();
        let factories = self
            .factory_rows
            .into_iter()
            .map(|(k, v)| {
                let old = self
                    .base.factories().values()
                    .find(|r| r.base.id == k)
                    .expect("existing factory")
                    .clone();
                (k, old, v)
            })
            .collect();
        let debts = self
            .debts
            .into_iter()
            .map(|(k, v)| {
                let old = self
                    .base.obligations().values()
                    .find(|r| r.obligation_id == k)
                    .cloned();
                (k, old, v)
            })
            .collect();
        let cash_reservations = self
            .cash_reservations
            .into_iter()
            .map(|(k, v)| {
                let old = self
                    .base.money_reservations().values()
                    .find(|r| {
                        r.owner_id == k.owner_id
                            && r.currency_id == k.currency_id
                            && r.purpose == k.purpose
                    })
                    .expect("existing reserve")
                    .clone();
                (k, old, v)
            })
            .collect();
        Ok(PreparedChanges {
            stock_contributions: self.base.stock_contributions().values().cloned().collect::<Vec<_>>(),
            market_reads: self.market_reads,
            pops,
            contracts,
            catalog: self.base.production_catalog().clone(),
            policies: self.base.production_policies().clone(),
            campaign: self.base.campaign().clone(),
            accounts,
            stocks,
            storage,
            factories,
            debts,
            cash_reservations,
            operations: self.operations,
        })
    }
}
pub(crate) fn canonical_operation(op: &EconomicOperation) -> OpResult<Value> {
    let mut op = op.clone();
    for key in [
        "gross_minor",
        "paid_gross_minor",
        "tax_minor",
        "net_minor",
        "idle_guarantee_minor",
        "paid_minor",
        "issuance_minor",
    ] {
        if let Some(value) = op.meta.get_mut(key) {
            *value = serde_json::to_value(serde_json::from_value::<Money>(value.clone())?)?;
        }
    }
    for key in [
        "planned_fte",
        "actual_fte",
        "tax_rate",
        "output_quantity",
        "planned_possible_output",
    ] {
        if let Some(value) = op.meta.get_mut(key) {
            if let Some(n) = value.as_f64() {
                *value = json!(n);
            }
        }
    }
    Ok(serde_json::to_value(op)?)
}
fn canonical_event(value: &Value) -> OpResult<Value> {
    if matches!(
        value.get("type").and_then(Value::as_str),
        Some(
            "transfer"
                | "payroll"
                | "debt_payment"
                | "storage_rent"
                | "storage_rent_refund"
                | "production"
                | "blocked"
                | "adventurer_income"
                | "state_budget_funding"
                | "budget_funding"
        )
    ) {
        canonical_operation(&serde_json::from_value(value.clone())?)
    } else {
        Ok(value.clone())
    }
}
pub fn external_text(id: &ExternalId) -> String {
    match id {
        ExternalId::String(s) => s.to_string(),
        ExternalId::Number(n) => n.to_string(),
    }
}
pub fn contract_rate(c: &LaborContract) -> OpResult<MoneyRate> {
    if let Some(r) = &c.weekly_wage_rate {
        return Ok(r.clone());
    }
    let (n, d) = crate::money_math::decimal_ratio(
        &c.wage_weekly_per_worker
            .ok_or_else(|| reject("Missing wage rate"))?
            .to_string(),
    )?;
    Ok(MoneyRate {
        numerator: n.to_string(),
        denominator: d.to_string(),
    })
}
pub fn id_meta(op: &EconomicOperation, key: &str) -> OpResult<ExternalId> {
    ExternalId::from_json(
        op.meta
            .get(key)
            .cloned()
            .ok_or_else(|| reject(format!("Missing {key}")))?,
    )
    .map_err(reject)
}
pub fn number_meta(op: &EconomicOperation, key: &str) -> OpResult<f64> {
    op.meta
        .get(key)
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite())
        .ok_or_else(|| reject(format!("Invalid {key}")))
}

pub fn apply_prepared(world:&mut WorldState,d:PreparedChanges)->OpResult<()> {
    let base=kingmaker_core::world_read::BorrowedWorld::new(world).map_err(reject)?;
    let mut transaction=crate::world_transaction::WorldTransaction::new(&base);
    transaction.apply_phase(d)?;
    let delta=transaction.finish()?;
    crate::world_transaction::apply_world_delta(world,delta)
}
pub(crate) fn apply_prepared_phase(world: &mut crate::world_transaction::WorldTransaction, d: PreparedChanges) -> OpResult<()> {
    if world.campaign() != &d.campaign {
        return Err(reject("Prepared campaign/revision changed"));
    }
    crate::operation_history::validate_active_history_keys(world)?;
    let mut repeated = 0;
    for op in &d.operations {
        if let Some(old) = world.all_committed_operations().get(&op.event_id)? {
            if old.event_id != op.event_id || canonical_event(&old.value)? != canonical_operation(op)? {
                return Err(reject(
                    "Prepared event conflicts with new committed history",
                ));
            }
            repeated += 1;
        }
    }
    if repeated > 0 {
        if repeated == d.operations.len() {
            return Ok(());
        }
        return Err(reject(
            "Prepared phase partially applied in intervening history",
        ));
    }
    if world.production_catalog() != &d.catalog || world.production_policies() != &d.policies {
        return Err(reject("Prepared catalogue/policies changed"));
    }
    if world.stock_contributions() != d.stock_contributions {
        return Err(reject("Prepared producer stock provenance changed"));
    }
    let next_provenance = crate::stock_provenance::next_stock_provenance(world, &d.operations)?;
    if d.market_reads.as_ref().is_some_and(|r| !r.unchanged(world)) {
        return Err(reject("Prepared market consulted rows changed"));
    }
    let pop_rows: HashMap<_, _> = world.pops().iter().map(|p| (&p.external_id, p)).collect();
    let contract_rows: HashMap<_, _> = world.contracts()
        .iter()
        .map(|c| (&c.contract_id, c))
        .collect();
    if d.pops
        .iter()
        .any(|p| pop_rows.get(&p.external_id) != Some(&p))
        || d.contracts
            .iter()
            .any(|c| contract_rows.get(&c.contract_id) != Some(&c))
    {
        return Err(reject("Prepared worker/contract changed"));
    }
    let mut addresses = std::collections::HashSet::new();
    if world.accounts()
        .iter()
        .any(|a| !addresses.insert(a.address()))
    {
        return Err(reject("Duplicate account before commit"));
    }
    let mut stock_ids = std::collections::HashSet::new();
    if world.stockpiles()
        .iter()
        .any(|s| !stock_ids.insert(s.external_id.clone()))
    {
        return Err(reject("Duplicate stock before commit"));
    }
    for (k, old, _) in &d.accounts {
        if world.accounts().get(k) != old.as_ref() {
            return Err(reject("Prepared account precondition changed"));
        }
    }
    for (k, old, _) in &d.stocks {
        if world.stockpiles().get(k) != old.as_ref() {
            return Err(reject("Prepared stock precondition changed"));
        }
    }
    for (k, old, _) in &d.storage {
        if world.storage_reservations().get(k) != old.as_ref() {
            return Err(reject("Prepared storage precondition changed"));
        }
    }
    for (k, old, _) in &d.factories {
        if world.factories().get(k) != Some(old) {
            return Err(reject("Prepared factory precondition changed"));
        }
    }
    for (k, old, _) in &d.debts {
        if world.obligations().get(k) != old.as_ref() {
            return Err(reject("Prepared debt precondition changed"));
        }
    }
    for (k, old, _) in &d.cash_reservations {
        if world.money_reservations().iter().find(|r| {
            r.owner_id == k.owner_id && r.currency_id == k.currency_id && r.purpose == k.purpose
        }) != Some(old)
        {
            return Err(reject("Prepared reservation precondition changed"));
        }
    }
    let views = if world.production_catalog().is_some() {
        // Lookup only: the existing row iterators still determine publication order.
        let changed_accounts: std::collections::HashSet<AccountAddress> =
            d.accounts.iter().map(|(key, _, _)| key.clone()).collect();
        let mut changed_factories: HashMap<ExternalId, &FactoryEnterprise> = HashMap::new();
        for (key, _, row) in &d.factories {
            // Retain the first match, exactly as the previous linear find did.
            changed_factories.entry(key.clone()).or_insert(row);
        }
        Some(
            (world as &dyn WorldRead).derive_enterprise_views(
                    world.accounts()
                        .iter()
                        .filter(|a| !changed_accounts.contains(&a.address()))
                        .chain(d.accounts.iter().map(|(_, _, r)| r)),
                    world.factories().iter().map(|f| {
                        changed_factories.get(&f.base.id).copied().unwrap_or(f)
                    }),
                    d.stocks
                        .iter()
                        .map(|(_, _, r)| u64::from(r.id))
                        .max()
                        .unwrap_or(0),
                )
                .map_err(reject)?,
        )
    } else {
        None
    };
    let receipts: Vec<_> = d
        .operations
        .iter()
        .map(|op| {
            Ok(CommittedOperation {
                event_id: op.event_id.clone(),
                value: serde_json::to_value(op)?,
            })
        })
        .collect::<OpResult<_>>()?;
    // All fallible economic derivation and preconditions precede the staged writes.
    world.replace_stock_contributions(next_provenance)?;
    let mut accounts=d.accounts;
    accounts.sort_by_key(|(_,_,row)|row.id.0);
    for (key,_,row) in accounts {world.put_accounts(key,row)?;}
    let mut stocks=d.stocks;
    stocks.sort_by_key(|(_,_,row)|row.id);
    for (key,_,row) in stocks {world.put_stockpiles(key,row)?;}
    let mut storage=d.storage;
    storage.sort_by_cached_key(|(id,_,_)|id.json().to_string());
    for (key,_,row) in storage {world.put_storage_reservations(key,row)?;}
    for (key,_,row) in d.factories {world.put_factories(key,row)?;}
    let mut debts=d.debts;
    debts.sort_by_cached_key(|(id,_,_)|id.json().to_string());
    for (key,_,row) in debts {world.put_obligations(key,row)?;}
    for (address,_,row) in d.cash_reservations {
        let key=world.money_reservations().keyed().find(|(_,r)| r.owner_id==address.owner_id && r.currency_id==address.currency_id && r.purpose==address.purpose).map(|(k,_)|k).expect("checked reserve");
        world.put_money_reservations(key,row)?;
    }
    let pops:Vec<_>=world.pops().values().filter_map(|pop| {
        let currency=pop.currency_id.as_ref()?;
        let address=AccountAddress {owner_id:pop.external_id.clone(),currency_id:currency.clone(),purpose:"personal_savings".into()};
        let account=world.accounts().get(&address)?;
        if pop.account_id==Some(account.id) && pop.account_status=="resolved" {return None;}
        let mut row=pop.clone(); row.account_id=Some(account.id);row.account_status="resolved".into();Some(row)
    }).collect();
    for pop in pops {world.put_pops(pop.external_id.clone(),pop)?;}
    for receipt in receipts {world.put_committed_operations(receipt.event_id.clone(),receipt)?;}
    if let Some(views)=views {world.replace_enterprises(views)?;}
    Ok(())
}

#[cfg(test)]
mod publication_order_tests {
    use super::*;

    fn storage(id: Value) -> StorageReservation {
        serde_json::from_value(json!({
            "id": id, "owner_id": "owner", "facility_id": "warehouse",
            "goods_id": "iron", "unit": "ton", "status": "released", "quantity": 0,
            "accepted_quantity": 1.1, "released_quantity": 1.1,
            "author_note": "retain this exact row"
        }))
        .unwrap()
    }

    #[test]
    fn new_storage_rows_have_canonical_typed_id_order_without_reordering_existing_rows() {
        let mut world = WorldState::default();
        let first = storage(json!("z-existing"));
        let second = storage(json!("a-existing"));
        world.production_data.storage_reservations = vec![first.clone(), second.clone()];
        let mut changes = OperationSession::new(&world).unwrap().finish().unwrap();
        let mut updated = first.clone();
        updated.released_quantity = Some(1.0);
        let number = storage(json!(7));
        let text = storage(json!("7"));
        let last = storage(json!("z-new"));
        changes.storage = vec![
            (last.id.clone(), None, last.clone()),
            (number.id.clone(), None, number.clone()),
            (first.id.clone(), Some(first), updated.clone()),
            (text.id.clone(), None, text.clone()),
        ];
        apply_prepared(&mut world, changes).unwrap();
        assert_eq!(
            world.production_data.storage_reservations,
            vec![updated, second, text, last, number]
        );
    }

    #[test]
    fn new_stock_rows_follow_assigned_ids_without_renumbering_or_moving_existing_rows() {
        let mut world = WorldState::default();
        let old = StockpileRecord {
            id: 20,
            external_id: ExternalId::String("existing".into()),
            quantity: 3.0,
            ..Default::default()
        };
        world.stockpiles.push(old.clone());
        let mut changes = OperationSession::new(&world).unwrap().finish().unwrap();
        let mut updated = old.clone();
        updated.quantity = 2.0;
        let first = StockpileRecord {
            id: 21,
            external_id: ExternalId::String("z-first".into()),
            quantity: 1.1,
            ..Default::default()
        };
        let second = StockpileRecord {
            id: 22,
            external_id: ExternalId::String("a-second".into()),
            quantity: 0.4,
            ..Default::default()
        };
        changes.stocks = vec![
            (second.external_id.clone(), None, second.clone()),
            (old.external_id.clone(), Some(old), updated.clone()),
            (first.external_id.clone(), None, first.clone()),
        ];
        apply_prepared(&mut world, changes).unwrap();
        assert_eq!(world.stockpiles, vec![updated, first, second]);
    }

    #[test]
    fn new_obligations_have_canonical_typed_id_order_without_moving_existing_rows() {
        fn debt(id: Value) -> Obligation {
            serde_json::from_value(json!({"obligation_id": id, "payer_id": "payer",
                "payee_id": "payee", "currency_id": 7, "purpose": "production",
                "amount_minor": "123", "author_note": "retain creditor rights"}))
            .unwrap()
        }
        let mut world = WorldState::default();
        let old = debt(json!("z-existing"));
        world.production_data.obligations.push(old.clone());
        let mut changes = OperationSession::new(&world).unwrap().finish().unwrap();
        let number = debt(json!(7));
        let text = debt(json!("7"));
        changes.debts = vec![
            (number.obligation_id.clone(), None, number.clone()),
            (text.obligation_id.clone(), None, text.clone()),
        ];
        apply_prepared(&mut world, changes).unwrap();
        assert_eq!(world.production_data.obligations, vec![old, text, number]);
    }
}

#[cfg(test)]
#[path = "../tests/internal/perf_preconditions.rs"]
mod perf_preconditions_tests;
