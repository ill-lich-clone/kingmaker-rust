use crate::{codec::CodecContext,market_projection::singleton,production_projection::rows};
use anyhow::Result;
use kingmaker_core::{mobility::MobilityData,SourceSnapshot};
pub fn project_mobility(source:&SourceSnapshot,codec:&CodecContext)->Result<MobilityData>{Ok(MobilityData{reference:singleton(source,codec,"registry/reference")?,capitalist_entities:rows(source,codec,"registry/capitalistEntities")?,project_investments:rows(source,codec,"registry/projectInvestments")?,production_beneficiaries:rows(source,codec,"registry/productionBeneficiaries")?,factory_beneficiaries:rows(source,codec,"registry/factoryBeneficiaries")?,shipments:rows(source,codec,"registry/shipments")?,..Default::default()})}
