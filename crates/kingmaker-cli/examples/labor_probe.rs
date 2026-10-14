//! Read-only full source → RAM diagnostic labor and monetary opening, not a full week.
use kingmaker_core::{budget::BudgetRequest, labor::LaborRequest, WorldState};
use kingmaker_io::{runtime_config::default_runtime_path, RuntimeConfig};
use kingmaker_sim::{budget_cycle::execute_budget_cycle, labor_cycle::execute_local_labor};
use serde_json::json;
use std::{collections::BTreeMap, time::Instant};
fn money(w: &WorldState) -> BTreeMap<String, String> {
    let mut m = BTreeMap::<String, i128>::new();
    for a in &w.accounts {
        *m.entry(serde_json::to_string(&a.currency_id).unwrap())
            .or_default() += a.balance.0 as i128;
    }
    m.into_iter().map(|(k, v)| (k, v.to_string())).collect()
}
fn main() -> anyhow::Result<()> {
    eprintln!("Загрузка полного исходного мира в RAM (база только для чтения).");
    let config = RuntimeConfig::load(&default_runtime_path())?;
    let start = Instant::now();
    let mut w = config.load_world()?;
    let load_ms = start.elapsed().as_millis();
    let source = w.campaign.clone();
    let initial = json!({"pops":w.pops.len(),"people":w.pops.iter().map(|p|p.size as u64).sum::<u64>(),"workforce":w.pops.iter().map(|p|p.workforce).sum::<f64>(),"employed":w.pops.iter().map(|p|p.employed).sum::<f64>(),"jobs":w.budget_data.jobs.len(),"employment":w.budget_data.employment.len(),"money_by_currency":money(&w)});
    w.campaign.campaign_id = format!("labor-probe-{}", std::process::id());
    let r = LaborRequest {
        expected_revision: w.campaign.revision,
        period: "probe-opening".into(),
        policies: w.production_data.policies.clone(),
    };
    eprintln!(
        "Мир загружен за {:.1} с. Расчёт вместимости, освобождения и местного найма.",
        load_ms as f64 / 1000.
    );
    let start = Instant::now();
    let result = execute_local_labor(&mut w, &r);
    let labor_ms = start.elapsed().as_millis();
    let labor = match result {
        Ok(receipt) => {
            let mut reasons = BTreeMap::<String, usize>::new();
            for b in &receipt.blocked {
                *reasons
                    .entry(b["reason"].as_str().unwrap_or("unspecified").into())
                    .or_default() += 1;
            }
            let examples: Vec<_> = receipt
                .released
                .iter()
                .filter(|e| {
                    e["reason"] == "profession_mismatch"
                        || e["reason"] == "stale_workplace_reference"
                })
                .take(12)
                .collect();
            json!({"status":"passed","ms":labor_ms,"hired_fte":receipt.hired_fte,"released_fte":receipt.released_fte,"hires":receipt.hires.len(),"operations":receipt.operations.len(),"release_examples":examples,"blocked_reasons":reasons})
        }
        Err(e) => {
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &json!({"status":"failed","phase":"local_labor","error":e.to_string(),"initial":initial,"source_header":source,"load_ms":load_ms,"labor_ms":labor_ms,"source_writes":0,"full_world_weeks_executed":0})
                )?
            );
            anyhow::bail!("Full-source local labor rejected: {e}")
        }
    };
    eprintln!(
        "Местный труд завершён за {:.1} с. Бюджет и выплаты по актуальным назначениям.",
        labor_ms as f64 / 1000.
    );
    let start = Instant::now();
    let b = BudgetRequest {
        expected_revision: w.campaign.revision,
        period: r.period,
        policies: w.production_data.policies.clone(),
    };
    let result = execute_budget_cycle(&mut w, &b);
    let budget_ms = start.elapsed().as_millis();
    let (budget, failed) = match result {
        Ok(p) => (
            json!({"status":"passed","ms":budget_ms,"operations":p.operations.len(),"issuance":p.issuance_by_currency}),
            false,
        ),
        Err(e) => (
            json!({"status":"failed","ms":budget_ms,"error":e.to_string()}),
            true,
        ),
    };
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"mode":"isolated_full_source_ram_labor_budget","source_header":source,"run_header":w.campaign,"initial":initial,"final":{"people":w.pops.iter().map(|p|p.size as u64).sum::<u64>(),"workforce":w.pops.iter().map(|p|p.workforce).sum::<f64>(),"employed":w.pops.iter().map(|p|p.employed).sum::<f64>(),"money_by_currency":money(&w)},"load_ms":load_ms,"labor":labor,"budget":budget,"source_writes":0,"full_world_weeks_executed":0})
        )?
    );
    if failed {
        anyhow::bail!("Full-source budget rejected after local labor")
    };
    Ok(())
}
