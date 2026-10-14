//! Isolated RAM monetary opening, never a week or a write to the source DB.
use kingmaker_core::budget::BudgetRequest;
use kingmaker_io::{runtime_config::default_runtime_path, RuntimeConfig};
use kingmaker_sim::budget_cycle::execute_budget_cycle;
use serde_json::json;
fn main() -> anyhow::Result<()> {
    let config = RuntimeConfig::load(&default_runtime_path())?;
    let start = std::time::Instant::now();
    let mut world = config.load_world()?;
    let load_ms = start.elapsed().as_millis();
    let source_header = world.campaign.clone();
    let counts = json!({"employment":world.budget_data.employment.len(),"jobs":world.budget_data.jobs.len(),"adventurer_rates":world.budget_data.adventurer_income_rates.len(),"pops":world.pops.len(),"source_revision":world.campaign.revision});
    world.campaign.campaign_id = format!("budget-probe-{}", std::process::id());
    let request = BudgetRequest {
        expected_revision: world.campaign.revision,
        period: "probe-opening".into(),
        policies: world.production_data.policies.clone(),
    };
    let start = std::time::Instant::now();
    let result = execute_budget_cycle(&mut world, &request);
    let elapsed_ms = start.elapsed().as_millis();
    match result {
        Ok(receipt) => {
            let payroll = receipt
                .operations
                .iter()
                .filter(|op| op.kind == kingmaker_core::operations::OperationKind::Payroll)
                .count();
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &json!({"mode":"isolated_ram_budget_probe","source_header":source_header,"run_header":world.campaign,"counts":counts,"load_ms":load_ms,"budget_ms":elapsed_ms,"operations":receipt.operations.len(),"payroll":payroll,"issuance":receipt.issuance_by_currency,"full_world_weeks_executed":0,"source_writes":0,"status":"passed"})
                )?
            );
        }
        Err(error) => {
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &json!({"mode":"isolated_ram_budget_probe","source_header":source_header,"counts":counts,"load_ms":load_ms,"budget_ms":elapsed_ms,"status":"failed","error":error.to_string(),"source_writes":0,"full_world_weeks_executed":0})
                )?
            );
            anyhow::bail!("Isolated native budget rejected: {error}");
        }
    }
    Ok(())
}
