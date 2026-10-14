use crate::codec::CodecContext;
use anyhow::{bail, Context, Result};
use kingmaker_core::{
    CampaignHeader, ExternalId, ProvinceId, SourceSnapshot, StoredRecord, WorldState,
};
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use std::{collections::HashMap, path::Path, sync::Arc};

pub struct SqliteLoader;
impl SqliteLoader {
    pub fn load_from_path<P: AsRef<Path>>(path: P, campaign_id: &str) -> Result<WorldState> {
kingmaker_core::diagnostics::measure("io.load", kingmaker_core::diagnostics::Visibility::Boundary, || {
        let mut conn = Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .context("Failed to open source SQLite read-only")?;
        // All tables belong to the same read snapshot, even if another process publishes later.
        let tx = conn.transaction()?;
        let campaign_key = serde_json::to_string(&["string", campaign_id])?;
        let (revision, payload, static_payload): (u64, String, String) = tx
            .query_row(
                "SELECT revision,payload,static_payload FROM km_campaign WHERE id=?1",
                [&campaign_key],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .with_context(|| format!("Campaign {campaign_id} was not found or cannot be read"))?;
        let mut backing = crate::source_store::SourceStoreBuilder::temporary()?;
        let mut dictionary = HashMap::new();
        {
            let mut stmt = tx.prepare("SELECT n,value FROM km_dictionary ORDER BY n")?;
            let mut rows = stmt.query([])?;
            while let Some(r) = rows.next()? {
                let n: u64 = r.get(0)?;
                let text: String = r.get(1)?;
                let value: Value =
                    serde_json::from_str(&text).with_context(|| format!("Dictionary {n}"))?;
                if !value.is_number() && !value.is_string() {
                    bail!("Dictionary {n}: external ID must be string or number");
                }
                backing.push_dictionary(n,&value)?;
                dictionary.insert(n, value);
            }
        }
        let mut shared = HashMap::new();
        {
            let mut stmt = tx.prepare("SELECT kind,n,payload FROM km_shared ORDER BY kind,n")?;
            let mut rows = stmt.query([])?;
            while let Some(r) = rows.next()? {
                let kind: String = r.get(0)?;
                let n: u64 = r.get(1)?;
                let text: String = r.get(2)?;
                let value:Value=serde_json::from_str(&text).with_context(||format!("Shared {kind}:{n}"))?;
                backing.push_shared(&kind,n,&value)?;
                shared.insert((kind,n),value);
            }
        }
        let codec = CodecContext::new(dictionary, shared);
        let static_raw: Value =
            serde_json::from_str(&static_payload).context("@static envelope JSON")?;
        if static_raw["table"] != "@static" {
            bail!("@static envelope table mismatch");
        }
        codec.validate_references(&static_raw).context("@static")?;
        let header_raw: Value = serde_json::from_str(&payload).context("Campaign envelope JSON")?;
        if header_raw["table"] != "@campaign" {
            bail!("Campaign envelope table mismatch");
        }
        let header = codec.decode_record(&header_raw)?;
        if header["campaign_id"].as_str() != Some(campaign_id) {
            bail!("Campaign external ID mismatch");
        }
        if header["revision"].as_u64() != Some(revision) {
            bail!("Campaign SQL/envelope revision mismatch");
        }
        let year = i32::try_from(
            header["date"]["year"]
                .as_i64()
                .context("Missing campaign year")?,
        )?;
        let month = u8::try_from(
            header["date"]["month"]
                .as_u64()
                .context("Missing campaign month")?,
        )?;
        let day = u8::try_from(
            header["date"]["day"]
                .as_u64()
                .context("Missing campaign day")?,
        )?;
        if !(1..=12).contains(&month) || !(1..=30).contains(&day) {
            bail!("Invalid campaign 30-day calendar date");
        }
        let campaign = CampaignHeader {
            campaign_id: campaign_id.into(),
            revision,
            year,
            month,
            day,
            engine_hash: optional_hash(&header, "engine_hash")?,
            rules_hash: optional_hash(&header, "rules_hash")?,
            source_hash: optional_hash(&header, "source_hash")?,
        };
        let mut source = SourceSnapshot {
            campaign_id: campaign.campaign_id.clone(),
            revision,
            database_path: path.as_ref().to_string_lossy().into_owned(),
            campaign_payload: payload,
            static_payload,
            dictionary: codec.dictionary(),
            shared: codec.shared(),
            ..SourceSnapshot::default()
        };
        {
            let mut stmt=tx.prepare("SELECT table_name,id_key,external_id,ordinal,sequence,payload FROM km_entity WHERE campaign=?1 ORDER BY table_name,ordinal,sequence")?;
            let mut rows = stmt.query([&campaign_key])?;
            let mut count = 0;
            while let Some(r) = rows.next()? {
                let row = StoredRecord {
                    table_name: r.get(0)?,
                    id_key: r.get(1)?,
                    external_id_json: r.get(2)?,
                    ordinal: r.get(3)?,
                    sequence: r.get(4)?,
                    payload: r.get(5)?,
                };
                let envelope: Value = serde_json::from_str(&row.payload).with_context(|| {
                    format!("{} {}: invalid envelope JSON", row.table_name, row.id_key)
                })?;
                if envelope["storage_schema_version"].as_u64() != Some(2)
                    || envelope["table"].as_str() != Some(&row.table_name)
                {
                    bail!("{} {}: schema/table mismatch", row.table_name, row.id_key);
                }
                // Check references without building a second decoded copy of each unported table.
                codec
                    .validate_references(&envelope)
                    .with_context(|| format!("{} {}", row.table_name, row.id_key))?;
                validate_row_identity(&codec, &row, &envelope)
                    .with_context(|| format!("{} {}", row.table_name, row.id_key))?;
                backing.push(&row)?;
                count += 1;
                if count % 100000 == 0 {
                    eprintln!("Загрузка исходного мира: {count} записей прочитано");
                }
            }
        }
        {
            let mut stmt = tx.prepare(
                "SELECT table_name,max_ordinal,next_sequence FROM km_table_meta WHERE campaign=?1",
            )?;
            let mut rows = stmt.query([&campaign_key])?;
            while let Some(r) = rows.next()? {
                source.table_meta.insert(r.get(0)?, (r.get(1)?, r.get(2)?));
            }
        }
        source.backing = Some(Arc::new(backing.finish(crate::source_store::SOURCE_CACHE_BYTES)?));
        let mut world = crate::projection::project(&source, &codec, campaign)?;
        world.production_data = crate::production_projection::project_production(&source, &codec)?;
        let validation = world.validate_links();
        if !validation.errors.is_empty() {
            bail!(
                "Invalid loaded source graph ({}): {}",
                validation.errors.len(),
                validation
                    .errors
                    .iter()
                    .take(10)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("; ")
            );
        }
        source.dictionary=Arc::new(HashMap::new());
        source.shared=Arc::new(HashMap::new());
        world.source = Some(Arc::new(source));
        tx.commit()?;
        Ok(world)
    
})
}
}
fn validate_row_identity(codec: &CodecContext, row: &StoredRecord, raw: &Value) -> Result<()> {
    let sql: Value = serde_json::from_str(
        row.external_id_json
            .as_deref()
            .context("Missing SQL external_id")?,
    )
    .context("Invalid SQL external_id JSON")?;
    ExternalId::from_json(sql.clone()).map_err(anyhow::Error::msg)?;
    if let Some(n) = raw.get("id") {
        if !same_row_identity(
            &row.table_name,
            codec.resolve(n.as_u64().context("Invalid envelope own ordinal")?)?.as_ref(),
            &sql,
        )? {
            bail!("SQL external_id disagrees with envelope dictionary ID");
        }
    }
    if let Some(id) = raw.get("external_id") {
        if !same_row_identity(&row.table_name, id, &sql)? {
            bail!("SQL external_id disagrees with envelope external_id");
        }
    }
    // Mirror codec recordExternalId, plus the explicit identities used by typed views.
    // Identity-less/global rows keep their store-assigned synthetic address.
    let component = row.table_name.strip_prefix("registry/");
    let component_field = component.map(|s| format!("{s}_id"));
    let specific = match row.table_name.as_str() {
        "accounts" => Some("account_id"),
        "stocks" => Some("opening_stock_id"),
        "reservations" => Some("reservation_id"),
        "obligations" => Some("obligation_id"),
        "registry/pops" => Some("pop_id"),
        "registry/territory" => Some("province_id"),
        _ => None,
    };
    for field in [Some("id"), component_field.as_deref(), specific]
        .into_iter()
        .flatten()
    {
        let value = raw
            .get("value")
            .and_then(|v| v.get(field))
            .or_else(|| raw.get("extensions").and_then(|v| v.get(field)));
        let Some(value) =
            value.filter(|v| !v.is_null() && v[crate::codec::CODEC_TAG] != "undefined")
        else {
            continue;
        };
        let decoded = codec.decode_record(
            &serde_json::json!({"storage_schema_version":2,"table":"@identity",
            "value":value,"extensions":{},"bigint_fields":[]}),
        )?;
        ExternalId::from_json(decoded.clone()).map_err(anyhow::Error::msg)?;
        if !same_row_identity(&row.table_name, &decoded, &sql)? {
            bail!("SQL external_id disagrees with intrinsic {field}");
        }
        break;
    }
    Ok(())
}
fn same_row_identity(table: &str, left: &Value, right: &Value) -> Result<bool> {
    if table == "registry/territory" {
        // The existing map registry addresses provinces with decimal string keys;
        // their authored province_id is numeric. This is the one approved normalization.
        let province = |v: &Value| -> Result<ProvinceId> {
            let id = ExternalId::from_json(v.clone()).map_err(anyhow::Error::msg)?;
            ProvinceId::from_external(&id).map_err(anyhow::Error::msg)
        };
        Ok(province(left)? == province(right)?)
    } else {
        Ok(left == right)
    }
}
fn optional_hash(v: &Value, key: &str) -> Result<Option<String>> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        _ => bail!("Invalid campaign {key}"),
    }
}
