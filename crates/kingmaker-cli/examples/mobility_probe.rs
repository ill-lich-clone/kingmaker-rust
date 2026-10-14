//! Full source is read-only; native opening/budget in RAM, not an economic week.
use kingmaker_core::{budget::BudgetRequest, labor::LaborRequest, WorldState};
use kingmaker_io::{runtime_config::default_runtime_path, RuntimeConfig};
use kingmaker_sim::{budget_cycle::execute_budget_cycle, mobility_cycle::execute_mobility_opening};
use serde_json::json;
use std::{collections::BTreeMap, time::Instant};
fn cash(w: &WorldState) -> BTreeMap<String, String> {
    let mut rows = BTreeMap::<String, i128>::new();
    for a in &w.accounts {
        *rows
            .entry(serde_json::to_string(&a.currency_id).unwrap())
            .or_default() += i128::from(a.balance.0);
    }
    rows.into_iter()
        .map(|(id, n)| (id, n.to_string()))
        .collect()
}
fn summary(w: &WorldState) -> serde_json::Value {
    json!({"pops":w.pops.len(),"people":w.pops.iter().map(|p|u64::from(p.size)).sum::<u64>(),"workforce":w.pops.iter().map(|p|p.workforce).sum::<f64>(),"employed":w.pops.iter().map(|p|p.employed).sum::<f64>(),"stocks":w.stockpiles.len(),"accounts":w.accounts.len(),"cash_by_currency":cash(w),"link_errors":w.validate_links().errors.into_iter().take(20).collect::<Vec<_>>()})
}
fn main() -> anyhow::Result<()> {
    eprintln!("Загрузка полного мира в RAM; исходная база только для чтения.");
    let start = Instant::now();
    let config = RuntimeConfig::load(&default_runtime_path())?;
    let mut world = config.load_world()?;
    let load_ms = start.elapsed().as_millis();
    let header = world.campaign.clone();
    let before = summary(&world);
    world.campaign.campaign_id = format!("mobility-probe-{}", std::process::id());
    let request = LaborRequest {
        expected_revision: world.campaign.revision,
        period: "probe-mobility-opening".into(),
        policies: world.production_data.policies.clone(),
    };
    eprintln!(
        "Загрузка завершена за {:.1} с. Начало хода: прибытия, труд, профессии, отправления.",
        load_ms as f64 / 1000.
    );
    let start = Instant::now();
    let result = execute_mobility_opening(&mut world, &request);
    let mobility_ms = start.elapsed().as_millis();
    let receipt = match result {
        Ok(r) => r,
        Err(e) => {
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &json!({"status":"failed","phase":"mobility_opening","error":e.to_string(),"source_header":header,"initial":before,"current":summary(&world),"load_ms":load_ms,"mobility_ms":mobility_ms,"source_writes":0,"full_world_weeks_executed":0})
                )?
            );
            anyhow::bail!("Native full-source mobility rejected: {e}")
        }
    };
    let opening = json!({"status":"passed","ms":mobility_ms,"hired_fte":receipt.local.hired_fte,"released_fte":receipt.local.released_fte,"profession_workers":receipt.professions.changed_workforce,"departed_workers":receipt.departures.workforce,"arrived_workers":receipt.arrivals.workforce,"operations":receipt.operations.len(),"invariants":receipt.invariants,"profession_blocked":receipt.professions.blocked.iter().take(10).collect::<Vec<_>>(),"migration_blocked":receipt.departures.blocked.iter().take(10).collect::<Vec<_>>()});
    eprintln!(
        "Начало хода завершено за {:.1} с. Проверка бюджета по фактическим назначениям.",
        mobility_ms as f64 / 1000.
    );
    let start = Instant::now();
    let budget_request = BudgetRequest {
        expected_revision: world.campaign.revision,
        period: request.period,
        policies: world.production_data.policies.clone(),
    };
    let result = execute_budget_cycle(&mut world, &budget_request);
    let budget_ms = start.elapsed().as_millis();
    let failed = result.is_err();
    let budget = match result {
        Ok(r) => {
            json!({"status":"passed","ms":budget_ms,"operations":r.operations.len(),"issuance":r.issuance_by_currency})
        }
        Err(e) => json!({"status":"failed","ms":budget_ms,"error":e.to_string()}),
    };
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"mode":"isolated_full_source_ram_mobility_budget","source_header":header,"run_header":world.campaign,"initial":before,"final":summary(&world),"load_ms":load_ms,"opening":opening,"budget":budget,"source_writes":0,"full_world_weeks_executed":0})
        )?
    );
    if failed {
        anyhow::bail!("Native full-source budget rejected after mobility")
    }
    Ok(())
}
