use kingmaker_core::{world_read::WorldRead,row_store::ReadSequence};
// Authored levels determine capacity; population never determines a level here.
use crate::{
    needs::{finite_or_null, js_string, js_truthy, undefined},
    operation_session::{OpResult, OperationError},
};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
const MAX_SAFE: u64 = 9_007_199_254_740_991;
fn err(s: &str) -> OperationError {
    OperationError(s.into())
}
fn undef() -> Value {
    json!({"__kingmaker_codec_v2__":"undefined"})
}
fn value(row: &Value, key: &str) -> Value {
    row.get(key).cloned().unwrap_or_else(undef)
}
fn id_key(v: &Value) -> String {
    v.to_string()
}
fn nullish(v: &Value) -> bool {
    v.is_null() || undefined(v)
}
fn safe_integer(v: &Value) -> Option<u64> {
    let n = v.as_f64()?;
    (n.is_finite() && n >= 0. && n.fract() == 0. && n <= MAX_SAFE as f64).then_some(n as u64)
}
pub fn derive_labor_jobs(world: &dyn WorldRead) -> OpResult<Vec<Value>> {
    let mut jobs = if let Some(rules) = &world.institution_rules() {
        derive_institution_jobs(
            &world.institutions(),
            &world.jobs(),
            rules,
        )?
    } else {
        world.jobs().iter().cloned().collect()
    };
    let mut capacities = HashMap::new();
    for rgo in &world.resource_operations() {
        capacities.insert(
            id_key(&rgo.base.id.json()),
            crate::production_activities::rgo_capacity(rgo, &world.expansions())?,
        );
    }
    for factory in &world.factories() {
        let slots = factory
            .people_slots
            .ok_or_else(|| err("Factory physical labor capacity unresolved"))?;
        if !slots.is_finite() || slots < 0. {
            return Err(err("Invalid factory physical capacity"));
        }
        capacities.insert(id_key(&factory.base.id.json()), slots);
    }
    for job in &mut jobs {
        let building = value(job, "building_id");
        if let Some(capacity) = capacities.get(&id_key(&building)) {
            let fields = job
                .as_object_mut()
                .ok_or_else(|| err("Production job must be an object"))?;
            fields.insert("slots_total".into(), json!(capacity));
            fields.insert("capacity_pool_id".into(), building);
            fields.insert("capacity_pool_total".into(), json!(capacity));
        }
    }
    Ok(jobs)
}
pub fn institution_capacity(building: &Value, rule: &Value) -> OpResult<u64> {
    let level = building
        .get("level")
        .and_then(safe_integer)
        .ok_or_else(|| err("Institution level must be a nonnegative integer"))?;
    if rule.get("places_per_level").and_then(Value::as_f64) != Some(2000.) {
        return Err(err("Institution rule must specify 2000 places per level"));
    }
    level
        .checked_mul(2000)
        .filter(|n| *n <= MAX_SAFE)
        .ok_or_else(|| err("Institution capacity overflow"))
}
fn apportion(total: u64, ids: &[Value], weights: &[f64]) -> OpResult<Vec<u64>> {
    if ids.is_empty() {
        return Ok(vec![]);
    }
    let sum: f64 = weights.iter().sum();
    if !sum.is_finite() || sum <= 0. || weights.iter().any(|w| !w.is_finite() || *w < 0.) {
        return Err(err("Invalid institution role weights"));
    }
    let exact = weights
        .iter()
        .map(|w| (total as f64) * w / sum)
        .collect::<Vec<_>>();
    if exact
        .iter()
        .any(|n| !n.is_finite() || *n < 0. || *n > MAX_SAFE as f64)
    {
        return Err(err("Institution apportion overflow"));
    }
    let mut amounts = exact.iter().map(|n| n.floor() as u64).collect::<Vec<_>>();
    let used = amounts
        .iter()
        .try_fold(0u64, |n, a| n.checked_add(*a))
        .ok_or_else(|| err("Institution allocation overflow"))?;
    let mut left = total
        .checked_sub(used)
        .ok_or_else(|| err("Institution apportion exceeds capacity"))?;
    let locale: icu_locale_core::Locale = "en".parse().unwrap();
    let collator = icu_collator::Collator::try_new(locale.into(), Default::default())
        .map_err(|_| err("English collation unavailable"))?;
    let mut order = (0..ids.len()).collect::<Vec<_>>();
    order.sort_by(|&a, &b| {
        (exact[b] % 1.)
            .partial_cmp(&(exact[a] % 1.))
            .unwrap()
            .then_with(|| collator.compare(&js_string(&ids[a]), &js_string(&ids[b])))
    });
    for i in order {
        if left == 0 {
            break;
        }
        amounts[i] += 1;
        left -= 1;
    }
    if left != 0 {
        return Err(err("Institution apportion lost capacity"));
    }
    Ok(amounts)
}
pub fn derive_institution_jobs(
    institutions: &(impl ReadSequence<Value> + ?Sized),
    jobs: &(impl ReadSequence<Value> + ?Sized),
    rules: &Value,
) -> OpResult<Vec<Value>> {
    let mut by_building = HashMap::<String, Vec<&Value>>::new();
    for j in jobs.iter() {
        by_building
            .entry(id_key(&value(j, "building_id")))
            .or_default()
            .push(j);
    }
    let mut replacements = HashMap::new();
    let mut added = vec![];
    for b in institutions.iter() {
        let Some(rule) = rules
            .get(js_string(&value(b, "building_type_id")))
            .filter(|r| js_truthy(r))
        else {
            continue;
        };
        let capacity = institution_capacity(b, rule)?;
        let id = b
            .get("building_id")
            .filter(|v| js_truthy(v))
            .or_else(|| b.get("id"))
            .filter(|v| js_truthy(v))
            .ok_or_else(|| err("Institution roles unresolved"))?;
        let roles = b
            .get("staffing_roles")
            .filter(|v| js_truthy(v))
            .or_else(|| rule.get("roles"))
            .and_then(Value::as_array)
            .filter(|r| !r.is_empty())
            .ok_or_else(|| err("Institution roles unresolved"))?;
        let role_ids = roles
            .iter()
            .map(|r| value(r, "role_id"))
            .collect::<Vec<_>>();
        let mut seen = HashSet::new();
        if role_ids.iter().any(|id| !seen.insert(id_key(id))) {
            return Err(err("Institution roles unresolved"));
        }
        let existing = by_building.get(&id_key(id)).cloned().unwrap_or_default();
        if existing
            .iter()
            .any(|j| !role_ids.contains(&value(j, "role_id")))
        {
            return Err(err("Institution job role unresolved"));
        }
        let weights = roles
            .iter()
            .map(|r| {
                r.get("weight")
                    .and_then(Value::as_f64)
                    .ok_or_else(|| err("Invalid institution role weights"))
            })
            .collect::<OpResult<Vec<_>>>()?;
        let role_amounts = apportion(capacity, &role_ids, &weights)?;
        for ((role, role_id), role_amount) in roles.iter().zip(&role_ids).zip(role_amounts) {
            let pop_type = value(role, "pop_type_id");
            if !js_truthy(role_id) || !js_truthy(&pop_type) {
                return Err(err("Institution profession unresolved"));
            }
            let old = existing
                .iter()
                .filter(|j| value(j, "role_id") == *role_id)
                .copied()
                .collect::<Vec<_>>();
            let rows = if old.is_empty() {
                vec![
                    json!({"job_id":format!("{}:{}",crate::labor_identity::component_value(id),crate::labor_identity::component_value(role_id)),"building_id":id,"role_id":role_id,"pop_type_id":pop_type}),
                ]
            } else {
                old.iter().map(|j| (*j).clone()).collect()
            };
            let job_ids = rows.iter().map(|j| value(j, "job_id")).collect::<Vec<_>>();
            let mut weights = rows
                .iter()
                .map(|j| {
                    let w = j
                        .get("capacity_weight")
                        .filter(|v| !nullish(v))
                        .or_else(|| j.get("slots_total").filter(|v| !nullish(v)));
                    finite_or_null(w)
                        .map(|n| n.max(0.))
                        .unwrap_or(if w.is_none() { 0. } else { f64::NAN })
                })
                .collect::<Vec<_>>();
            if weights.iter().any(|w| !w.is_finite()) {
                return Err(err("Invalid institution job weights"));
            }
            if !weights.iter().any(|w| *w > 0.) {
                weights.fill(1.);
            }
            let amounts = apportion(role_amount, &job_ids, &weights)?;
            for ((j, amount), weight) in rows.iter().zip(amounts).zip(weights) {
                let mut next = j
                    .as_object()
                    .ok_or_else(|| err("Institution job must be an object"))?
                    .clone();
                let state = b
                    .get("state_id")
                    .filter(|v| !nullish(v))
                    .cloned()
                    .unwrap_or_else(|| value(b, "owner_entity_id"));
                for (key, val) in [
                    ("building_id", id.clone()),
                    ("province_id", value(b, "province_id")),
                    ("state_id", state),
                    ("pop_type_id", pop_type.clone()),
                    ("role_id", role_id.clone()),
                    ("slots_total", json!(amount)),
                    (
                        "capacity_pool_id",
                        json!(format!(
                            "institution:{}",
                            crate::labor_identity::component_value(id)
                        )),
                    ),
                    ("capacity_pool_total", json!(capacity)),
                    ("capacity_weight", json!(weight)),
                    ("capacity_source", json!("institution_level")),
                    ("operational_status", value(b, "operational_status")),
                ] {
                    next.insert(key.into(), val);
                }
                let next = Value::Object(next);
                if old.is_empty() {
                    added.push(next);
                } else {
                    replacements.insert(id_key(&value(j, "job_id")), next);
                }
            }
        }
    }
    let mut result = jobs
        .iter()
        .map(|j| {
            replacements
                .get(&id_key(&value(j, "job_id")))
                .cloned()
                .unwrap_or_else(|| j.clone())
        })
        .collect::<Vec<_>>();
    result.extend(added);
    Ok(result)
}

