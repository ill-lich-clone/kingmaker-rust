//! Directed, authored-order land graph and a bounded FIFO cache for one phase.
use crate::{needs::{js_string, js_truthy}, operation_session::OpResult};
use kingmaker_core::{market::MarketData, ExternalId, ProvinceId};
use serde_json::Value;
use std::{collections::{HashMap, VecDeque}, sync::Arc};

pub const PHASE_PATH_CACHE_BYTES: usize = 16 * 1024 * 1024;
struct LandNode { land: Value, neighbors: Vec<ProvinceId> }
type Pair = (ProvinceId, ProvinceId);
type Path = Option<Arc<[ProvinceId]>>;
pub struct LandPathIndex {
    nodes: HashMap<ProvinceId, LandNode>,
    cache: HashMap<Pair, Path>,
    fifo: VecDeque<(Pair, usize)>,
    bytes: usize,
    limit: usize,
}
impl LandPathIndex {
    pub fn new(data: &MarketData, cache_byte_limit: usize) -> OpResult<Self> {
        Self::from_territory(data.territory.iter(), cache_byte_limit)
    }
    pub fn from_territory<'a>(territory:impl IntoIterator<Item=&'a kingmaker_core::market::MarketTerritory>,cache_byte_limit:usize)->OpResult<Self> {
        let mut nodes = HashMap::new();
        for row in territory {
            crate::run_control::work("land_graph")?;
            // Last duplicate wins, including an unusable definition.
            nodes.remove(&row.province_id);
            if row.extra.get("is_floating") != Some(&Value::Bool(false)) { continue; }
            let Some(land) = row.extra.get("landmass_id").filter(|v| js_truthy(v)) else { continue; };
            let parse = |v: &Value| ExternalId::from_json(v.clone()).ok()
                .and_then(|id| ProvinceId::from_external(&id).ok());
            let neighbors = if let Some(ns) = row.extra.get("neighbors").and_then(Value::as_array) {
                ns.iter().filter_map(|n| {
                    if !n.is_object() { parse(n) }
                    else if n.get("contact_type").and_then(Value::as_str) == Some("land") {
                        n.get("neighbor_id").and_then(parse)
                    } else { None }
                }).collect()
            } else if let Some(ns) = row.extra.get("neighbor_ids").and_then(Value::as_array) {
                ns.iter().filter_map(parse).collect()
            } else {
                row.extra.get("neighbor_ids").map(js_string).unwrap_or_default()
                    .split(|c: char| c == ',' || c == ';' || c.is_whitespace())
                    .filter(|s| !s.is_empty()).filter_map(|s| parse(&Value::String(s.into()))).collect()
            };
            nodes.insert(row.province_id, LandNode { land: land.clone(), neighbors });
        }
        Ok(Self { nodes, cache: HashMap::new(), fifo: VecDeque::new(), bytes: 0, limit: cache_byte_limit })
    }
    pub fn cache_bytes(&self) -> usize { self.bytes }
    pub fn path(&mut self, from: ProvinceId, to: ProvinceId) -> OpResult<Path> {
        let pair = (from, to);
        if let Some(path) = self.cache.get(&pair) { return Ok(path.clone()); }
        let path = self.find(from, to)?;
        // Conservative entry charge includes hash-table slack, FIFO capacity and Arc header.
        let cost = 256 + path.as_ref().map_or(0, |p| std::mem::size_of_val(&**p));
        if cost <= self.limit {
            while self.bytes > self.limit - cost {
                let (key, old_cost) = self.fifo.pop_front().expect("cache accounting");
                self.cache.remove(&key);
                self.bytes -= old_cost;
            }
            self.cache.insert(pair, path.clone());
            self.fifo.push_back((pair, cost));
            self.bytes += cost;
        }
        Ok(path)
    }
    fn find(&self, from: ProvinceId, to: ProvinceId) -> OpResult<Path> {
        let Some(start)=self.nodes.get(&from) else {return Ok(None)};
        if self.nodes.get(&to).is_none_or(|n|n.land!=start.land) {return Ok(None);}
        let mut parents = HashMap::from([(from, None)]);
        let mut queue = VecDeque::from([from]);
        while let Some(p) = queue.pop_front() {
            crate::run_control::work("land_path")?;
            if p == to {
                let mut path = vec![];
                let mut p = Some(to);
                while let Some(next) = p { path.push(next); p = parents[&next]; }
                path.reverse();
                return Ok(Some(path.into()));
            }
            for &next in &self.nodes[&p].neighbors {
                crate::run_control::work("land_edge")?;
                if self.nodes.get(&next).is_some_and(|n| n.land == start.land) && !parents.contains_key(&next) {
                    parents.insert(next, Some(p));
                    queue.push_back(next);
                }
            }
        }
        Ok(None)
    }
}
