//! Composite IDs preserve ordinary strings and keep typed components distinct.
use kingmaker_core::ExternalId;
use serde_json::{json, Value};
pub(crate) fn component_value(id: &Value) -> String {
    match id {
        Value::Number(_) => json!(["number", id]).to_string(),
        Value::String(s) if s.starts_with('[') => json!(["string", id]).to_string(),
        _ => crate::needs::js_string(id),
    }
}
pub(crate) fn component(id: &ExternalId) -> String {
    component_value(&id.json())
}
pub(crate) fn service_offer_id(job: &ExternalId) -> ExternalId {
    format!("service:{}", component(job)).into()
}
