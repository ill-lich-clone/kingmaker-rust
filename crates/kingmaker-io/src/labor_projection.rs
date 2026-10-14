use crate::{codec::CodecContext, market_projection::singleton, production_projection::rows};
use anyhow::{bail, Result};
use kingmaker_core::{labor::LaborData, SourceSnapshot};
use serde_json::Value;
pub fn project_labor(source: &SourceSnapshot, codec: &CodecContext) -> Result<LaborData> {
    let catalogs: Option<Value> = singleton(source, codec, "registry/catalogs")?;
    let species = match catalogs.as_ref().and_then(|v| v.get("species")) {
        None | Some(Value::Null) => vec![],
        Some(Value::Array(rows)) => rows.clone(),
        Some(_) => bail!("Authored species catalog must be an array"),
    };
    Ok(LaborData {
        phase_receipts: vec![],
        institution_rules: singleton(source, codec, "registry/institutionRules")?,
        profiles: rows(source, codec, "registry/profiles")?,
        species,
        pop_mobility: singleton(source, codec, "registry/popMobility")?,
    })
}
