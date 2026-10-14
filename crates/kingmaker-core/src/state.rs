use crate::enterprise::{EnterpriseRecord, StockpileRecord};
use crate::identity::{AccountAddress, ExternalId};
use crate::ids::AccountId;
use crate::money::Money;
use crate::pop::PopRecord;
use crate::province::ProvinceRecord;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct CampaignHeader {
    pub campaign_id: String,
    pub revision: u64,
    pub year: i32,
    pub month: u8,
    pub day: u8,
    pub engine_hash: Option<String>,
    pub rules_hash: Option<String>,
    pub source_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct AccountBalance {
    pub id: AccountId,
    pub external_id: ExternalId,
    pub owner_id: ExternalId,
    pub currency_id: ExternalId,
    pub purpose: String,
    pub balance: Money,
}
impl AccountBalance {
    pub fn address(&self) -> AccountAddress {
        AccountAddress {
            owner_id: self.owner_id.clone(),
            currency_id: self.currency_id.clone(),
            purpose: self.purpose.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct StoredRecord {
    pub table_name: String,
    pub id_key: String,
    pub external_id_json: Option<String>,
    pub ordinal: u64,
    pub sequence: u64,
    pub payload: String,
}
#[derive(Debug, Clone, Default)]
pub struct SourceSnapshot {
    pub campaign_id: String,
    pub revision: u64,
    pub database_path: String,
    pub campaign_payload: String,
    pub static_payload: String,
    pub dictionary: Arc<HashMap<u64, Value>>,
    pub shared: Arc<HashMap<(String, u64), Value>>,
    /// In-memory fixtures or empty table declarations; production records live in backing.
    pub tables: BTreeMap<String, Vec<StoredRecord>>,
    pub backing: Option<Arc<dyn crate::source_store::SourceRead>>,
    pub table_meta: BTreeMap<String, (i64, i64)>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct WorldState {
    #[serde(default)]
    pub week_data:crate::week::WeekData,
    #[serde(default)]
    pub construction_data: crate::construction::ConstructionData,
    #[serde(default)]
    pub income_data: crate::income::IncomeData,
    #[serde(default)]
    pub transport_data: crate::transport::TransportData,
    #[serde(default)]
    pub mobility_data: crate::mobility::MobilityData,
    #[serde(default)]
    pub labor_data: crate::labor::LaborData,
    #[serde(default)]
    pub budget_data: crate::budget::BudgetData,
    #[serde(default)]
    pub market_data: crate::market::MarketData,
    #[serde(default)]
    pub production_data: crate::production::ProductionData,
    pub campaign: CampaignHeader,
    pub provinces: Vec<ProvinceRecord>,
    pub pops: Vec<PopRecord>,
    pub enterprises: Vec<EnterpriseRecord>,
    pub stockpiles: Vec<StockpileRecord>,
    pub accounts: Vec<AccountBalance>,
    #[serde(skip)]
    pub account_index: HashMap<AccountAddress, usize>,
    #[serde(skip)]
    pub source: Option<Arc<SourceSnapshot>>,
    #[serde(skip)]
    pub history: Option<crate::history::HistoryBacking>,
}
#[derive(Debug, Clone, Default, Serialize)]
pub struct ValidationReport {
    pub errors: Vec<String>,
    pub not_measured: Vec<String>,
}
impl WorldState {
    pub fn refresh_enterprise_views(&mut self) -> Result<(), String> {
        self.enterprises = self.derive_enterprise_views(
            self.accounts.iter(),
            self.production_data.factories.iter(),
            0,
        )?;
        Ok(())
    }
    pub fn derive_enterprise_views<'a>(
        &'a self,
        accounts: impl Iterator<Item = &'a AccountBalance>,
        factories: impl Iterator<Item = &'a crate::production::FactoryEnterprise>,
        extra_max: u64,
    ) -> Result<Vec<EnterpriseRecord>, String> {
        (self as &dyn crate::world_read::WorldRead).derive_enterprise_views(accounts, factories, extra_max)
    }
    pub fn derive_enterprise_views_with_artisans<'a>(
        &'a self,
        accounts: impl Iterator<Item = &'a AccountBalance>,
        factories: impl Iterator<Item = &'a crate::production::FactoryEnterprise>,
        artisans: impl Iterator<Item = &'a crate::production::ArtisanEnterprise>,
        extra_max: u64,
    ) -> Result<Vec<EnterpriseRecord>, String> {
        (self as &dyn crate::world_read::WorldRead).derive_enterprise_views_with_artisans(accounts, factories, artisans, extra_max)
    }
    pub fn rebuild_account_index(&mut self) -> Result<(), String> {
        self.account_index.clear();
        for (i, a) in self.accounts.iter().enumerate() {
            if self.account_index.insert(a.address(), i).is_some() {
                return Err(format!("Duplicate account address: {:?}", a.address()));
            }
        }
        Ok(())
    }
    pub fn account_by_address(&self, key: &AccountAddress) -> Option<&AccountBalance> {
        self.account_index
            .get(key)
            .and_then(|&i| self.accounts.get(i))
            .filter(|a| a.address() == *key)
    }
    pub fn validate_links(&self) -> ValidationReport {
        (self as &dyn crate::world_read::WorldRead).validate_links()
    }
}
