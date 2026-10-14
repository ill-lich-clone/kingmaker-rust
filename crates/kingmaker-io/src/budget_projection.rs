use crate::{codec::CodecContext, production_projection::rows};
use anyhow::{bail, Result};
use kingmaker_core::{budget::BudgetData, production::optional_value, SourceSnapshot};
use serde_json::Value;
pub fn project_budget(source: &SourceSnapshot, codec: &CodecContext) -> Result<BudgetData> {
    let mut policies: Vec<Value> = rows(source, codec, "registry/foundationBudgetPolicy")?;
    if policies.len() > 1 {
        bail!("Multiple authoritative foundation budget policies");
    }
    Ok(BudgetData {
        profile_id: codec
            .decode_record(&serde_json::from_str(&source.campaign_payload)?)?
            .get("profile_id")
            .and_then(Value::as_str)
            .map(str::to_owned),
        employment: rows(source, codec, "registry/employment")?,
        jobs: rows(source, codec, "registry/jobs")?,
        adventurer_income_rates: rows(source, codec, "registry/adventurerIncomeRates")?,
        foundation_budget_policy: policies.pop().map(optional_value).transpose()?.flatten(),
        ..Default::default()
    })
}
