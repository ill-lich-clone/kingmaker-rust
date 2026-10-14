use kingmaker_core::world_read::WorldRead;
use crate::market::OrderPriority;
use kingmaker_core::{Goods, PopRecord, ProvinceId};

use crate::{
    market_policy::MarketScope,
    operation_session::{OpResult, OperationError},
};
use kingmaker_core::{market::*, production::Extra, ExternalId};
use num_bigint::BigInt;
use num_traits::ToPrimitive;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

fn err(s: impl Into<String>) -> OperationError {
    OperationError(s.into())
}
pub(crate) fn undefined(v: &Value) -> bool {
    v.as_object().is_some_and(|o| {
        o.len() == 1 && o.get("__kingmaker_codec_v2__").and_then(Value::as_str) == Some("undefined")
    })
}
fn get<'a>(m: &'a Extra, k: &str) -> Option<&'a Value> {
    m.get(k).filter(|v| !undefined(v))
}
pub(crate) fn js_string(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::String(s) => s.clone(),
        Value::Number(n) => n
            .as_f64()
            .map(|f| f.to_string())
            .unwrap_or_else(|| n.to_string()),
        Value::Array(a) => a
            .iter()
            .map(|v| {
                if v.is_null() || undefined(v) {
                    String::new()
                } else {
                    js_string(v)
                }
            })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) if undefined(v) => "undefined".into(),
        Value::Object(o)
            if o.get("__kingmaker_codec_v2__").and_then(Value::as_str) == Some("bigint") =>
        {
            o.get("value").and_then(Value::as_str).unwrap_or("").into()
        }
        _ => "[object Object]".into(),
    }
}
/// JSON identity quantities use the current JS integer representation within its safe range.
pub(crate) fn js_quantity_value(q: f64) -> Value {
    if q.fract() == 0. && q.abs() <= 9007199254740991. {
        json!(q as i64)
    } else {
        json!(q)
    }
}
pub(crate) fn js_truthy(v: &Value) -> bool {
    if undefined(v) {
        return false;
    }
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|n| n != 0.0 && !n.is_nan()),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}
fn parse_number(s: &str) -> Option<f64> {
    let s = s.trim();
    if s.is_empty() {
        return Some(0.0);
    }
    let radix = if s.starts_with("0x") || s.starts_with("0X") {
        Some(16)
    } else if s.starts_with("0b") || s.starts_with("0B") {
        Some(2)
    } else if s.starts_with("0o") || s.starts_with("0O") {
        Some(8)
    } else {
        None
    };
    let n = match radix {
        Some(r) => BigInt::parse_bytes(s[2..].as_bytes(), r)?.to_f64()?,
        None => s.parse::<f64>().ok()?,
    };
    n.is_finite().then_some(n)
}
pub(crate) fn finite_or_null(v: Option<&Value>) -> Option<f64> {
    let v = v?;
    if v.is_null() || undefined(v) || v.as_str() == Some("") {
        return None;
    }
    if v.is_number() {
        return v.as_f64().filter(|n| n.is_finite());
    }
    parse_number(&js_string(v).replacen(',', ".", 1))
}
fn arithmetic_number(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Null => Some(0.0),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        v => parse_number(&js_string(v)),
    }
}
fn nonnegative(v: Option<&Value>, fallback: f64) -> f64 {
    finite_or_null(v).filter(|n| *n >= 0.0).unwrap_or(fallback)
}
fn truthy<'a>(v: Option<&'a Value>) -> Option<&'a Value> {
    v.filter(|v| js_truthy(v))
}
fn or_null(v: Option<&Value>) -> Value {
    truthy(v).cloned().unwrap_or(Value::Null)
}
fn text(v: Option<&Value>) -> String {
    v.filter(|v| !v.is_null())
        .map(js_string)
        .unwrap_or_default()
        .trim()
        .to_owned()
}
fn numeric(v: Option<f64>) -> Value {
    v.and_then(serde_json::Number::from_f64)
        .map(Value::Number)
        .unwrap_or(Value::Null)
}
fn checked(v: f64) -> OpResult<f64> {
    if v.is_finite() {
        Ok(v)
    } else {
        Err(err("Nonfinite computed need quantity"))
    }
}
fn normalize(v: f64) -> OpResult<Option<f64>> {
    if !v.is_finite() {
        return Ok(None);
    }
    if v.abs() >= 1e21 {
        return Ok(Some(v));
    }
    // JS toFixed rounds the actual binary float, not its shortest decimal spelling.
    let bits = v.abs().to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i32;
    let mantissa = (bits & ((1u64 << 52) - 1)) | if exponent == 0 { 0 } else { 1u64 << 52 };
    let power = if exponent == 0 {
        -1074
    } else {
        exponent - 1023 - 52
    };
    let mut n = BigInt::from(mantissa) * BigInt::from(1_000_000_000_000u64);
    let mut d = BigInt::from(1);
    if power >= 0 {
        n <<= power as usize;
    } else {
        d <<= (-power) as usize;
    }
    let rounded = crate::money_math::round_ratio(&n, &d)?.to_string();
    let padded = format!("{:0>13}", rounded);
    let split = padded.len() - 12;
    let spelling = format!(
        "{}{}.{}",
        if v.is_sign_negative() { "-" } else { "" },
        &padded[..split],
        &padded[split..]
    );
    Ok(Some(
        spelling
            .parse()
            .map_err(|_| err("Need normalization failed"))?,
    ))
}
fn per_person(norm: &Extra, resolver: &Extra, id: &ExternalId) -> OpResult<Option<f64>> {
    let weekly = get(resolver, "period").and_then(Value::as_str) == Some("week");
    let field = if weekly {
        "quantity_per_person_week"
    } else {
        "quantity_per_person_month"
    };
    if weekly && !norm.contains_key(field) {
        return Err(err(format!(
            "Missing native weekly norm {}",
            js_string(&id.json())
        )));
    }
    Ok(finite_or_null(get(norm, field)))
}
fn wealth(reference: &NeedProfileRef) -> f64 {
    match text(get(&reference.extra, "wealth_band_id")).as_str() {
        "poor" => 0.5,
        "lower" => 0.75,
        "middle" => 1.0,
        "upper" => 1.25,
        "rich" => 1.5,
        _ => 1.0,
    }
}
fn luxury_allowed(reference: &NeedProfileRef) -> bool {
    matches!(
        text(get(&reference.extra, "wealth_band_id")).as_str(),
        "upper" | "rich"
    )
}
fn desired(
    norm: &Extra,
    id: &ExternalId,
    good: &Value,
    resolver: &Extra,
    reference: &NeedProfileRef,
) -> OpResult<Option<f64>> {
    let normalizer = get(resolver, "normalizers").and_then(|m| m.get(js_string(good)));
    let h = finite_or_null(
        get(norm, "H")
            .filter(|v| !v.is_null())
            .or_else(|| normalizer.and_then(|n| n.get("H"))),
    );
    let Some(h) = h else { return Ok(None) };
    let size = nonnegative(get(&reference.extra, "size"), 0.0);
    let rule = get(norm, "allocation_rule").and_then(Value::as_str);
    if matches!(
        rule,
        Some("species_clothing_factor" | "none" | "species_food_factor")
    ) {
        return per_person(norm, resolver, id)?
            .map(|q| checked(q * size))
            .transpose();
    }
    let mean = arithmetic_number(normalizer.and_then(|n| n.get("mean_raw")));
    let Some(mean) = mean.filter(|m| *m > 0.0) else {
        return Ok(Some(0.0));
    };
    let factor = if truthy(get(norm, "luxury")).is_some() && !luxury_allowed(reference) {
        0.0
    } else {
        match rule {
            Some("wealth_weight×literacy_factor") => {
                wealth(reference)
                    * (0.5
                        + finite_or_null(get(&reference.extra, "literacy"))
                            .unwrap_or(0.0)
                            .clamp(0.0, 1.0))
            }
            Some("wealth_weight") => wealth(reference),
            _ => 1.0,
        }
    };
    Ok(Some(checked(h * size / 1000.0 * factor / mean)?))
}
fn add_present(out: &mut Extra, key: &str, v: Option<&Value>) {
    if let Some(v) = v {
        out.insert(key.into(), v.clone());
    }
}
fn make_row(id: &ExternalId, q: Option<f64>, gross: Option<f64>, mut out: Extra) -> ResolvedNeed {
    out.remove("need_id");
    out.remove("quantity");
    out.remove("gross_quantity");
    ResolvedNeed {
        need_id: id.clone(),
        quantity: q,
        gross_quantity: gross,
        extra: out,
    }
}
fn unknown(norm: &NeedNorm, period: &str, coverage: &str) -> ResolvedNeed {
    let n = &norm.extra;
    let mut out = Extra::new();
    for k in [
        "goods_id",
        "unit",
        "alternative_group",
        "share_status",
        "functional_group",
        "budget_scope",
    ] {
        out.insert(k.into(), or_null(get(n, k)));
    }
    for k in [
        "norm_quantity",
        "budget_quantity",
        "actual_purchase_quantity",
    ] {
        out.insert(k.into(), Value::Null);
    }
    out.insert("substitute_quantity".into(), json!(0));
    out.insert("military_substitution_quantity".into(), json!(0));
    out.insert(
        "alternatives".into(),
        truthy(get(n, "alternatives")).cloned().unwrap_or(json!([])),
    );
    out.insert(
        "period".into(),
        truthy(get(n, "period")).cloned().unwrap_or(json!(period)),
    );
    out.insert("coverage_status".into(), json!(coverage));
    out.insert(
        "consumable".into(),
        json!(get(n, "consumable") != Some(&Value::Bool(false))),
    );
    out.insert(
        "quantity_status".into(),
        truthy(get(n, "quantity_status"))
            .cloned()
            .unwrap_or(json!("unknown")),
    );
    out.insert(
        "substitution_coefficient".into(),
        get(n, "substitution_coefficient")
            .cloned()
            .unwrap_or(Value::Null),
    );
    out.insert("actual_purchase_status".into(), json!("not_executed_T0"));
    out.insert("provenance".into(), or_null(get(n, "provenance")));
    add_present(&mut out, "status", get(n, "status"));
    add_present(&mut out, "source", get(n, "source"));
    make_row(&norm.need_id, None, None, out)
}
fn material(
    norm: &NeedNorm,
    good: &Value,
    alternative: Option<&Value>,
    resolver: &Extra,
    reference: &NeedProfileRef,
    inputs: &HashMap<ExternalId, f64>,
    period: &str,
) -> OpResult<ResolvedNeed> {
    let n = &norm.extra;
    let mut source = n.clone();
    if let Some(a) = alternative {
        if let Some(a) = a.as_object() {
            source.extend(a.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        source.remove("H");
        if let Some(h) = a_field(alternative, "H") {
            source.insert("H".into(), h.clone());
        }
        if !truthy(source.get("allocation_rule")).is_some() {
            if let Some(rule) = get(n, "allocation_rule") {
                source.insert("allocation_rule".into(), rule.clone());
            }
        }
    }
    let wanted = desired(&source, &norm.need_id, good, resolver, reference)?;
    let good_id = ExternalId::from_json(good.clone()).map_err(err)?;
    let mil = normalize(*inputs.get(&good_id).unwrap_or(&0.0))?.unwrap_or(0.0);
    let gross = wanted.map(normalize).transpose()?.flatten();
    let budget = gross
        .map(|g| normalize((g - mil).max(0.0)))
        .transpose()?
        .flatten();
    let eligible = truthy(get(n, "luxury")).is_none() || luxury_allowed(reference);
    let mut out = Extra::new();
    out.insert("goods_id".into(), good.clone());
    out.insert("norm_quantity".into(), numeric(wanted));
    out.insert("budget_quantity".into(), numeric(budget));
    out.insert("substitute_quantity".into(), json!(0));
    out.insert("military_substitution_quantity".into(), numeric(Some(mil)));
    out.insert(
        "unit".into(),
        truthy(a_field(alternative, "unit"))
            .or_else(|| truthy(get(n, "unit")))
            .cloned()
            .unwrap_or(Value::Null),
    );
    for k in [
        "alternative_group",
        "share_status",
        "functional_group",
        "budget_scope",
        "pricing_scope",
    ] {
        out.insert(k.into(), or_null(get(n, k)));
    }
    out.insert(
        "alternatives".into(),
        if alternative.is_some() {
            Value::Null
        } else {
            truthy(get(n, "alternatives")).cloned().unwrap_or(json!([]))
        },
    );
    out.insert(
        "period".into(),
        truthy(get(n, "period")).cloned().unwrap_or(json!(period)),
    );
    out.insert(
        "coverage_status".into(),
        json!(if eligible {
            "not_evaluated"
        } else {
            "not_applicable"
        }),
    );
    if eligible {
        add_present(&mut out, "status", get(n, "status"));
    } else {
        out.insert("status".into(), json!("not_applicable"));
    }
    out.insert(
        "consumable".into(),
        json!(get(n, "consumable") != Some(&Value::Bool(false))),
    );
    out.insert(
        "quantity_status".into(),
        if eligible {
            truthy(get(n, "quantity_status"))
                .cloned()
                .unwrap_or(json!("desired_norm"))
        } else {
            json!("not_applicable_wealth_band")
        },
    );
    out.insert(
        "substitution_coefficient".into(),
        get(n, "substitution_coefficient")
            .cloned()
            .unwrap_or(Value::Null),
    );
    out.insert("actual_purchase_quantity".into(), json!(0));
    out.insert("actual_purchase_status".into(), json!("not_executed_T0"));
    add_present(&mut out, "source", get(n, "source"));
    out.insert("provenance".into(), or_null(get(n, "provenance")));
    Ok(make_row(&norm.need_id, budget, gross, out))
}
fn a_field<'a>(a: Option<&'a Value>, key: &str) -> Option<&'a Value> {
    a.and_then(|a| a.get(key)).filter(|v| !undefined(v))
}

pub fn resolve_need_quantities(
    needs: &NeedsData,
    reference: &NeedProfileRef,
) -> OpResult<Vec<ResolvedNeed>> {
    let profile = needs
        .profiles
        .iter()
        .find(|p| p.need_profile_id == reference.need_profile_id)
        .ok_or_else(|| err("unknown need profile"))?;
    let resolver = &needs.resolver.extra;
    let weekly = get(resolver, "period").and_then(Value::as_str) == Some("week")
        || needs.schema_version.as_deref() == Some("weekly-needs-v1");
    if weekly
        && (get(resolver, "period").and_then(Value::as_str) != Some("week")
            || get(resolver, "period_days").and_then(Value::as_f64) != Some(7.0))
    {
        return Err(err("Invalid weekly resolver period"));
    }
    let period = if weekly { "week" } else { "month" };
    let days = if weekly {
        7.0
    } else {
        finite_or_null(get(resolver, "month_days"))
            .or_else(|| finite_or_null(get(&profile.extra, "month_days")))
            .unwrap_or(30.0)
    };
    let size = nonnegative(get(&reference.extra, "size"), 0.0);
    let active = nonnegative(get(&reference.extra, "active_army_workers"), 0.0);
    // The source supply unit is 1000 ration-days.
    let military = normalize(
        active * days * nonnegative(get(&profile.extra, "species_food_factor"), 1.0) / 1000.0,
    )?
    .unwrap_or(0.0);
    let mut inputs: HashMap<ExternalId, f64> = HashMap::new();
    if let Some(links) = get(resolver, "military_links").and_then(Value::as_array) {
        for link in links {
            let Some(good) = link.get("input_goods_id").filter(|v| !undefined(v)) else {
                continue;
            };
            let id = ExternalId::from_json(good.clone()).map_err(err)?;
            let q = normalize(
                inputs.get(&id).copied().unwrap_or(0.0)
                    + military * nonnegative(link.get("coefficient"), 0.0),
            )?
            .unwrap_or(0.0);
            inputs.insert(id, q);
        }
    }
    let military_food = ["grain", "flour", "meat", "fish", "produce"]
        .iter()
        .map(|id| inputs.get(&ExternalId::from(*id)).copied().unwrap_or(0.0))
        .sum::<f64>();
    let mut rows = Vec::new();
    let mut material_indexes = Vec::new();
    for norm in &profile.norms {
        crate::run_control::work("needs_norms")?;
        let n = &norm.extra;
        match norm.need_id.as_str() {
            Some("food") => {
                let gross = per_person(n, resolver, &norm.need_id)?
                    .map(|q| checked(q * size))
                    .transpose()?;
                let budget = gross.map(|q| (q - military_food).max(0.0));
                let mut out = Extra::new();
                for k in [
                    "unit",
                    "alternatives",
                    "period",
                    "status",
                    "quantity_status",
                    "source",
                ] {
                    add_present(&mut out, k, get(n, k));
                }
                out.insert("norm_quantity".into(), numeric(gross));
                out.insert("budget_quantity".into(), numeric(budget));
                out.insert("substitute_quantity".into(), json!(0));
                out.insert(
                    "military_substitution_quantity".into(),
                    numeric(Some(military_food)),
                );
                out.insert("alternative_group".into(), json!("food"));
                out.insert("coverage_status".into(), json!("not_evaluated"));
                out.insert("consumable".into(), json!(true));
                out.insert("share_status".into(), Value::Null);
                out.insert("budget_scope".into(), json!("basic"));
                out.insert("actual_purchase_quantity".into(), json!(0));
                out.insert("actual_purchase_status".into(), json!("not_executed_T0"));
                out.insert("provenance".into(), or_null(get(n, "provenance")));
                rows.push(make_row(&norm.need_id, budget, gross, out));
            }
            Some("military") => {}
            Some("wealth_bands" | "budget") => {
                let mut r = unknown(norm, period, "not_applicable");
                r.extra.insert("consumable".into(), json!(false));
                rows.push(r);
            }
            Some(
                "heat"
                | "housing"
                | "construct_maintenance"
                | "health_literacy"
                | "education"
                | "electricity",
            ) => {
                let coverage = if get(n, "status").and_then(Value::as_str) == Some("not_applicable")
                {
                    "not_applicable"
                } else {
                    "not_evaluated"
                };
                let mut r = unknown(norm, period, coverage);
                if norm.need_id.as_str() == Some("heat") {
                    let mut alternatives = get(n, "alternatives")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    for a in &mut alternatives {
                        let h = a.get("H").filter(|v| !v.is_null() && !undefined(v));
                        let q = h
                            .and_then(|h| arithmetic_number(Some(h)))
                            .map(|h| h * size / 1000.0);
                        a.as_object_mut()
                            .ok_or_else(|| err("Invalid heat alternative"))?
                            .insert("norm_quantity".into(), numeric(q));
                    }
                    r.extra.insert("alternatives".into(), json!(alternatives));
                }
                rows.push(r);
            }
            Some("leisure") => {
                if let Some(alts) = get(n, "alternatives").and_then(Value::as_array) {
                    for a in alts {
                        let good = a
                            .get("goods_id")
                            .ok_or_else(|| err("Leisure alternative goods missing"))?;
                        material_indexes.push(rows.len());
                        rows.push(material(
                            norm,
                            good,
                            Some(a),
                            resolver,
                            reference,
                            &inputs,
                            period,
                        )?);
                    }
                }
            }
            _ => {
                if let Some(good) = truthy(get(n, "goods_id")) {
                    material_indexes.push(rows.len());
                    rows.push(material(
                        norm, good, None, resolver, reference, &inputs, period,
                    )?);
                } else {
                    rows.push(unknown(norm, period, "not_evaluated"));
                }
            }
        }
    }
    let military_norm = profile
        .norms
        .iter()
        .find(|n| n.need_id.as_str() == Some("military"));
    if active > 0.0 {
        for good in ["military_supply", "weapons", "ammunition"] {
            let is_supply = good == "military_supply";
            let force =
                finite_or_null(get(resolver, "reference_active_force_per_1000")).unwrap_or(20.0);
            let h = get(resolver, "military_H").and_then(|v| v.get(good));
            let q = if is_supply {
                Some(military)
            } else if force > 0.0 {
                finite_or_null(h).map(|h| active * h / force)
            } else {
                None
            };
            let mut out = Extra::new();
            out.insert("goods_id".into(), json!(good));
            out.insert("norm_quantity".into(), numeric(q));
            out.insert("budget_quantity".into(), numeric(q));
            out.insert("substitute_quantity".into(), json!(0));
            out.insert("military_substitution_quantity".into(), json!(0));
            out.insert(
                "unit".into(),
                if is_supply {
                    json!("1000 нормо-дней довольствия")
                } else {
                    or_null(get(resolver, "military_units").and_then(|v| v.get(good)))
                },
            );
            out.insert("alternative_group".into(), Value::Null);
            out.insert("alternatives".into(), json!([{"goods_id":good,"share":1}]));
            out.insert("period".into(), json!(period));
            out.insert("coverage_status".into(), json!("not_evaluated"));
            out.insert(
                "status".into(),
                military_norm
                    .and_then(|n| truthy(get(&n.extra, "status")))
                    .cloned()
                    .unwrap_or(json!("known")),
            );
            out.insert("consumable".into(), json!(true));
            out.insert(
                "quantity_status".into(),
                json!(if is_supply {
                    "active_force_only"
                } else if force > 0.0 {
                    "scenario_derived_reference_basket"
                } else {
                    "unknown"
                }),
            );
            out.insert("source".into(),if is_supply{military_norm.and_then(|n|truthy(get(&n.extra,"source"))).cloned().unwrap_or(json!("active staffed soldiers/officers × period_days"))}else{json!("active staffed soldiers/officers × authored H / 20 reference active workers per 1000 (scenario-derived)")});
            out.insert(
                "provenance".into(),
                or_null(military_norm.and_then(|n| get(&n.extra, "provenance"))),
            );
            rows.push(make_row(&"military".into(), q, q, out));
        }
    } else if let Some(norm) = military_norm {
        let mut n = norm.clone();
        n.extra.insert(
            "quantity_status".into(),
            json!("not_applicable_no_active_force"),
        );
        let mut r = unknown(&n, period, "not_applicable");
        r.extra.insert("consumable".into(), json!(false));
        rows.push(r);
    }
    for &i in &material_indexes {
        if !matches!(rows[i].need_id.as_str(), Some("clothing" | "furniture")) {
            continue;
        }
        let group = rows[i].extra.get("functional_group");
        let luxury = material_indexes.iter().copied().find(|j| {
            rows[*j].extra.get("functional_group") == group
                && rows[*j].extra.get("budget_scope").and_then(Value::as_str) == Some("luxury")
        });
        let selected = get(&reference.extra, "selected_upgrade_groups")
            .filter(|v| !v.is_null())
            .or_else(|| get(&reference.extra, "selected_upgrades"));
        let selected = match (selected, group) {
            (Some(Value::Array(a)), Some(g)) => a.contains(g),
            (Some(Value::Object(o)), Some(g)) => o.get(&js_string(g)) == Some(&json!(true)),
            _ => false,
        };
        if let Some(j) = luxury.filter(|_| selected) {
            let substitute = rows[j].gross_quantity.unwrap_or(0.0);
            let mil =
                finite_or_null(rows[i].extra.get("military_substitution_quantity")).unwrap_or(0.0);
            let coef = truthy(rows[i].extra.get("substitution_coefficient"))
                .and_then(|v| arithmetic_number(Some(v)))
                .unwrap_or(1.0);
            let q = rows[i]
                .gross_quantity
                .map(|g| (g - mil - substitute * coef).max(0.0));
            rows[i]
                .extra
                .insert("substitute_quantity".into(), numeric(Some(substitute)));
            rows[i].extra.insert("budget_quantity".into(), numeric(q));
            rows[i].quantity = q;
            rows[j]
                .extra
                .insert("selected_for_upgrade".into(), json!(true));
        }
    }
    Ok(rows)
}

pub fn build_need_orders(world: &dyn WorldRead, profile: &MarketProfile) -> OpResult<NeedBuildResult> {
    let multiplier = profile.food_norm_multiplier.unwrap_or(1.0);
    if !multiplier.is_finite() || multiplier < 0.0 {
        return Err(err("Invalid food norm multiplier"));
    }
    let Some(needs) = &world.needs() else {
        return Ok(NeedBuildResult {
            diagnostics: vec![json!({"status":"not_measured","reason":"needs_data_absent"})],
            ..Default::default()
        });
    };
    let market_scope = MarketScope::from_world(world)?;
    let pops: HashMap<_, _> = world.pops().values().map(|p| (&p.external_id, p)).collect();
    let agreements: HashMap<_, _> = world.maintenance_agreements().values()
        .map(|a| (&a.pop_id, a))
        .collect();
    let mut available: HashMap<_, HashSet<ExternalId>> = HashMap::new();
    for s in world.stockpiles().values() {
        crate::run_control::work("needs_stocks")?;
        if s.quantity > 0.0 {
            available
                .entry(market_scope.key(s.province_id))
                .or_default()
                .insert(s.goods.external_id());
        }
    }
    let mut result = NeedBuildResult::default();
    let referenced: HashSet<_> = needs.profile_refs.iter().map(|r| &r.pop_id).collect();
    for pop in world.pops().values() {
        crate::run_control::work("needs_people")?;
        if !referenced.contains(&pop.external_id) {
            result.diagnostics.push(json!({"pop_id":pop.external_id,"status":"not_measured","reason":"need_profile_reference_absent"}));
        }
    }
    for reference in &needs.profile_refs {
        crate::run_control::work("needs_profiles")?;
        let pop = pops
            .get(&reference.pop_id)
            .ok_or_else(|| err("Unknown need POP"))?;
        let agreement = agreements.get(&pop.external_id);
        if let Some(size) = finite_or_null(reference.extra.get("size")) {
            if size != f64::from(pop.size) {
                result.diagnostics.push(json!({"pop_id":pop.external_id,"status":"data_mismatch","reason":"stale_need_profile_reference_size","reference_size":size,"actual_pop_size":pop.size,"quantity_source":"authored_profile_ref"}));
            }
        }
        let rows = resolve_need_quantities(needs, reference)?;
        for mut row in rows {
            let n = &row.extra;
            let food = row.need_id.as_str() == Some("food");
            let known = row.quantity.is_some();
            if !known {
                result.diagnostics.push(json!({"pop_id":pop.external_id,"need_id":row.need_id,"status":"not_measured","reason":"quantity_unknown","coverage_status":n.get("coverage_status")}));
            }
            let should_buy = row
                .quantity
                .is_some_and(|q| q > 0.0 || row.gross_quantity.unwrap_or(0.0) > 0.0)
                && n.get("consumable") != Some(&Value::Bool(false));
            if should_buy {
                let quantity =
                    checked(row.quantity.unwrap() * (if food { multiplier } else { 1.0 }))?;
                let scope = n.get("budget_scope").and_then(Value::as_str);
                let slave = pop.pop_type_id.as_str() == Some("slaves");
                if slave && (agreement.is_none() || scope != Some("basic")) {
                    if agreement.is_none() && scope == Some("basic") {
                        result.diagnostics.push(json!({"pop_id":pop.external_id,"need_id":row.need_id,"status":"blocked","reason":"maintenance_agreement_absent"}));
                    }
                    row.extra.insert("pop_id".into(), pop.external_id.json());
                    result.rows.push(row);
                    continue;
                }
                let (owner, currency, purpose) = if slave {
                    let a = agreement.unwrap();
                    (
                        a.payer_id.clone(),
                        Some(a.currency_id.clone()),
                        "sponsored_maintenance_reserve",
                    )
                } else {
                    (
                        pop.external_id.clone(),
                        pop.currency_id.clone(),
                        "personal_savings",
                    )
                };
                let base = MarketOrder {
                    id: String::new(),
                    owner_id: owner,
                    pop_id: Some(pop.external_id.clone()),
                    province_id: pop.province_id,
                    currency_id: currency,
                    purpose: purpose.into(),
                    goods_id: ExternalId::default(),
                    quantity,
                    priority: match scope {
                        Some("luxury") => "luxury",
                        Some("everyday") => "everyday",
                        _ => "basic",
                    }
                    .into(),
                    consume: true,
                    need_id: Some(row.need_id.clone()),
                    maintenance: slave,
                    facility_id: None,
                    food_alternatives: vec![],
                    food_gross_quantity: None,
                    military_food_substitution: None,
                    extra: Default::default(),
                };
                if food {
                    let alternatives = n
                        .get("alternatives")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    let stocked = available.get(&market_scope.key(pop.province_id));
                    let food: Vec<_> = alternatives
                        .iter()
                        .filter(|a| {
                            a.get("goods_id")
                                .and_then(|g| ExternalId::from_json(g.clone()).ok())
                                .is_some_and(|g| stocked.is_some_and(|s| s.contains(&g)))
                        })
                        .collect();
                    let choices = if food.is_empty() {
                        alternatives.iter().collect::<Vec<_>>()
                    } else {
                        food
                    };
                    let total = choices
                        .iter()
                        .map(|a| arithmetic_number(truthy(a.get("share"))).unwrap_or(0.0))
                        .sum::<f64>();
                    for a in &choices {
                        let mut o = base.clone();
                        let good = a
                            .get("goods_id")
                            .ok_or_else(|| err("Food alternative goods missing"))?;
                        o.goods_id = ExternalId::from_json(good.clone()).map_err(err)?;
                        o.id = format!(
                            "need:{}:food:{}",
                            js_string(&pop.external_id.json()),
                            js_string(good)
                        );
                        o.quantity = checked(
                            quantity
                                * if total != 0.0 {
                                    arithmetic_number(truthy(a.get("share"))).unwrap_or(0.0) / total
                                } else {
                                    1.0 / choices.len() as f64
                                },
                        )?;
                        o.food_alternatives = alternatives.clone();
                        o.food_gross_quantity = Some(checked(
                            row.gross_quantity.unwrap_or(row.quantity.unwrap()) * multiplier,
                        )?);
                        o.military_food_substitution = Some(checked(
                            finite_or_null(n.get("military_substitution_quantity")).unwrap_or(0.0)
                                * multiplier,
                        )?);
                        result.orders.push(o);
                    }
                } else if let Some(good) = truthy(n.get("goods_id")) {
                    let mut o = base;
                    o.goods_id = ExternalId::from_json(good.clone()).map_err(err)?;
                    o.id = format!(
                        "need:{}:{}:{}",
                        js_string(&pop.external_id.json()),
                        js_string(&row.need_id.json()),
                        js_string(good)
                    );
                    result.orders.push(o);
                }
            }
            row.extra.insert("pop_id".into(), pop.external_id.json());
            result.rows.push(row);
        }
    }
    Ok(result)
}

#[derive(Debug, Clone)]
pub struct PopDemand {
    pub province_id: ProvinceId,
    pub goods: Goods,
    pub quantity: f64,
    pub priority: OrderPriority,
}

pub fn calculate_pop_demands(pops: &[PopRecord]) -> OpResult<Vec<PopDemand>> {
    calculate_pop_demands_with_reserves(pops, &[])
}

pub fn calculate_pop_demands_with_reserves(
    _pops: &[PopRecord],
    _stockpiles: &[kingmaker_core::StockpileRecord],
) -> OpResult<Vec<PopDemand>> {
    Err(err("Use build_need_orders/execute_market_phase with authoritative WorldState/request; detached POPs cannot supply norms or ownership"))
}
