use crate::codec::CodecContext;
use anyhow::{anyhow, bail, Context, Result};
use kingmaker_core::operations::CommittedOperation;
use kingmaker_core::production::*;
use kingmaker_core::{ExternalId, SourceSnapshot};
use serde::de::DeserializeOwned;
use serde_json::Value;

pub(crate) fn rows<T: DeserializeOwned>(
    source: &SourceSnapshot,
    codec: &CodecContext,
    table: &str,
) -> Result<Vec<T>> {
    eprintln!("Проекция исходного мира: {table}");
    source
        .records(table)
        .map(|row| {
            let row = row?;
            let parse = || -> Result<T> {
                let raw: Value = serde_json::from_str(&row.payload)?;
                Ok(serde_json::from_value(codec.decode_record(&raw)?)?)
            };
            parse().with_context(|| format!("{table} {}: authored terms", row.id_key))
        })
        .collect()
}
pub fn project_production(source: &SourceSnapshot, codec: &CodecContext) -> Result<ProductionData> {
kingmaker_core::diagnostics::measure("io.project", kingmaker_core::diagnostics::Visibility::Boundary, || {
    let mut catalog: Vec<ProductionCatalog> = rows(source, codec, "registry/productionCatalog")?;
    if catalog.len() > 1 {
        bail!("registry/productionCatalog: multiple authoritative catalogs");
    }
    let mut policies: Vec<ProductionPolicies> = rows(source, codec, "registry/policies")?;
    if policies.len() > 1 {
        bail!("Multiple authoritative policies");
    }
    Ok(ProductionData {
        policies: policies.pop().unwrap_or_default(),
        catalog: catalog.pop(),
        resource_operations: rows(source, codec, "registry/resourceOperations")?,
        factories: rows(source, codec, "registry/factoryEnterprises")?,
        artisans: rows(source, codec, "registry/artisanEnterprises")?,
        contracts: rows(source, codec, "registry/contracts")?,
        expansions: rows(source, codec, "registry/completedExpansions")?,
        construction_projects: rows(source, codec, "registry/constructionProjects")?,
        completed_construction: rows(source, codec, "registry/completedConstruction")?,
        storage_facilities: rows(source, codec, "registry/storageFacilities")?,
        storage_reservations: rows(source, codec, "registry/storageReservations")?,
        goods_reservations: rows(source, codec, "registry/goodsReservations")?,
        money_reservations: rows(source, codec, "reservations")?,
        obligations: rows(source, codec, "obligations")?,
        ..ProductionData::default()
    })

})
}
pub fn read_committed_operations(
    source: &SourceSnapshot,
    event_ids: &[ExternalId],
) -> Result<Vec<CommittedOperation>> {
    use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
    let mut conn = Connection::open_with_flags(
        &source.database_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let tx = conn.transaction()?;
    let key = serde_json::to_string(&serde_json::json!(["string", source.campaign_id]))?;
    let revision: u64 =
        tx.query_row("SELECT revision FROM km_campaign WHERE id=?", [&key], |r| {
            r.get(0)
        })?;
    if revision != source.revision {
        bail!("Committed events: source campaign revision changed");
    }
    let codec = CodecContext::from_source(source);
    let mut result = Vec::new();
    let mut statement = tx.prepare(
        "SELECT external_id,payload FROM km_event WHERE campaign=?1 AND id_key=?2 AND revision<=?3",
    )?;
    for id in event_ids {
        let kind = if id.json().is_number() {
            "number"
        } else {
            "string"
        };
        let id_key = serde_json::to_string(&serde_json::json!([kind, id]))?;
        let row: Option<(String, String)> = statement
            .query_row(params![key, id_key, source.revision], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        if let Some((external, payload)) = row {
            let parse = || -> Result<CommittedOperation> {
                let ext = ExternalId::from_json(serde_json::from_str(&external)?)
                    .map_err(|e| anyhow!(e))?;
                if &ext != id {
                    bail!("SQL event identity mismatch");
                }
                let raw: Value = serde_json::from_str(&payload)?;
                if raw["table"] != "@event" {
                    bail!("Event envelope table mismatch");
                }
                codec.validate_references(&raw)?;
                let value = codec.decode_record(&raw)?;
                if value.get("event_id") != Some(&id.json()) {
                    bail!("Event payload identity mismatch");
                }
                Ok(CommittedOperation {
                    event_id: id.clone(),
                    value,
                })
            };
            result.push(parse().with_context(|| format!("@event {id_key}"))?);
        }
    }
    drop(statement);
    tx.commit()?;
    Ok(result)
}
