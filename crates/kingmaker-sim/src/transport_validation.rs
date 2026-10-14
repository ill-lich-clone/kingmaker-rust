//! Current authored physical route terms; validation and adaptation never move cargo.
use kingmaker_core::world_read::WorldRead;
use crate::{
    budget_funding::error,
    needs::{js_string, js_truthy, undefined},
    operation_session::OpResult,
    storage::approved,
};
use kingmaker_core::{market::SaleOffer, ExternalId, ProvinceId};
use serde_json::{json, Value};
use std::collections::HashMap;

pub(crate) fn actor(v: &Value) -> OpResult<ExternalId> {
    ExternalId::from_json(v.clone()).map_err(crate::operation_session::OperationError)
}
pub(crate) fn province(v: &Value) -> OpResult<ProvinceId> {
    ProvinceId::from_external(&actor(v)?).map_err(crate::operation_session::OperationError)
}
fn approved_row(v: &Value) -> bool {
    approved(v["status"].as_str(), v.get("source"), v.get("approval_ref"))
}
fn positive(v: &Value) -> bool {
    v.as_f64().is_some_and(|n| n.is_finite() && n > 0.)
}
fn safe(v: &Value, positive: bool) -> bool {
    v.as_f64().is_some_and(|n| {
        n.is_finite()
            && n.fract() == 0.
            && n <= 9_007_199_254_740_991.
            && if positive { n > 0. } else { n >= 0. }
    })
}

pub struct RouteValidation<'a> {
    world: &'a dyn WorldRead,
    assets: HashMap<ExternalId, &'a Value>,
    terminals: HashMap<ExternalId, &'a Value>,
    provinces: HashMap<ProvinceId, Value>,
    sea: HashMap<ProvinceId, &'a Value>,
    paths: HashMap<(ProvinceId, ProvinceId), Option<Vec<ProvinceId>>>,
}
impl<'a> RouteValidation<'a> {
    pub fn new(w: &'a dyn WorldRead) -> OpResult<Self> {
        let assets = w.transport_assets().values()
            .filter_map(|v| actor(&v["id"]).ok().map(|id| (id, v)))
            .collect();
        let terminals = w.transport_terminals().values()
            .filter_map(|v| actor(&v["id"]).ok().map(|id| (id, v)))
            .collect();
        let sea: HashMap<_, _> = w.navigation_geography().values()
            .filter_map(|v| province(&v["province_id"]).ok().map(|id| (id, v)))
            .collect();
        let mut provinces = sea
            .iter()
            .map(|(id, v)| (*id, (*v).clone()))
            .collect::<HashMap<_, _>>();
        for row in w.territory().values() {
            provinces.insert(row.province_id, serde_json::to_value(row)?);
        }
        Ok(Self {
            world: w,
            assets,
            terminals,
            provinces,
            sea,
            paths: HashMap::new(),
        })
    }
    pub fn reason(&self, route: &Value, offer: &SaleOffer) -> Option<&'static str> {
        let bad = || Some("transport_parameters_unknown");
        let Ok(asset_id) = actor(&route["asset_id"]) else {
            return bad();
        };
        let Some(asset) = self.assets.get(&asset_id) else {
            return bad();
        };
        let mapping = &route["goods"][js_string(&offer.goods_id.json())];
        let mode = route["mode"].as_str();
        if !approved_row(route)
            || !approved_row(asset)
            || !matches!(mode, Some("land" | "sea" | "air"))
            || !asset["modes"]
                .as_array()
                .is_some_and(|m| m.contains(&route["mode"]))
            || asset["owner_id"] != route["carrier_id"]
            || js_string(&asset["home_province_id"]) != js_string(&route["from_province_id"])
            || !js_truthy(&mapping["unit"])
            || offer.extra.get("unit") != mapping.get("unit")
            || !positive(&mapping["space_per_unit"])
            || asset["unit"] != route["unit"]
            || ![
                &route["capacity_per_week"],
                &route["source_handling_per_week"],
                &route["destination_handling_per_week"],
                &asset["capacity"],
            ]
            .iter()
            .all(|v| positive(v))
            || !safe(&route["travel_days"], true)
            || !safe(&route["return_days"], false)
            || route
                .get("freight_minor_per_space")
                .is_none_or(|v| v.is_null() || undefined(v))
        {
            return bad();
        }
        if route.get("currency_id") != offer.extra.get("currency_id") {
            return Some("transport_currency_mismatch");
        }
        let Some(path) = route["path"].as_array() else {
            return Some("physical_route_unknown");
        };
        let path_ids = path
            .iter()
            .map(province)
            .collect::<Result<Vec<_>, _>>()
            .ok();
        let Some(ids) = path_ids else {
            return Some("physical_route_unknown");
        };
        if ids.len() < 2
            || js_string(&path[0]) != js_string(&route["from_province_id"])
            || js_string(path.last().unwrap()) != js_string(&route["to_province_id"])
            || ids.iter().any(|id| !self.provinces.contains_key(id))
        {
            return Some("physical_route_unknown");
        }
        if mode == Some("land") {
            let land = &self.provinces[&ids[0]]["landmass_id"];
            for (i, id) in ids.iter().enumerate() {
                let p = &self.provinces[id];
                if p["is_floating"] != json!(false) || !js_truthy(land) || &p["landmass_id"] != land
                {
                    return Some("physical_land_route_unavailable");
                }
                if i + 1 < ids.len() {
                    let neighbors = if let Some(rows) = p["neighbor_ids"].as_array() {
                        rows.iter().map(js_string).collect::<Vec<_>>()
                    } else {
                        js_string(&p["neighbor_ids"])
                            .split(|c: char| matches!(c, ';' | ',' | '|') || c.is_whitespace())
                            .map(str::to_owned)
                            .collect()
                    };
                    if !neighbors.contains(&js_string(&path[i + 1])) {
                        return Some("physical_land_route_unavailable");
                    }
                }
            }
        } else {
            if mode == Some("sea") {
                for (i, id) in ids.iter().enumerate() {
                    let Some(p) = self.sea.get(id) else {
                        return Some("physical_sea_route_unavailable");
                    };
                    if js_truthy(&p["is_floating"])
                        || (i > 0
                            && i + 1 < ids.len()
                            && !matches!(p["kind"].as_str(), Some("sea" | "water")))
                        || (i + 1 < ids.len()
                            && !p["neighbors"].as_array().is_some_and(|rows| {
                                rows.iter().any(|n| {
                                    js_string(&n["neighbor_id"]) == js_string(&path[i + 1])
                                        && matches!(
                                            n["contact_type"].as_str(),
                                            Some("water" | "coast")
                                        )
                                })
                            }))
                    {
                        return Some("physical_sea_route_unavailable");
                    }
                }
            }
            let kind = if mode == Some("air") {
                "air_port"
            } else {
                "sea_port"
            };
            for (terminal, location) in [
                ("source_terminal_id", "from_province_id"),
                ("destination_terminal_id", "to_province_id"),
            ] {
                let t = actor(&route[terminal])
                    .ok()
                    .and_then(|id| self.terminals.get(&id));
                if t.is_none_or(|t| {
                    !approved_row(t)
                        || t["kind"] != json!(kind)
                        || js_string(&t["province_id"]) != js_string(&route[location])
                }) {
                    return Some("physical_terminal_unavailable");
                }
            }
        }
        None
    }
    pub fn adapt_land(
        &mut self,
        template: &Value,
        from: ProvinceId,
        to: ProvinceId,
    ) -> OpResult<Option<Value>> {
        if !self.paths.contains_key(&(from, to)) {
            self.paths.insert(
                (from, to),
                crate::land_paths::LandPathIndex::from_territory(self.world.territory().values(),0)?.path(from,to)?.map(|p|p.to_vec()),
            );
        }
        let Some(path) = self.paths[&(from, to)].as_ref() else {
            return Ok(None);
        };
        let edges = path
            .len()
            .checked_sub(1)
            .ok_or_else(|| error("Empty adapted land path"))?;
        let mut route = template.clone();
        route["id"] = json!(format!("land-path:{}:{}", from.0, to.0));
        route["from_province_id"] = json!(from.0);
        route["to_province_id"] = json!(to.0);
        route["path"] = json!(path.iter().map(|p| p.0).collect::<Vec<_>>());
        route["travel_days"] = json!(7 * edges);
        route["return_days"] = json!(7 * edges);
        let fallback = route["freight_minor_per_space"].clone();
        let Some(goods) = route["goods"].as_object_mut() else {
            return Err(error("Adapted freight mapping missing"));
        };
        for mapping in goods.values_mut() {
            let rate = mapping
                .get("freight_minor_per_space")
                .filter(|v| !v.is_null() && !undefined(v))
                .unwrap_or(&fallback);
            let amount: kingmaker_core::Money = serde_json::from_value(rate.clone())?;
            let scaled = amount
                .0
                .checked_mul(i64::try_from(edges).map_err(|_| error("Land edges overflow"))?)
                .ok_or_else(|| error("Land freight overflow"))?;
            mapping["freight_minor_per_space"] = json!(scaled.to_string());
        }
        Ok(Some(route))
    }
}
