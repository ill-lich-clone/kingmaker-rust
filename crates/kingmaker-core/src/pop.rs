use crate::ids::{AccountId, PopId, ProvinceId};
use crate::ExternalId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PopRecord {
    /// Namespaced so authored fields cannot shadow native identity/bindings.
    #[serde(default)]
    pub extra: crate::production::Extra,
    pub state_id: Option<ExternalId>,
    pub id: PopId,
    pub external_id: ExternalId,
    pub province_id: ProvinceId,
    pub pop_type: String,
    pub pop_type_id: ExternalId,
    pub culture_id: Option<ExternalId>,
    pub religion_id: Option<ExternalId>,
    pub size: u32,
    pub workforce: f64,
    pub employed: f64,
    pub unemployed: f64,
    pub currency_id: Option<ExternalId>,
    pub maintenance_payer_id: Option<ExternalId>,
    /// No account is different from a fabricated zero-balance account.
    pub account_status: String,
    pub culture: String,
    pub religion: String,
    pub account_id: Option<AccountId>,
    pub literacy: Option<f64>,
    pub militancy: Option<f64>,
    pub consciousness: Option<f64>,
}
