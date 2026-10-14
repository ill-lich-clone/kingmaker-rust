//! Typed read projections. Unmodeled author data remains in SourceSnapshot.
use crate::codec::{CodecContext, CODEC_TAG};
use anyhow::{anyhow, bail, Context, Result};
use kingmaker_core::*;
use serde_json::Value;
use std::collections::HashMap;

pub fn project(
    source: &SourceSnapshot,
    codec: &CodecContext,
    campaign: CampaignHeader,
) -> Result<WorldState> {
    let mut world = WorldState {
        campaign,
        ..WorldState::default()
    };
    world.market_data = crate::market_projection::project_market(source, codec)?;
    world.construction_data = crate::construction_projection::project_construction(source, codec)?;
    world.income_data = crate::income_projection::project_income(source, codec)?;
    world.transport_data = crate::income_projection::project_transport(source, codec)?;
    world.budget_data = crate::budget_projection::project_budget(source, codec)?;
    world.labor_data = crate::labor_projection::project_labor(source, codec)?;
    world.mobility_data = crate::mobility_projection::project_mobility(source, codec)?;
    for row in source.records("accounts") {
        let row = row?;
        let row = row.as_ref();
        project_row(row, || {
            let (raw, v) = decode(codec, row)?;
            let id = own_ordinal(&raw, "account_id")?;
            world.accounts.push(AccountBalance {
                id: AccountId(id),
                external_id: id_field(&v, "account_id")?,
                owner_id: id_field(&v, "owner_id")?,
                currency_id: id_field(&v, "currency_id")?,
                purpose: text(&v, "purpose")?,
                balance: money(required(&v, "amount_minor")?)?,
            });
            Ok(())
        })?;
    }
    world.rebuild_account_index().map_err(|e| anyhow!(e))?;
    let mut owner_accounts: HashMap<ExternalId, Vec<AccountId>> = HashMap::new();
    for a in &world.accounts {
        owner_accounts
            .entry(a.owner_id.clone())
            .or_default()
            .push(a.id);
    }
    let account_positions: HashMap<_, _> = world
        .accounts
        .iter()
        .enumerate()
        .map(|(i, a)| (a.id, i))
        .collect();
    for row in source.records("registry/territory") {
        let row = row?;
        let row = row.as_ref();
        project_row(row, || {
            let (_, v) = decode(codec, row)?;
            let id =
                ProvinceId::from_external(&id_field(&v, "province_id")?).map_err(|e| anyhow!(e))?;
            world.provinces.push(ProvinceRecord {
                id,
                name: optional_text(&v, "name")?.unwrap_or_default(),
                owner_state_id: optional_id(&v, "owner_state_id")?,
                controller_state_id: optional_id(&v, "controller_state_id")?,
                goods_id: optional_id(&v, "goods_id")?.map(Goods::from_external),
                installed_capacity: optional_number(&v, "installed_capacity")?,
                pop_range: (0, 0),
            });
            Ok(())
        })?;
    }
    let mut maintenance: HashMap<ExternalId, Vec<ExternalId>> = HashMap::new();
    for row in source.records("registry/maintenanceAgreements") {
        let row = row?;
        let row = row.as_ref();
        project_row(row, || {
            let (_, v) = decode(codec, row)?;
            maintenance
                .entry(id_field(&v, "pop_id")?)
                .or_default()
                .push(id_field(&v, "payer_id")?);
            Ok(())
        })?;
    }
    for row in source.records("registry/pops") {
        let row = row?;
        let row = row.as_ref();
        project_row(row, || {
            let (raw, v) = decode(codec, row)?;
            let external_id = id_field(&v, "pop_id")?;
            let currency_id = optional_id(&v, "currency_id")?;
            let account_id = currency_id
                .as_ref()
                .and_then(|currency| {
                    world.account_by_address(&AccountAddress {
                        owner_id: external_id.clone(),
                        currency_id: currency.clone(),
                        purpose: "personal_savings".into(),
                    })
                })
                .map(|a| a.id);
            let sponsored = maintenance.get(&external_id);
            let maintenance_payer_id = sponsored
                .filter(|rows| rows.len() == 1)
                .map(|rows| rows[0].clone());
            let pop_type_id = id_field(&v, "pop_type_id")?;
            let culture_id = optional_id(&v, "culture_id")?;
            let religion_id = optional_id(&v, "religion_id")?;
            let mut extra = v
                .as_object()
                .ok_or_else(|| anyhow!("POP record must be an object"))?
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect::<kingmaker_core::production::Extra>();
            for key in [
                "pop_id",
                "state_id",
                "province_id",
                "pop_type_id",
                "culture_id",
                "religion_id",
                "size",
                "workforce",
                "employed",
                "unemployed",
                "currency_id",
                "literacy",
                "militancy",
                "consciousness",
            ] {
                extra.remove(key);
            }
            world.pops.push(PopRecord {
                extra,
                state_id: optional_id(&v, "state_id")?,
                id: PopId(own_ordinal(&raw, "pop_id")?),
                external_id,
                province_id: ProvinceId::from_external(&id_field(&v, "province_id")?)
                    .map_err(|e| anyhow!(e))?,
                pop_type: display_id(&pop_type_id),
                pop_type_id,
                size: u32_value(required(&v, "size")?)?,
                workforce: number(&v, "workforce")?,
                employed: number(&v, "employed")?,
                unemployed: number(&v, "unemployed")?,
                currency_id,
                maintenance_payer_id,
                account_status: if account_id.is_some() {
                    "resolved"
                } else if sponsored.is_some() {
                    "sponsored_no_personal_account"
                } else {
                    "absent_in_source"
                }
                .into(),
                culture: culture_id.as_ref().map(display_id).unwrap_or_default(),
                religion: religion_id.as_ref().map(display_id).unwrap_or_default(),
                culture_id,
                religion_id,
                account_id,
                literacy: optional_number(&v, "literacy")?,
                militancy: optional_number(&v, "militancy")?,
                consciousness: optional_number(&v, "consciousness")?,
            });
            Ok(())
        })?;
    }
    // Derived view order, not an economic scheduling rule. Original row order is retained in source.
    world.pops.sort_by_key(|p| (p.province_id.0, p.id.0));
    let mut ranges = HashMap::new();
    for (i, p) in world.pops.iter().enumerate() {
        let i = u32::try_from(i).context("POP index exceeds u32")?;
        ranges
            .entry(p.province_id)
            .and_modify(|range: &mut (u32, u32)| range.1 = i + 1)
            .or_insert((i, i + 1));
    }
    for p in &mut world.provinces {
        p.pop_range = ranges.get(&p.id).copied().unwrap_or((0, 0));
    }
    for (table, is_artisan) in [
        ("registry/artisanEnterprises", true),
        ("registry/factoryEnterprises", false),
    ] {
        for row in source.records(table) {
            let row = row?;
            let row = row.as_ref();
            project_row(row, || {
                let (raw, v) = decode(codec, row)?;
                let owner_id = id_field(&v, "owner_id")?;
                let account_ids = owner_accounts.get(&owner_id).cloned().unwrap_or_default();
                let payment_purpose = optional_text(&v, "payment_purpose")?;
                let eligible: Vec<_> = account_ids
                    .iter()
                    .copied()
                    .filter(|id| {
                        account_positions
                            .get(id)
                            .map(|&i| &world.accounts[i])
                            .is_some_and(|a| {
                                payment_purpose.as_ref().is_none_or(|p| &a.purpose == p)
                            })
                    })
                    .collect();
                let account_id = if eligible.len() == 1 {
                    Some(eligible[0])
                } else {
                    None
                };
                let employees = if let Some(value) = optional_number(&v, "staff_count")? {
                    Some(value)
                } else if let Some(assignments) = get(&v, "assignments") {
                    let rows = assignments
                        .as_array()
                        .context("Assignments must be an array")?;
                    let mut count = 0.0;
                    for assignment in rows {
                        count += number(assignment, "workers_count")?
                            * number(assignment, "time_fraction")?;
                    }
                    Some(count)
                } else {
                    None
                };
                let operating_capital = get(&v, "working_capital")
                    .and_then(|w| get(w, "minimum_minor"))
                    .map(|value| money(value).map(|m| m.units()))
                    .transpose()?;
                world.enterprises.push(EnterpriseRecord {
                    id: EnterpriseId(own_ordinal(&raw, "id")?),
                    external_id: id_field(&v, "id")?,
                    owner_id,
                    province_id: ProvinceId::from_external(&id_field(&v, "province_id")?)
                        .map_err(|e| anyhow!(e))?,
                    recipe_id: optional_id(&v, "method_id")?,
                    level: match get(&v, "level") {
                        Some(x) => u32_value(x)?,
                        None => 1,
                    },
                    account_id,
                    account_ids,
                    payment_purpose,
                    employees,
                    is_artisan,
                    operating_capital,
                });
                Ok(())
            })?;
        }
    }
    for row in source.records("stocks") {
        let row = row?;
        let row = row.as_ref();
        project_row(row, || {
            let (raw, v) = decode(codec, row)?;
            let mut extra = v
                .as_object()
                .ok_or_else(|| anyhow!("Stock must be an object"))?
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect::<kingmaker_core::production::Extra>();
            for key in [
                "opening_stock_id",
                "inventory_origin",
                "method_kind",
                "province_id",
                "goods_id",
                "amount",
                "inventory_purpose",
                "owner_id",
                "unit",
                "facility_id",
                "currency_id",
                "beneficiary_pop_id",
            ] {
                extra.remove(key);
            }
            world.stockpiles.push(StockpileRecord {
                extra,
                inventory_origin: optional_text(&v, "inventory_origin")?,
                method_kind: optional_text(&v, "method_kind")?,
                id: own_ordinal(&raw, "opening_stock_id")?,
                external_id: id_field(&v, "opening_stock_id")?,
                province_id: ProvinceId::from_external(&id_field(&v, "province_id")?)
                    .map_err(|e| anyhow!(e))?,
                goods: Goods::from_external(id_field(&v, "goods_id")?),
                quantity: number(&v, "amount")?,
                purpose: optional_text(&v, "inventory_purpose")?
                    .unwrap_or_else(|| "commercial".into()),
                owner_id: id_field(&v, "owner_id")?,
                unit: get(&v, "unit").cloned(),
                facility_id: optional_id(&v, "facility_id")?,
                currency_id: optional_id(&v, "currency_id")?,
                beneficiary_pop_id: optional_id(&v, "beneficiary_pop_id")?,
            });
            Ok(())
        })?;
    }
    Ok(world)
}
fn project_row<T>(row: &StoredRecord, operation: impl FnOnce() -> Result<T>) -> Result<T> {
    operation().with_context(|| format!("{} {}", row.table_name, row.id_key))
}
fn decode(codec: &CodecContext, row: &StoredRecord) -> Result<(Value, Value)> {
    let raw: Value = serde_json::from_str(&row.payload)
        .with_context(|| format!("{} {}: invalid JSON", row.table_name, row.id_key))?;
    let value = codec
        .decode_record(&raw)
        .with_context(|| format!("{} {}", row.table_name, row.id_key))?;
    Ok((raw, value))
}
fn own_ordinal(raw: &Value, field: &str) -> Result<u32> {
    let intrinsic = raw
        .get("value")
        .and_then(|v| v.get(field))
        .or_else(|| raw.get("extensions").and_then(|v| v.get(field)))
        .filter(|v| v[CODEC_TAG] == "id")
        .and_then(|v| v["value"].as_u64());
    let metadata = raw.get("id").and_then(Value::as_u64);
    if metadata.zip(intrinsic).is_some_and(|(a, b)| a != b) {
        bail!("Own dictionary ordinal disagrees with {field}");
    }
    let n = metadata
        .or(intrinsic)
        .context("Missing own dictionary ordinal")?;
    u32::try_from(n).context("Own dictionary ordinal exceeds u32")
}
fn get<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    v.get(key)
        .filter(|x| !x.is_null() && x.get(CODEC_TAG).and_then(Value::as_str) != Some("undefined"))
}
fn required<'a>(v: &'a Value, key: &str) -> Result<&'a Value> {
    get(v, key).with_context(|| format!("Missing required field {key}"))
}
fn id_field(v: &Value, key: &str) -> Result<ExternalId> {
    ExternalId::from_json(required(v, key)?.clone()).map_err(|e| anyhow!("{key}: {e}"))
}
fn optional_id(v: &Value, key: &str) -> Result<Option<ExternalId>> {
    get(v, key)
        .map(|x| ExternalId::from_json(x.clone()).map_err(|e| anyhow!("{key}: {e}")))
        .transpose()
}
fn text(v: &Value, key: &str) -> Result<String> {
    required(v, key)?
        .as_str()
        .map(str::to_string)
        .with_context(|| format!("{key} must be a string"))
}
fn optional_text(v: &Value, key: &str) -> Result<Option<String>> {
    get(v, key)
        .map(|x| {
            x.as_str()
                .map(str::to_string)
                .with_context(|| format!("{key} must be a string"))
        })
        .transpose()
}
fn number(v: &Value, key: &str) -> Result<f64> {
    optional_number(v, key)?.with_context(|| format!("Missing numeric field {key}"))
}
fn optional_number(v: &Value, key: &str) -> Result<Option<f64>> {
    get(v, key)
        .map(|x| {
            x.as_f64()
                .filter(|n| n.is_finite())
                .with_context(|| format!("{key} must be finite"))
        })
        .transpose()
}
fn u32_value(v: &Value) -> Result<u32> {
    u32::try_from(v.as_u64().context("Expected nonnegative integer")?)
        .context("Integer exceeds u32")
}
fn display_id(id: &ExternalId) -> String {
    id.as_str()
        .map(str::to_string)
        .unwrap_or_else(|| id.json().to_string())
}
pub fn money(value: &Value) -> Result<Money> {
    let text = if value.get(CODEC_TAG).and_then(Value::as_str) == Some("bigint") {
        value["value"]
            .as_str()
            .context("Invalid bigint amount")?
            .to_string()
    } else if let Some(s) = value.as_str() {
        s.to_string()
    } else if let Some(n) = value.as_i64() {
        n.to_string()
    } else {
        bail!("Money must be an exact integer or decimal integer text")
    };
    Ok(Money::from_units(text.parse::<i64>().with_context(
        || format!("Money out of i64 range or invalid: {text}"),
    )?))
}
