//! Source construction state machine; the closed cycle owns publication and prerequisites.
use kingmaker_core::world_read::WorldRead;
use crate::{
    budget_funding::error,
    construction_operations::stage_construction_operations,
    construction_rights::{complete_project_ownership, plan_project_refunds},
    needs::{js_string, js_truthy},
    operation_session::{OpResult, OperationSession},
    transport_validation::{actor, province},
};
use kingmaker_core::{
    construction::ConstructionRequest, operations::*, production::*, Money,
};
use serde_json::{json, Value};
use std::collections::HashSet;
fn money(v: &Value) -> OpResult<Money> {
    let n: Money = serde_json::from_value(v.clone())?;
    if n.0 < 0 {
        return Err(error("Negative construction money"));
    }
    Ok(n)
}
fn cash(
    w: &dyn WorldRead,
    owner: &kingmaker_core::ExternalId,
    currency: &kingmaker_core::ExternalId,
    purpose: &str,
) -> Money {
    w.account_by_address(&kingmaker_core::AccountAddress {
        owner_id: owner.clone(),
        currency_id: currency.clone(),
        purpose: purpose.into(),
    })
    .map(|a| a.balance)
    .unwrap_or(Money::ZERO)
}
fn apply(
    w: &mut crate::world_transaction::WorldTransaction<'_>,
    all: &mut Vec<EconomicOperation>,
    ops: &[EconomicOperation],
) -> OpResult<()> {
    if ops.is_empty() {
        return Ok(());
    }
    let mut session = OperationSession::new(w)?;
    stage_construction_operations(&mut session, ops)?;
    let changes = session.finish()?;
    w.apply_phase(changes)?;
    all.extend_from_slice(ops);
    Ok(())
}
fn operation(v: Value) -> OpResult<EconomicOperation> {
    Ok(serde_json::from_value(v)?)
}
pub(crate) fn progress(
    w: &mut crate::world_transaction::WorldTransaction<'_>,
    r: &ConstructionRequest,
    paid: &[EconomicOperation],
) -> OpResult<(Vec<EconomicOperation>, Vec<Value>)> {
    let mut operations = vec![];
    let mut blocked = vec![];
    for index in 0..w.construction_projects().len() {
        crate::run_control::work("construction_projects")?;
        let original = w.construction_projects().at(index).expect("validated row position");
        if matches!(
            original.status.as_str(),
            "completed" | "cancelled" | "proposed" | "awaiting_funding" | "awaiting_owner_consent"
        ) {
            continue;
        }
        if let Some(reason) = crate::construction_limits::building_limit_reason(w, original)? {
            blocked.push(json!({"project_id":original.id,"reason":reason}));
            continue;
        }
        let mut p = serde_json::to_value(original)?;
        let c = p["construction"].clone();
        let duration = c["duration_days"]
            .as_f64()
            .filter(|n| n.is_finite() && *n > 0. && n.fract() == 0. && *n <= 9007199254740991.);
        let required = c["labor_required_fte_days"]
            .as_f64()
            .filter(|n| n.is_finite() && *n > 0.);
        let materials = c["materials"].as_array();
        if c["status"] != "ready"
            || !js_truthy(&c["source"])
            || duration.is_none()
            || required.is_none()
            || !matches!(
                c["cash_cost_basis"].as_str(),
                Some("includes_wages" | "excludes_wages")
            )
            || c["provider_id"].is_null()
            || c["currency_id"] != p["terms"]["currency_id"]
            || c["cost_minor"].is_null()
            || materials.is_none_or(|m| {
                m.iter().any(|i| {
                    i["goods_id"].is_null()
                        || i["amount"]
                            .as_f64()
                            .is_none_or(|n| !n.is_finite() || n <= 0.)
                })
            })
        {
            blocked
                .push(json!({"project_id":p["id"],"reason":"construction_parameters_unresolved"}));
            continue;
        }
        let price = money(&c["cost_minor"])?;
        let required = required.unwrap();
        let duration = duration.unwrap();
        let owner = actor(&p["owner_id"])?;
        let currency = actor(&c["currency_id"])?;
        let place = province(&p["province_id"])?;
        let assignments = w.construction_projects().at(index).expect("validated row position")
            .assignments
            .clone();
        crate::production_activities::validate_production_assignments(&w.pops(), &assignments)?;
        for a in &assignments {
            let pop = w.pops()
                .iter()
                .find(|p| p.external_id == a.pop_id)
                .ok_or_else(|| error("Construction POP missing"))?;
            if !matches!(
                js_string(&pop.pop_type_id.json()).as_str(),
                "farmers" | "laborers"
            ) || pop.province_id != place
            {
                return Err(error("Invalid construction worker"));
            }
        }
        let mut changes = vec![];
        let mut missing = false;
        if p["materials_consumed"] != true {
            for material in materials.unwrap() {
                let good = actor(&material["goods_id"])?;
                let mut left = material["amount"].as_f64().unwrap();
                for stock in w.stockpiles().iter().filter(|s| {
                    s.owner_id == owner && s.province_id == place && s.goods.external_id() == good
                }) {
                    let q = left.min(stock.quantity);
                    if q > 0. {
                        changes.push(json!({"stock_id":stock.external_id,"delta":-q}));
                    }
                    left -= q;
                }
                if left > 1e-9 {
                    missing = true;
                }
            }
        }
        if missing {
            blocked
                .push(json!({"project_id":p["id"],"reason":"construction_materials_unavailable"}));
            continue;
        }
        let event = format!(
            "construction:{}:{}:{}",
            w.campaign().campaign_id,
            r.period,
            js_string(&p["id"])
        );
        let elapsed = p["elapsed_days"].as_f64().unwrap_or(0.) + 7.;
        if !elapsed.is_finite() || elapsed > 9007199254740991. {
            return Err(error("Construction elapsed time overflow"));
        }
        p["elapsed_days"] = json!(elapsed as u64);
        let mut fte = 0.;
        let mut wages = 0i64;
        let mut seen = HashSet::new();
        for op in paid.iter().filter(|op| {
            op.kind == OperationKind::Payroll
                && op.meta.get("period") == Some(&json!(r.period))
                && op.meta.get("employer_id") == Some(&owner.json())
        }) {
            let Some(a) = assignments.iter().find(|a| {
                a.employment_id
                    .as_ref()
                    .is_some_and(|id| op.meta.get("employment_id") == Some(&id.json()))
                    && op.meta.get("pop_id") == Some(&a.pop_id.json())
            }) else {
                continue;
            };
            let actual = op
                .meta
                .get("actual_fte")
                .and_then(Value::as_f64)
                .ok_or_else(|| error("Builder actual FTE missing"))?;
            if !seen.insert(&op.event_id)
                || op.meta.get("currency_id") != Some(&currency.json())
                || !actual.is_finite()
                || actual < 0.
                || actual > a.workers_count * a.time_fraction + 1e-7
            {
                return Err(error("Invalid construction paid labor receipt"));
            }
            fte += actual;
            wages = wages
                .checked_add(money(op.meta.get("paid_gross_minor").unwrap_or(&Value::Null))?.0)
                .ok_or_else(|| error("Construction wage sum overflow"))?;
        }
        let previous = p["labor_fte_days"].as_f64().unwrap_or(0.);
        if fte == 0. && previous + 1e-8 < required {
            p["status"] = json!("awaiting_labor");
            blocked
                .push(json!({"project_id":p["id"],"reason":"construction_paid_labor_unavailable"}));
            apply(
                w,
                &mut operations,
                &[operation(
                    json!({"event_id":event,"type":"construction_wait","project_id":p["id"]}),
                )?],
            )?;
            let key=w.construction_projects().keyed().nth(index).ok_or_else(||error("Missing selected construction_projects row"))?.0;
        w.put_construction_projects(key,serde_json::from_value(p)?)?;
            continue;
        }
        let old = p
            .get("paid_wages_minor")
            .map(money)
            .transpose()?
            .unwrap_or(Money::ZERO);
        let total = old
            .0
            .checked_add(wages)
            .ok_or_else(|| error("Construction accumulated wages overflow"))?;
        p["paid_wages_minor"] = json!(total.to_string());
        p["labor_fte_days"] = json!(required.min(previous + fte * 7.));
        if p["materials_consumed"] != true {
            apply(
                w,
                &mut operations,
                &[operation(
                    json!({"event_id":format!("{event}:materials"),"type":"construction_materials","project_id":p["id"],"stock_changes":changes}),
                )?],
            )?;
            p["materials_consumed"] = json!(true);
        }
        p["status"] = json!("building");
        apply(
            w,
            &mut operations,
            &[operation(
                json!({"event_id":event,"type":"construction_progress","project_id":p["id"],"labor_fte_days":p["labor_fte_days"]}),
            )?],
        )?;
        if elapsed < duration || p["labor_fte_days"].as_f64().unwrap() + 1e-8 < required {
            let key=w.construction_projects().keyed().nth(index).ok_or_else(||error("Missing selected construction_projects row"))?.0;
        w.put_construction_projects(key,serde_json::from_value(p)?)?;
            continue;
        }
        let config = p["factory_configuration"].clone();
        let factory = p["kind"] == "factory";
        let expansion = js_truthy(&p["target_factory_id"]);
        if factory && !expansion {
            let future_owner = format!("enterprise:{}", js_string(&p["id"]));
            let facility = config.get("facility_id").and_then(|id| {
                w.storage_facilities()
                    .iter()
                    .find(|f| f.id.json() == *id)
            });
            if config["people_slots"].as_f64() != Some(40000.)
                || !js_truthy(&config["capacity_source"])
                || config["equipment_capacity_weekly"]
                    .as_f64()
                    .is_none_or(|n| !n.is_finite() || n <= 0.)
                || !js_truthy(&config["equipment_source"])
                || facility.is_none_or(|f| f.owner_id.json() != json!(future_owner))
            {
                blocked.push(json!({"project_id":p["id"],"reason":"factory_commissioning_configuration_unresolved"}));
                let key=w.construction_projects().keyed().nth(index).ok_or_else(||error("Missing selected construction_projects row"))?.0;
        w.put_construction_projects(key,serde_json::from_value(p)?)?;
                continue;
            }
        }
        let fee = price.0
            - if c["cash_cost_basis"] == "includes_wages" {
                total
            } else {
                0
            };
        if fee < 0 {
            blocked.push(
                json!({"project_id":p["id"],"reason":"construction_wages_exceed_cash_price"}),
            );
            let key=w.construction_projects().keyed().nth(index).ok_or_else(||error("Missing selected construction_projects row"))?.0;
        w.put_construction_projects(key,serde_json::from_value(p)?)?;
            continue;
        }
        if cash(w, &owner, &currency, "construction_escrow").0 < fee {
            blocked.push(
                json!({"project_id":p["id"],"reason":"construction_final_payment_unavailable"}),
            );
            let key=w.construction_projects().keyed().nth(index).ok_or_else(||error("Missing selected construction_projects row"))?.0;
        w.put_construction_projects(key,serde_json::from_value(p)?)?;
            continue;
        }
        let reserve = if factory {
            p["terms"]
                .get("operating_reserve_minor")
                .map(money)
                .transpose()?
                .unwrap_or(Money::ZERO)
        } else {
            Money::ZERO
        };
        let purpose = if p["protected_operating_reserve"] == true {
            "factory_startup_reserve"
        } else {
            "construction_escrow"
        };
        let budget = money(&p["terms"]["budget_minor"])?;
        if reserve.0 > budget.0 - price.0 {
            blocked
                .push(json!({"project_id":p["id"],"reason":"factory_operating_reserve_invalid"}));
            let key=w.construction_projects().keyed().nth(index).ok_or_else(||error("Missing selected construction_projects row"))?.0;
        w.put_construction_projects(key,serde_json::from_value(p)?)?;
            continue;
        }
        if reserve.0 > 0
            && cash(w, &owner, &currency, purpose).0
                < reserve.0
                    + if purpose == "construction_escrow" {
                        fee
                    } else {
                        0
                    }
        {
            blocked.push(
                json!({"project_id":p["id"],"reason":"factory_operating_reserve_unavailable"}),
            );
            let key=w.construction_projects().keyed().nth(index).ok_or_else(||error("Missing selected construction_projects row"))?.0;
        w.put_construction_projects(key,serde_json::from_value(p)?)?;
            continue;
        }
        let mut ownership = complete_project_ownership(w, &p)?;
        let original_factory = if expansion {
            let f = w.factories()
                .iter()
                .find(|f| f.base.id.json() == p["target_factory_id"])
                .ok_or_else(|| error("Expansion factory missing"))?;
            let value = serde_json::to_value(f)?;
            ownership["asset"] = crate::construction_rights::complete_expansion(w, &value, &p)?;
            ownership["shares"] = ownership["asset"]["shares"].clone();
            Some(value)
        } else {
            None
        };
        apply(
            w,
            &mut operations,
            &[operation(
                json!({"event_id":format!("{event}:fee"),"type":"construction_contract_fee","project_id":p["id"],"debits":[{"owner_id":owner,"currency_id":currency,"purpose":"construction_escrow","amount_minor":fee.to_string()}],"credits":[{"owner_id":c["provider_id"],"currency_id":currency,"purpose":"operating_capital","amount_minor":fee.to_string()}]}),
            )?],
        )?;
        p["status"] = json!("completed");
        p["place_reserved"] = json!(false);
        p["completed_asset_id"] = ownership["asset"]["id"].clone();
        p["shares"] = ownership["shares"].clone();
        if factory {
            p["factory_id"] = ownership["asset"]["id"].clone();
            let mut asset = ownership["asset"].clone();
            if reserve.0 > 0 {
                apply(
                    w,
                    &mut operations,
                    &[operation(
                        json!({"event_id":format!("{event}:working-capital"),"type":"factory_working_capital","income_recognized":false,"debits":[{"owner_id":owner,"currency_id":currency,"purpose":purpose,"amount_minor":reserve}],"credits":[{"owner_id":asset["owner_id"],"currency_id":currency,"purpose":"employer_operating_capital","amount_minor":reserve}]}),
                    )?],
                )?;
            }
            let refund = plan_project_refunds(w, &p, &event)?;
            let returned = refund
                .refunds
                .iter()
                .map(|r| money(&r["amount_minor"]).map(|n| i128::from(n.0)))
                .collect::<OpResult<Vec<_>>>()?
                .into_iter()
                .sum::<i128>();
            p["investor_refunds"] = json!(refund.refunds);
            p["refunded_minor"] = json!(returned.to_string());
            apply(w, &mut operations, &refund.operations)?;
            ownership = complete_project_ownership(w, &p)?;
            if let Some(original) = &original_factory {
                asset = crate::construction_rights::complete_expansion(w, original, &p)?;
                p["shares"] = asset["shares"].clone();
                let old_level = original["level"].as_f64().unwrap_or(1.);
                let new_level = asset["level"].as_f64().unwrap();
                let mut facility_updates=vec![];
                for facility in w.storage_facilities().iter() {
                    if facility.owner_id.json() == asset["owner_id"]
                        && facility.province_id == place
                    {
                        let capacity = facility
                            .capacity
                            .ok_or_else(|| error("Expansion storage capacity missing"))?
                            * new_level
                            / old_level;
                        if !capacity.is_finite() || capacity < 0. {
                            return Err(error("Invalid expansion storage capacity"));
                        }
                        let mut facility=facility.clone();facility.capacity=Some(capacity);facility_updates.push(facility);
                    }
                }
                for facility in facility_updates {w.put_storage_facilities(facility.id.clone(),facility)?;}
                let position = w.factories()
                    .iter()
                    .position(|f| f.base.id.json() == asset["id"])
                    .unwrap();
                let key=w.factories().keyed().nth(position).ok_or_else(||error("Missing selected factories row"))?.0;
        w.put_factories(key,serde_json::from_value(asset)?)?;
                let key=w.construction_projects().keyed().nth(index).ok_or_else(||error("Missing selected construction_projects row"))?.0;
        w.put_construction_projects(key,serde_json::from_value(p)?)?;
                continue;
            }
            asset["shares"] = ownership["shares"].clone();
            asset["capital_value_minor"] = json!((money(&p["funded_minor"])?.0
                - i64::try_from(returned).map_err(|_| error("Refund overflow"))?)
            .to_string());
            p["shares"] = asset["shares"].clone();
            let object = asset
                .as_object_mut()
                .ok_or_else(|| error("New factory asset invalid"))?;
            object.extend(
                config
                    .as_object()
                    .ok_or_else(|| error("Factory configuration missing"))?
                    .clone(),
            );
            for field in [
                "id",
                "owner_id",
                "enterprise_id",
                "province_id",
                "region_id",
                "method_id",
                "level",
                "kind",
                "provenance",
            ] {
                asset[field] = ownership["asset"][field].clone();
            }
            asset["shares"] = ownership["shares"].clone();
            asset["capital_value_minor"] = json!((money(&p["funded_minor"])?.0
                - i64::try_from(returned).map_err(|_| error("Refund overflow"))?)
            .to_string());
            asset["assignments"] = json!([]);
            asset["tenure_days"] = json!(0);
            asset["staff_count"] = json!(0);
            asset["status"] = json!("awaiting_staff");
            let row:kingmaker_core::production::FactoryEnterprise=serde_json::from_value::<FactoryEnterprise>(asset)?;
        w.put_factories(row.base.id.clone(),row)?;
        } else if matches!(p["kind"].as_str(), Some("mine" | "pasture")) {
            finish_nonfactory(w, &mut p, &event, &mut operations, &mut ownership)?;
            let row:kingmaker_core::production::RgoExpansion=serde_json::from_value(json!({"id":p["id"],"rgo_id":p["target_rgo_id"],"kind":p["kind"],"status":"completed","completed_levels":p.get("completed_levels").cloned().unwrap_or(json!(1)),"places_per_level":2000,"asset":ownership["asset"]}))?;
        w.put_expansions(row.id.clone(),row)?;
        } else {
            finish_nonfactory(w, &mut p, &event, &mut operations, &mut ownership)?;
            let key=w.completed_construction().next_key().map_err(crate::operation_session::OperationError)?;
        w.put_completed_construction(key,serde_json::from_value(json!({"id":p["id"],"province_id":p["province_id"],"region_id":p["region_id"],"kind":p["kind"],"status":"completed","asset":ownership["asset"],"effects":c.get("effects").cloned().unwrap_or(json!({})),"source":c["source"]}))?)?;
        }
        let key=w.construction_projects().keyed().nth(index).ok_or_else(||error("Missing selected construction_projects row"))?.0;
        w.put_construction_projects(key,serde_json::from_value(p)?)?;
    }
    Ok((operations, blocked))
}
fn finish_nonfactory(
    w: &mut crate::world_transaction::WorldTransaction<'_>,
    p: &mut Value,
    event: &str,
    ops: &mut Vec<EconomicOperation>,
    ownership: &mut Value,
) -> OpResult<()> {
    let refund = plan_project_refunds(w, p, event)?;
    let returned = refund
        .refunds
        .iter()
        .map(|r| money(&r["amount_minor"]).map(|m| i128::from(m.0)))
        .collect::<OpResult<Vec<_>>>()?
        .into_iter()
        .sum::<i128>();
    p["investor_refunds"] = json!(refund.refunds);
    p["refunded_minor"] = json!(returned.to_string());
    apply(w, ops, &refund.operations)?;
    *ownership = complete_project_ownership(w, p)?;
    ownership["asset"]["capital_value_minor"] = json!((money(&p["funded_minor"])?.0
        - i64::try_from(returned).map_err(|_| error("Refund overflow"))?)
    .to_string());
    p["shares"] = ownership["shares"].clone();
    Ok(())
}
