use crate::{codec::CodecContext, market_projection::singleton, production_projection::rows};
use anyhow::Result;
use kingmaker_core::{construction::ConstructionData, SourceSnapshot};
pub fn project_construction(
    source: &SourceSnapshot,
    codec: &CodecContext,
) -> Result<ConstructionData> {
    Ok(ConstructionData {
        investment_opportunities: rows(source, codec, "registry/investmentOpportunities")?,
        construction_catalog: rows(source, codec, "registry/constructionCatalog")?,
        construction_catalog_by_currency: singleton(
            source,
            codec,
            "registry/constructionCatalogByCurrency",
        )?,
        building_site_grants: rows(source, codec, "registry/buildingSiteGrants")?,
        ..Default::default()
    })
}
