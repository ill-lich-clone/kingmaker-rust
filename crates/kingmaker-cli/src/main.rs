use kingmaker_io::{runtime_config::default_runtime_path, RuntimeConfig};
use serde_json::json;
use std::path::PathBuf;
mod runner;

fn main() -> anyhow::Result<()> {
    let mut runtime = default_runtime_path();
    let mut inspect = false;
    let mut profile = false;
    let mut weeks = None;
    let mut output = None;
    let mut resume = None;
    let mut stop_file = None;
    let mut verify_save = false;
    let mut verification_probe = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--runtime" => {
                runtime = PathBuf::from(
                    args.next()
                        .ok_or_else(|| anyhow::anyhow!("--runtime requires a path"))?,
                )
            }
            "--inspect" => inspect = true,
            "--profile" => profile = true,
            "--verify-save" => verify_save = true,
            "--verify-continuation" => {
                verification_probe = Some(PathBuf::from(args.next().ok_or_else(|| {
                    anyhow::anyhow!("--verify-continuation requires a checkpoint")
                })?))
            }
            "--run-weeks" => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--run-weeks requires a positive integer"))?;
                let count = value
                    .parse::<usize>()
                    .map_err(|_| anyhow::anyhow!("--run-weeks requires a positive integer"))?;
                anyhow::ensure!(count > 0, "--run-weeks must be positive");
                weeks = Some(count);
            }
            "--output" => {
                output =
                    Some(PathBuf::from(args.next().ok_or_else(|| {
                        anyhow::anyhow!("--output requires a path")
                    })?))
            }
            "--resume" => {
                resume =
                    Some(PathBuf::from(args.next().ok_or_else(|| {
                        anyhow::anyhow!("--resume requires a checkpoint")
                    })?))
            }
            "--stop-file" => {
                stop_file =
                    Some(PathBuf::from(args.next().ok_or_else(|| {
                        anyhow::anyhow!("--stop-file requires a path")
                    })?))
            }
            "--help" | "-h" => {
                println!("kingmaker-cli --runtime <runtime.json> --inspect\nkingmaker-cli --runtime <runtime.json> --run-weeks X --output <new-directory> [--verify-save] [--profile]\nkingmaker-cli --resume <checkpoint-directory> --run-weeks X --output <new-directory>\nkingmaker-cli --verify-continuation <checkpoint-directory> --output <new-directory>\nSTOP in the output directory requests cancellation between phases; --stop-file selects a separate path. --verify-save compares one extra verification week in a new process. Measured full-world admission requires the initial audit.");
                return Ok(());
            }
            _ => anyhow::bail!("Unknown argument: {arg}"),
        }
    }
    if let Some(checkpoint) = verification_probe {
        anyhow::ensure!(
            weeks.is_none() && !inspect && resume.is_none() && !verify_save && !profile,
            "Continuation probe cannot combine another mode"
        );
        let output =
            output.ok_or_else(|| anyhow::anyhow!("--verify-continuation requires --output"))?;
        return runner::continuation_probe(&checkpoint, &output);
    }
    if let Some(weeks) = weeks {
        anyhow::ensure!(!inspect, "--inspect cannot be combined with --run-weeks");
        let output = output.ok_or_else(|| anyhow::anyhow!("--run-weeks requires --output"))?;
        anyhow::ensure!(!output.exists(), "Output directory already exists");
        return runner::run(
            &runtime,
            resume.as_deref(),
            &output,
            weeks,
            stop_file.as_deref(),
            verify_save,
            profile,
        );
    }
    anyhow::ensure!(
        resume.is_none() && output.is_none() && stop_file.is_none() && !verify_save && !profile,
        "Runner options require --run-weeks"
    );
    if !inspect {
        anyhow::bail!("EXPLICIT_RUN_REQUIRED: укажите --run-weeks X --output <новая директория>. Состояние и дата источника не изменены.");
    }
    let config = RuntimeConfig::load(&runtime)?;
    let start = std::time::Instant::now();
    let world = config.load_world()?;
    let validation = world.validate_links();
    let production_catalog_validation =
        match kingmaker_sim::production_validation::validate_production_catalog(&world) {
            Ok(()) => json!({"status":"passed"}),
            Err(e) => json!({"status":"failed","reason":e.to_string()}),
        };
    let personal_accounts = world.pops.iter().filter(|p| p.account_id.is_some()).count();
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "mode":"read_only","native_week_ready":true,"native_ram_week_available":true,"native_checkpoint_available":true,"native_measured_runner_available":true,"source_admission":"requires initial audit before a measured series","full_world_economic_health":"not_measured","native_ram_week_entry":"kingmaker_sim::native_week::execute_native_week","missing_phases":[],
            "native_isolated_production":true,"isolated_production_entry":"kingmaker_sim::production::execute_production_cycle",
            "native_isolated_market":true,"isolated_market_entry":"kingmaker_sim::market_cycle::execute_market_phase","market_stages":["maintenance","early_food","closing_market"],
            "native_isolated_budget":true,"isolated_budget_entry":"kingmaker_sim::budget_cycle::execute_budget_cycle",
            "native_isolated_local_labor":true,"isolated_local_labor_entry":"kingmaker_sim::labor_cycle::execute_local_labor","native_professions_passenger_migration":true,"isolated_mobility_entry":"kingmaker_sim::mobility_cycle::execute_mobility_opening",
            "income_data_counts":{"producer_income":world.income_data.producer_income.len()},
            "construction_data_counts":{"investment_opportunities":world.construction_data.investment_opportunities.len(),"catalog":world.construction_data.construction_catalog.len(),"catalog_by_currency_present":world.construction_data.construction_catalog_by_currency.is_some(),"site_grants":world.construction_data.building_site_grants.len(),"projects":world.production_data.construction_projects.len(),"completed":world.production_data.completed_construction.len(),"rgo_expansions":world.production_data.expansions.len()},
            "transport_data_counts":{"assets":world.transport_data.transport_assets.len(),"terminals":world.transport_data.transport_terminals.len(),"routes":world.transport_data.supply_routes.len(),"reservations":world.transport_data.route_reservations.len(),"navigation_provinces":world.transport_data.navigation_geography.len(),"automatic_land_paths_present":world.transport_data.automatic_land_paths.is_some()},
            "mobility_data_counts":{"profession_reference_present":world.mobility_data.reference.is_some(),"capitalist_entities":world.mobility_data.capitalist_entities.len(),"project_investments":world.mobility_data.project_investments.len(),"production_beneficiaries":world.mobility_data.production_beneficiaries.len(),"factory_beneficiaries":world.mobility_data.factory_beneficiaries.len(),"shipments":world.mobility_data.shipments.len(),"fractional_workforce_pops":world.pops.iter().filter(|p|p.workforce.fract()!=0.).count()},
            "budget_data_counts":{"employment":world.budget_data.employment.len(),"jobs":world.budget_data.jobs.len(),"adventurer_rates":world.budget_data.adventurer_income_rates.len(),"native_phase_receipts":world.budget_data.phase_receipts.len()},
            "market_data_counts":{"profile_refs":world.market_data.needs.as_ref().map(|n|n.profile_refs.len()),"need_profiles":world.market_data.needs.as_ref().map(|n|n.profiles.len()),"territory":world.market_data.territory.len(),"maintenance_agreements":world.market_data.maintenance_agreements.len(),"sale_offers":world.market_data.sale_offers.len(),"native_phase_receipts":world.market_data.phase_receipts.len()},
            "campaign":world.campaign,"load_elapsed_ms":start.elapsed().as_millis(),
            "requested":{"db_path":config.db_path,"campaign_id":config.campaign_id,"storage":config.storage,
                "workers":config.workers,"maxHeapMiB":config.max_heap_mib,"native_heap_limit":"not_implemented"},
            "counts":{"provinces":world.provinces.len(),"pops":world.pops.len(),
                "people":world.pops.iter().map(|p|u64::from(p.size)).sum::<u64>(),"accounts":world.accounts.len(),
                "personal_account_bindings":personal_accounts,"personal_account_absent_in_source":world.pops.len()-personal_accounts,
                "enterprises":world.enterprises.len(),"stockpiles":world.stockpiles.len(),
                "source_records":world.source.as_ref().map(|s|s.record_count()).transpose()?},
            "validation":validation,"full_world_weeks_executed":0,
            "production_catalog_validation":production_catalog_validation,
        }))?
    );
    Ok(())
}
