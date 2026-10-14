//! Source-order BFS for walking passengers, one reachable tree per source province.
use crate::{budget_funding::error, needs::js_truthy, operation_session::OpResult};
use kingmaker_core::{
    market::MarketTerritory, production::ProductionPolicies, ExternalId, ProvinceId,
};
use serde_json::Value;
use std::collections::HashMap;
pub struct WalkingPaths<'a> {
    provinces: HashMap<ProvinceId, &'a MarketTerritory>,
    policies: &'a ProductionPolicies,
    parents: HashMap<ProvinceId, HashMap<ProvinceId, Option<ProvinceId>>>,
}
fn province(v: &Value) -> Option<ProvinceId> {
    ExternalId::from_json(v.clone())
        .ok()
        .and_then(|id| ProvinceId::from_external(&id).ok())
}
impl<'a> WalkingPaths<'a> {
    pub fn new(rows: impl IntoIterator<Item=&'a MarketTerritory>, policies: &'a ProductionPolicies) -> OpResult<Self> {
        let mut provinces = HashMap::new();
        for row in rows {
            if provinces.insert(row.province_id,row).is_some() {return Err(error("Duplicate walking province"));}
        }
        Ok(Self {
            provinces,
            policies,
            parents: HashMap::new(),
        })
    }
    pub fn path(&mut self, from: ProvinceId, to: ProvinceId) -> OpResult<Option<Vec<ProvinceId>>> {
        if !self.provinces.contains_key(&from) || !self.provinces.contains_key(&to) {
            return Ok(None);
        }
        if !self.parents.contains_key(&from) {
            let mut parents = HashMap::from([(from, None)]);
            let mut queue = vec![from];
            let mut cursor = 0;
            while cursor < queue.len() {
                crate::run_control::work("walking_path")?;
                let current = queue[cursor];
                cursor += 1;
                let p = self.provinces[&current];
                if p.extra.get("is_floating") != Some(&Value::Bool(false))
                    || !p.extra.get("landmass_id").is_some_and(js_truthy)
                {
                    continue;
                }
                let neighbors: Vec<Value> = if let Some(rows) =
                    p.extra.get("neighbors").and_then(Value::as_array)
                {
                    rows.iter()
                        .filter_map(|n| {
                            if n.is_object() {
                                (n.get("contact_type").and_then(Value::as_str) == Some("land"))
                                    .then(|| n.get("neighbor_id").cloned())
                                    .flatten()
                            } else {
                                Some(n.clone())
                            }
                        })
                        .collect()
                } else if let Some(rows) = p.extra.get("neighbor_ids").and_then(Value::as_array) {
                    rows.clone()
                } else {
                    p.extra
                        .get("neighbor_ids")
                        .filter(|v| js_truthy(v))
                        .map(|v| {
                            v.as_str()
                                .map(str::to_owned)
                                .unwrap_or_else(|| v.to_string())
                        })
                        .unwrap_or_default()
                        .split(|c: char| c == ',' || c == ';' || c.is_whitespace())
                        .filter(|s| !s.is_empty())
                        .map(|s| Value::String(s.into()))
                        .collect()
                };
                for id in neighbors {
                    crate::run_control::work("walking_edge")?;
                    let Some(key) = province(&id) else { continue };
                    let Some(q) = self.provinces.get(&key) else {
                        continue;
                    };
                    if parents.contains_key(&key)
                        || q.extra.get("is_floating") != Some(&Value::Bool(false))
                        || q.extra.get("landmass_id") != p.extra.get("landmass_id")
                    {
                        continue;
                    }
                    let from_state = p.extra.get("owner_state_id");
                    let to_state = q.extra.get("owner_state_id");
                    if from_state != to_state
                        && !self
                            .policies
                            .extra
                            .get("migration_borders")
                            .and_then(Value::as_array)
                            .is_some_and(|rows| {
                                rows.iter().any(|b| {
                                    b.get("from_state_id") == from_state
                                        && b.get("to_state_id") == to_state
                                        && b.get("allowed") == Some(&Value::Bool(true))
                                })
                            })
                    {
                        continue;
                    }
                    parents.insert(key, Some(current));
                    queue.push(key);
                }
            }
            self.parents.insert(from, parents);
        }
        let parents = &self.parents[&from];
        if !parents.contains_key(&to) {
            return Ok(None);
        }
        let mut path = vec![];
        let mut current = Some(to);
        while let Some(id) = current {
            path.push(id);
            current = parents[&id];
        }
        path.reverse();
        Ok(Some(path))
    }
}
