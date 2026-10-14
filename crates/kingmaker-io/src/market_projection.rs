use crate::codec::CodecContext;
use crate::production_projection::rows;
use anyhow::{bail, Context, Result};
use kingmaker_core::{market::*, production::optional_value, SourceSnapshot};
use serde::de::DeserializeOwned;
use serde_json::Value;

pub(crate) fn singleton<T: DeserializeOwned>(
    source: &SourceSnapshot,
    codec: &CodecContext,
    table: &str,
) -> Result<Option<T>> {
    let mut values: Vec<Value> = rows(source, codec, table)?;
    if values.len() > 1 {
        bail!("{table}: multiple authoritative singleton rows");
    }
    values
        .pop()
        .map(|value| {
            optional_value(value).with_context(|| format!("{table}: authored market terms"))
        })
        .transpose()
        .map(Option::flatten)
}

pub fn project_market(source: &SourceSnapshot, codec: &CodecContext) -> Result<MarketData> {
    Ok(MarketData {
        phase_receipts: vec![],
        needs: singleton(source, codec, "registry/needs")?,
        price_calibration: singleton(source, codec, "registry/priceCalibration")?,
        parameters: singleton(source, codec, "registry/foundationParameters")?,
        territory: rows(source, codec, "registry/territory")?,
        maintenance_agreements: rows(source, codec, "registry/maintenanceAgreements")?,
        sale_offers: rows(source, codec, "registry/saleOffers")?,
        institutions: rows(source, codec, "registry/institutions")?,
        construction_catalog: rows(source, codec, "registry/constructionCatalog")?,
        consumer_facility_by_province: singleton(
            source,
            codec,
            "registry/consumerFacilityByProvince",
        )?,
    })
}
