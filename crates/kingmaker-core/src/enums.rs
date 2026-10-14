use crate::identity::ExternalId;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Every authored commodity has its own identity, including unported ones.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum Goods {
    #[default]
    Grain,
    Flour,
    Fish,
    Meat,
    Timber,
    Iron,
    Coal,
    Cloth,
    Steel,
    MilitarySupply,
    LuxuryGoods,
    Custom(ExternalId),
}
impl Goods {
    pub fn from_external(id: ExternalId) -> Self {
        match id.as_str() {
            Some("grain") => Self::Grain,
            Some("flour") => Self::Flour,
            Some("fish") => Self::Fish,
            Some("meat") => Self::Meat,
            Some("wood") => Self::Timber,
            Some("iron") => Self::Iron,
            Some("coal") => Self::Coal,
            Some("textiles") => Self::Cloth,
            Some("steel") => Self::Steel,
            Some("military_supply") => Self::MilitarySupply,
            Some("luxury_goods") => Self::LuxuryGoods,
            _ => Self::Custom(id),
        }
    }
    pub fn external_id(&self) -> ExternalId {
        ExternalId::from(match self {
            Self::Grain => "grain",
            Self::Flour => "flour",
            Self::Fish => "fish",
            Self::Meat => "meat",
            Self::Timber => "wood",
            Self::Iron => "iron",
            Self::Coal => "coal",
            Self::Cloth => "textiles",
            Self::Steel => "steel",
            Self::MilitarySupply => "military_supply",
            Self::LuxuryGoods => "luxury_goods",
            Self::Custom(id) => return id.clone(),
        })
    }
}
impl Serialize for Goods {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.external_id().serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for Goods {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self::from_external(ExternalId::deserialize(deserializer)?))
    }
}
