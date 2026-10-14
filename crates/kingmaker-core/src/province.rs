use crate::ids::ProvinceId;
use crate::{ExternalId, Goods};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ProvinceRecord {
    pub id: ProvinceId,
    pub name: String,
    pub owner_state_id: Option<ExternalId>,
    pub controller_state_id: Option<ExternalId>,
    pub goods_id: Option<Goods>,
    pub installed_capacity: Option<f64>,
    /// (start_index, end_index) in the contiguous global Pops array
    pub pop_range: (u32, u32),
}
