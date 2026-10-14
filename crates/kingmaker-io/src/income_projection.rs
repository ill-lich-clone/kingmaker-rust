use crate::{codec::CodecContext, market_projection::singleton, production_projection::rows};
use anyhow::Result;
use kingmaker_core::{income::IncomeData, transport::TransportData, SourceSnapshot};

pub fn project_income(source: &SourceSnapshot, codec: &CodecContext) -> Result<IncomeData> {
    Ok(IncomeData {
        producer_income: rows(source, codec, "registry/producerIncome")?,
        ..Default::default()
    })
}

pub fn project_transport(source: &SourceSnapshot, codec: &CodecContext) -> Result<TransportData> {
    Ok(TransportData {
        transport_assets: rows(source, codec, "registry/transportAssets")?,
        transport_terminals: rows(source, codec, "registry/transportTerminals")?,
        supply_routes: rows(source, codec, "registry/supplyRoutes")?,
        route_reservations: rows(source, codec, "registry/routeReservations")?,
        navigation_geography: rows(source, codec, "registry/navigationGeography")?,
        shipment_recovery_plans: rows(source, codec, "registry/shipmentRecoveryPlans")?,
        automatic_land_paths: singleton(source, codec, "registry/automatic_land_paths")?,
        ..Default::default()
    })
}
