use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameDate {
    pub year: i32,
    pub month: u8,
    pub day: u8,
}
impl From<&crate::CampaignHeader> for GameDate {
    fn from(c: &crate::CampaignHeader) -> Self {
        Self {
            year: c.year,
            month: c.month,
            day: c.day,
        }
    }
}
