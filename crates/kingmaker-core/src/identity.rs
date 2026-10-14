use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};
use std::{sync::Arc,cell::RefCell,collections::{HashMap,VecDeque}};

#[derive(Default)]
struct TextCache { values:HashMap<Arc<str>,()>, order:VecDeque<(Arc<str>,usize)>, bytes:usize }
thread_local! {static TEXT_CACHE:RefCell<TextCache>=RefCell::new(TextCache::default());}
pub(crate) fn shared_text(text:&str)->Arc<str> {
    TEXT_CACHE.with(|cache| {
        let mut cache=cache.borrow_mut();
        if let Some((value,_))=cache.values.get_key_value(text) {return value.clone();}
        let value:Arc<str>=Arc::from(text);
        let cost=text.len().saturating_add(512);
        const LIMIT:usize=16*1024*1024;
        if cost<=LIMIT {
            while cache.bytes>LIMIT-cost {
                let (old,cost)=cache.order.pop_front().expect("identity cache accounting");
                cache.values.remove(&old);cache.bytes-=cost;
            }
            cache.values.insert(value.clone(),());cache.order.push_back((value.clone(),cost));cache.bytes+=cost;
        }
        value
    })
}
fn serialize_text<S:serde::Serializer>(text:&Arc<str>,serializer:S)->Result<S::Ok,S::Error> {serializer.serialize_str(text)}

/// External names retain their original JSON type. Numeric 7 is not string "7".
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(untagged)]
pub enum ExternalId {
    Number(Number),
    String(#[serde(serialize_with="serialize_text")] Arc<str>),
}
impl<'de> Deserialize<'de> for ExternalId {
    fn deserialize<D:serde::Deserializer<'de>>(deserializer:D)->Result<Self,D::Error> {
        Self::from_json(Value::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}
impl Default for ExternalId {
    fn default() -> Self {
        Self::String(Arc::from(""))
    }
}
impl ExternalId {
    pub fn from_json(value: Value) -> Result<Self, String> {
        match value {
            Value::String(s) => Ok(Self::String(shared_text(&s))),
            Value::Number(n) => Ok(Self::Number(n)),
            _ => Err("External ID must be a string or finite JSON number".into()),
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s),
            _ => None,
        }
    }
    pub fn json(&self) -> Value {
        match self {
            Self::String(s) => Value::String(s.to_string()),
            Self::Number(n) => Value::Number(n.clone()),
        }
    }
}
impl From<&str> for ExternalId {
    fn from(s: &str) -> Self {
        Self::String(shared_text(s))
    }
}
impl From<String> for ExternalId {
    fn from(s: String) -> Self {
        Self::String(shared_text(&s))
    }
}
impl From<u32> for ExternalId {
    fn from(n: u32) -> Self {
        Self::Number(n.into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct AccountAddress {
    pub owner_id: ExternalId,
    pub currency_id: ExternalId,
    pub purpose: String,
}
