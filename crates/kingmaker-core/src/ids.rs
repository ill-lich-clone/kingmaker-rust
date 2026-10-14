use serde::{Deserialize, Serialize};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct ProvinceId(pub u32);

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct PopId(pub u32);

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct AccountId(pub u32);

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct EnterpriseId(pub u32);

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct StateId(pub u16);

impl From<u16> for ProvinceId {
    fn from(v: u16) -> Self {
        ProvinceId(u32::from(v))
    }
}

impl ProvinceId {
    pub fn from_external(id: &crate::identity::ExternalId) -> Result<Self, String> {
        let value = match id {
            crate::identity::ExternalId::String(s) => {
                if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
                    return Err("Province ID must be a map number".into());
                }
                s.parse::<u32>()
                    .map_err(|_| "Province ID out of range".to_string())?
            }
            crate::identity::ExternalId::Number(n) => {
                let value = n
                    .as_u64()
                    .ok_or("Province ID must be a nonnegative integer")?;
                u32::try_from(value).map_err(|_| "Province ID out of range".to_string())?
            }
        };
        Ok(Self(value))
    }
}

impl From<u32> for PopId {
    fn from(v: u32) -> Self {
        PopId(v)
    }
}
