use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CheckStatus {
    Passed,
    Failed { reason: String },
    NotMeasured { reason: String },
}
impl CheckStatus {
    pub fn is_passed(&self) -> bool {
        matches!(self, Self::Passed)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvariantReport {
    pub money: CheckStatus,
    pub goods: CheckStatus,
    pub people: CheckStatus,
    pub labor: CheckStatus,
    pub rights: CheckStatus,
}
impl InvariantReport {
    pub fn all_required_passed(&self) -> bool {
        [
            &self.money,
            &self.goods,
            &self.people,
            &self.labor,
            &self.rights,
        ]
        .iter()
        .all(|s| s.is_passed())
    }
    pub fn production_required_passed(&self) -> bool {
        [&self.money, &self.goods, &self.people, &self.labor]
            .iter()
            .all(|s| s.is_passed())
            && !matches!(self.rights, CheckStatus::Failed { .. })
    }
}
impl Default for InvariantReport {
    fn default() -> Self {
        let status = CheckStatus::NotMeasured {
            reason: "Not executed".into(),
        };
        Self {
            money: status.clone(),
            goods: status.clone(),
            people: status.clone(),
            labor: status.clone(),
            rights: status,
        }
    }
}
