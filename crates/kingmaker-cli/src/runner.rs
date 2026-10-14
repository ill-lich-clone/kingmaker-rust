//! One process, one private RAM campaign. Never writes the selected runtime or source database.
use anyhow::{ensure, Context, Result};
use kingmaker_core::{week::NativeWeekRequest, WorldState};
use kingmaker_core::diagnostics::{self as profile, Outcome, Visibility, ProfileSession};
use kingmaker_io::{
    checkpoint::{load_checkpoint, save_checkpoint, save_checkpoint_controlled, CheckpointIdentity},
    RuntimeConfig,
};
use kingmaker_sim::{
    native_week::{apply_native_week, try_prepare_native_week},
    operation_session::OperationError,
    week_metrics::{collect_week_metrics, freeze_week_summary},
    world_audit::audit_initial_world,
};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{BufWriter, Read, Write},
    path::Path,
    sync::{mpsc, Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

pub fn native_fingerprint() -> Result<String> {
    let mut file = File::open(std::env::current_exe()?)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
}
struct Heartbeat {
    state: Arc<Mutex<(usize, String, Instant)>>,
    shutdown: mpsc::Sender<()>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Heartbeat {
    fn new(total: usize) -> Self {
        let state = Arc::new(Mutex::new((0, "загрузка".into(), Instant::now())));
        let clone = state.clone();
        let (tx, rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            while rx.recv_timeout(Duration::from_secs(30)) == Err(mpsc::RecvTimeoutError::Timeout) {
                let (week, phase, clock) = &*clone.lock().unwrap();
                println!(
                    "Неделя {week}/{total}; фаза {phase}; прошло {:.1} с",
                    clock.elapsed().as_secs_f64()
                );
                let _ = std::io::stdout().flush();
            }
        });
        Self {
            state,
            shutdown: tx,
            worker: Some(worker),
        }
    }
    fn phase(&self, week: usize, label: &str) {
        *self.state.lock().unwrap() = (week, label.into(), Instant::now());
        println!("Неделя {week}: {label}");
        let _ = std::io::stdout().flush();
    }
}
impl Drop for Heartbeat {
    fn drop(&mut self) {
        let _ = self.shutdown.send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut file = BufWriter::new(File::create(path)?);
    serde_json::to_writer_pretty(&mut file, value)?;
    file.flush()?;
    file.get_ref().sync_all()?;
    Ok(())
}
fn status(out: &Path, value: &Value) -> Result<()> {
    let pending = out.join("run.pending.json");
    write_json(&pending, value)?;
    fs::rename(pending, out.join("run.json"))?;
    Ok(())
}
fn csv(value: &Value) -> String {
    let text = match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        _ => value.to_string(),
    };
    format!("\"{}\"", text.replace('"', "\"\""))
}
fn validate_history(w: &WorldState) -> Result<()> {
    for p in &w.week_data.phase_receipts {
        ensure!(
            kingmaker_sim::week_history::validate_week_history(w, &p.request)?.as_ref() == Some(p),
            "Required weekly history altered"
        );
    }
    Ok(())
}
pub fn run(
    runtime: &Path,
    resume: Option<&Path>,
    out: &Path,
    total: usize,
    stop: Option<&Path>,
    verify_save: bool,
    profile_enabled: bool,
) -> Result<()> {
    ensure!(!out.exists(), "Output directory already exists");
    let parent = out
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    fs::create_dir(out)?;
    let session = if profile_enabled { Some(ProfileSession::install(Box::new(std::fs::OpenOptions::new().write(true).create_new(true).open(out.join("profile.jsonl"))?))?) } else { None };
    let outcome = profile::measure("run.total", Visibility::Boundary, || run_inner(runtime,resume,out,total,stop,verify_save,session.as_ref()));
    if let Some(session)=session {
        let profile_result=session.finish();
        let profile_status=json!({"complete":profile_result.is_ok(),"error":profile_result.as_ref().err().map(ToString::to_string),"calculation_succeeded":outcome.is_ok()});
        write_json(&out.join("profile-status.json"),&profile_status)?;
        if let Err(e)=profile_result {
            if let Ok(bytes)=fs::read(out.join("run.json")) { if let Ok(mut run)=serde_json::from_slice::<Value>(&bytes) {
                run["profile_status"]=json!("failed");run["profile_error"]=json!(e.to_string());
                if run["status"]=="completed" {run["status"]=json!("failed_profile");}
                status(out,&run)?;
            }}
            return match outcome {Err(original)=>Err(original.context(format!("Profile also failed: {e}"))),Ok(())=>Err(e.into())};
        }
    }
    outcome
}
fn run_inner(runtime:&Path,resume:Option<&Path>,out:&Path,total:usize,stop:Option<&Path>,verify_save:bool,profiler:Option<&ProfileSession>)->Result<()> {
    let heartbeat = Heartbeat::new(total);
    let load_clock = Instant::now();
    let native = profile::measure("run.identity",Visibility::Boundary,native_fingerprint)?;
    let load_scope=profile::scope("run.load",Visibility::Boundary);
    let (mut world, identity, configuration) = if let Some(checkpoint) = resume {
        let (w, manifest) =
            load_checkpoint(checkpoint).context("Load independent resume checkpoint")?;
        ensure!(
            manifest.identity.native_fingerprint == native,
            "Checkpoint native executable differs; no silent alpha migration"
        );
        (
            w,
            manifest.identity,
            json!({"resume":checkpoint,"source_database_read":false,"workers":1,"economy":"memory"}),
        )
    } else {
        let config = RuntimeConfig::load(runtime)?;
        ensure!(
            config.storage == "memory" && config.workers == 1,
            "Measured native runner requires configured memory economy and workers=1"
        );
        let mut w = config.load_world()?;
        let source_campaign = w.campaign.campaign_id.clone();
        ensure!(
            w.week_data.phase_receipts.is_empty(),
            "A source with native history must be resumed from its independent checkpoint"
        );
        w.campaign.campaign_id =
            format!("rust-test-{}-{}", std::process::id(), w.campaign.revision);
        let identity = CheckpointIdentity {
            native_fingerprint: native,
            engine_hash: w.campaign.engine_hash.clone(),
            rules_hash: w.campaign.rules_hash.clone(),
            source_hash: w.campaign.source_hash.clone(),
        };
        let parameters = json!({"runtime":runtime,"db_path":config.db_path,"source_campaign":source_campaign,"workers":config.workers,"economy":config.storage,"maxHeapMiB":config.max_heap_mib,"native_heap_limit":"не реализовано","source_writes":0});
        (w, identity, parameters)
    };
    load_scope.finish(Outcome::Ok);
    let load_ms = load_clock.elapsed().as_millis();
    validate_history(&world)?;
    heartbeat.phase(0, "исходный аудит");
    let audit_scope=profile::scope("run.initial_audit",Visibility::Boundary);
    let audit = audit_initial_world(&world);
    audit_scope.finish(Outcome::Ok);
    write_json(&out.join("initial-audit.json"), &audit)?;
    let mut run = json!({"schema":1,"status":"initializing","target_weeks":total,"completed_weeks":0,"configuration":configuration,"identity":identity,"initial_campaign":world.campaign,"load_elapsed_ms":load_ms,"initial_snapshot":"initial","automatic_systems":["employment/mobility","existing budget funding","capitalist construction","artisan capital/methods","production/market/supply","raw state procurement","income/history"],"player_decisions":"no synthetic orders; existing policies only","native_heap_limit":"не реализовано"});
    status(out, &run)?;
    let outcome = (|| -> Result<()> {
        let stop=stop.map(Path::to_path_buf).unwrap_or_else(||out.join("STOP"));
        // An already requested stop still owes the operator a recoverable world.
        // This recovery save is intentional; ordinary in-flight saves are cancellable.
        if stop.exists() {
            let save_clock=Instant::now();
            save_checkpoint(&world,&out.join("cancelled"),&identity)?;
            run["status"]=json!("cancelled");run["stop_file"]=json!(stop);
            run["last_recoverable_checkpoint"]=json!("cancelled");
            run["end_campaign"]=json!(world.campaign);
            run["final_save_elapsed_ms"]=json!(save_clock.elapsed().as_millis());
            status(out,&run)?;return Ok(());
        }
        let checkpoint_control=kingmaker_sim::run_control::RunControl::from_stop_file(stop.clone());
        heartbeat.phase(0, "начальный снимок");
        let save_clock = Instant::now();
        let saved = profile::measure("run.initial_save",Visibility::Boundary,||save_checkpoint_controlled(&world, &out.join("initial"), &identity,&|label|checkpoint_control.check(label).map_err(anyhow::Error::from)));
        run["initial_save_elapsed_ms"] = json!(save_clock.elapsed().as_millis());
        if let Err(e) = saved {
            run["status"] = json!("failed_initial_snapshot");
            run["error"] = json!(e.to_string());
            status(out, &run)?;
            return Err(e);
        }
        run["last_recoverable_checkpoint"] = json!("initial");
        if !audit.can_run() {
            run["status"] = json!("failed_initial_data");
            run["failed_phase"] = json!("initial_audit");
            run["error"] = json!("Initial audit rejects measured series; see initial-audit.json");
            status(out, &run)?;
            anyhow::bail!("Initial data audit failed; no weeks executed");
        }

        run["stop_file"] = json!(stop);
        run["status"] = json!("running");
        status(out, &run)?;
        let mut jsonl = BufWriter::new(
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(out.join("weeks.jsonl"))?,
        );
        let mut table = BufWriter::new(
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(out.join("metrics.csv"))?,
        );
        writeln!(
        table,
        "week,period,scope,name,goods_id,currency_id,unit,production_kind,purpose,status,amount"
    )?;
        for ordinal in 1..=total {
            if profiler.is_some_and(|p| !p.is_healthy()) { break; }
            let week_scope=profile::scope("run.week",Visibility::Boundary);
            let pipeline_clock=Instant::now();
            let request = week_request(&world);
            run["active_request"] = json!(request);
            let before = profile::measure("run.summary",Visibility::Boundary,||freeze_week_summary(&world))?;
            let turn_clock = Instant::now();
            let mut cancelled = false;
            let mut last_phase = String::new();
            let control=kingmaker_sim::run_control::RunControl::from_stop_file(stop.to_path_buf());
            let prepared = profile::measure("run.prepare",Visibility::Boundary,||kingmaker_sim::native_week::try_prepare_native_week_controlled(&world, &request, &control, |phase| {
                last_phase = phase.into();
                heartbeat.phase(ordinal, phase);
                if stop.exists() {
                    cancelled = true;
                    Err(OperationError("CANCELLED_BY_OPERATOR".into()))
                } else {
                    Ok(())
                }
            }));
            let prepared = match prepared {
                Ok(p) => p,
                Err(e) => {
                    cancelled |= control.is_cancelled();
                    let label = if cancelled { "cancelled" } else { "failed" };
                    run["status"] = json!(label);
                    run["failed_phase"] = json!(last_phase);
                    run["error"] = json!(e.to_string());
                    run["repro_request"] = json!(request);
                    run["end_campaign"] = json!(world.campaign);
                    heartbeat.phase(ordinal, "сохранение последнего завершённого мира");
                    let save = Instant::now();
                    if let Err(save_error) = save_checkpoint(&world, &out.join(label), &identity) {
                        run["error_artifact_error"] = json!(save_error.to_string());
                        if cancelled {
                            run["status"] = json!("failed_snapshot");
                            run["calculation_status"] = json!("cancelled");
                            run["error"] = json!(save_error.to_string());
                            return Err(save_error);
                        }
                        return Err(e.into());
                    }
                    run["last_recoverable_checkpoint"] = json!(label);
                    run["final_save_elapsed_ms"] = json!(save.elapsed().as_millis());
                    status(out, &run)?;
                    if cancelled {
                        println!(
                            "Остановлено: готово {}/{} недель; незавершённая неделя отброшена",
                            ordinal - 1,
                            total
                        );
                        return Ok(());
                    }
                    return Err(e.into());
                }
            };
            let compute_ms = turn_clock.elapsed().as_millis();
            let metrics_clock = Instant::now();
            let (preview, receipt) = prepared
                .preview(&world)
                .context("Measured runner received replay instead of a new week")?;
            let metrics = match profile::measure("run.metrics",Visibility::Boundary,||collect_week_metrics(&before, &preview, receipt)) {
                Ok(m) => m,
                Err(e) => {
                    run["status"] = json!("failed_diagnostics");
                    run["failed_phase"] = json!("metrics");
                    run["error"] = json!(e.to_string());
                    run["repro_request"] = json!(request);
                    if let Err(save_error) = save_checkpoint(&world, &out.join("failed"), &identity)
                    {
                        run["error_artifact_error"] = json!(save_error.to_string());
                        return Err(e.into());
                    }
                    run["last_recoverable_checkpoint"] = json!("failed");
                    status(out, &run)?;
                    return Err(e.into());
                }
            };
            let metrics_ms = metrics_clock.elapsed().as_millis();
            let publish_clock = Instant::now();
            let receipt = profile::measure("run.apply",Visibility::Boundary,||apply_native_week(&mut world, prepared))?;
            let elapsed_ms = compute_ms + publish_clock.elapsed().as_millis();
            run["completed_weeks"] = json!(ordinal);
            run["end_campaign"] = json!(world.campaign);
            let report_scope=profile::scope("run.week_report",Visibility::Boundary);
            serde_json::to_writer(
                &mut jsonl,
                &json!({"week":ordinal,"elapsed_ms":elapsed_ms,"metrics_elapsed_ms":metrics_ms,"phase_timings":receipt.timings,"metrics":metrics}),
            )?;
            writeln!(jsonl)?;
            jsonl.flush()?;
            for row in &metrics.rows {
                let values = [
                    json!(ordinal),
                    json!(request.period),
                    row["scope"].clone(),
                    row["name"].clone(),
                    row["goods_id"].clone(),
                    row["currency_id"].clone(),
                    row["unit"].clone(),
                    row["production_kind"].clone(),
                    row["purpose"].clone(),
                    row["status"].clone(),
                    row["amount"].clone(),
                ];
                writeln!(
                    table,
                    "{}",
                    values.iter().map(csv).collect::<Vec<_>>().join(",")
                )?;
            }
            table.flush()?;
            run["completed_weeks"] = json!(ordinal);
            run["end_campaign"] = json!(world.campaign);
            run["last_week_elapsed_ms"] = json!(elapsed_ms);
            run["last_metrics_elapsed_ms"] = json!(metrics_ms);
            status(out, &run)?;
            run["last_week_pipeline_ms"]=json!(pipeline_clock.elapsed().as_millis());
            report_scope.finish(Outcome::Ok);week_scope.finish(Outcome::Ok);
            status(out,&run)?;
            println!("Готово {ordinal}/{total} недель; время хода {:.3} с; показатели {:.3} с; дата {}-{:02}-{:02}; revision {}",elapsed_ms as f64/1000.,metrics_ms as f64/1000.,world.campaign.year,world.campaign.month,world.campaign.day,world.campaign.revision);
            if profiler.is_some_and(|p|!p.is_healthy()) {break;}
        }
        heartbeat.phase(total, "конечный снимок");
        run["status"] = json!("saving_final");
        status(out, &run)?;
        let save_clock = Instant::now();
        profile::measure("run.final_save",Visibility::Boundary,||save_checkpoint_controlled(&world, &out.join("final"), &identity,&|label|checkpoint_control.check(label).map_err(anyhow::Error::from)))?;
        let final_save_ms = save_clock.elapsed().as_millis();
        run["final_save_elapsed_ms"] = json!(final_save_ms);
        run["last_recoverable_checkpoint"] = json!("final");
        run["status"] = json!(if verify_save {
            "verifying_continuation"
        } else {
            "completed"
        });
        run["new_process_continuation"] = json!("not_verified");
        status(out, &run)?;
        anyhow::ensure!(!profiler.is_some_and(|p|!p.is_healthy()),"Profile failed; completed state saved, remaining weeks not executed");
        if verify_save {
            heartbeat.phase(total, "контроль непрерывного продолжения");
            let clock = Instant::now();
            let request = week_request(&world);
            run["active_request"] = json!(request);
            run["continuous_verification_weeks"] = json!(0);
            let control=kingmaker_sim::run_control::RunControl::from_stop_file(stop.to_path_buf());
            let prepared = kingmaker_sim::native_week::try_prepare_native_week_controlled(&world, &request, &control, |phase| {
                heartbeat.phase(total, phase);
                Ok(())
            })?;
            apply_native_week(&mut world, prepared)?;
            run["continuous_verification_weeks"] = json!(1);
            let continuous_ms = clock.elapsed().as_millis();
            heartbeat.phase(total, "загрузка и продолжение в новом процессе");
            let child_clock = Instant::now();
            let child_dir = out.join("restored-continuation");
            let log = File::create(out.join("continuation-process.log"))?;
            let child = std::process::Command::new(std::env::current_exe()?)
                .arg("--verify-continuation")
                .arg(out.join("final"))
                .arg("--output")
                .arg(&child_dir)
                .stdout(log.try_clone()?)
                .stderr(log)
                .status()?;
            let verify = if child.success() {
                kingmaker_io::economic_comparison::compare_economic_state(
                    &world,
                    &child_dir.join("economic.stream"),
                )
            } else {
                Err(anyhow::anyhow!(
                    "Continuation subprocess failed; see continuation-process.log"
                ))
            };
            run["new_process_continuation"] = json!({"status":if verify.is_ok(){"passed"}else{"failed"},"additional_verification_weeks":1,"continuous_elapsed_ms":continuous_ms,"new_process_elapsed_ms":child_clock.elapsed().as_millis(),"comparison":"complete native economics/mandatory history/raw Source; only week timings excluded"});
            if let Err(e) = verify {
                if let Err(artifact_error) = kingmaker_io::economic_comparison::save_economic_state(
                    &world,
                    &out.join("continuous-failure.stream"),
                ) {
                    run["error_artifact_error"] = json!(artifact_error.to_string());
                }
                run["status"] = json!("failed_continuation");
                run["error"] = json!(e.to_string());
                status(out, &run)?;
                return Err(e);
            }
            run["status"] = json!("completed");
            status(out, &run)?;
            println!("Снимок проверен: новое продолжение совпало с непрерывным по экономике, истории и исходным данным");
        }
        println!(
            "Сохранение отдельно: {:.3} с; результаты {}",
            final_save_ms as f64 / 1000.,
            out.display()
        );
        Ok(())
    })();
    if let Err(error) = outcome {
        let previous = run["status"].as_str().unwrap_or("unknown").to_owned();
        let failed = if previous == "verifying_continuation" {
            "failed_continuation"
        } else if previous == "saving_final" {
            "failed_snapshot"
        } else if previous.starts_with("failed") {
            previous.as_str()
        } else {
            "failed"
        };
        run["status"] = json!(failed);
        if run["error"].is_null() {
            run["error"] = json!(error.to_string());
        }
        if run["failed_phase"].is_null() {
            run["failed_phase"] = json!(heartbeat.state.lock().unwrap().1);
        }
        if run["repro_request"].is_null() {
            run["repro_request"] = run["active_request"].clone();
        }
        if failed == "failed_continuation" {
            if !run["new_process_continuation"].is_object() {
                run["new_process_continuation"] = json!({"status":"failed","additional_verification_weeks":run["continuous_verification_weeks"]});
            } else {
                run["new_process_continuation"]["status"] = json!("failed");
            }
        }
        if let Err(status_error) = status(out, &run) {
            return Err(error.context(format!(
                "Also unable to persist terminal status: {status_error}"
            )));
        }
        return Err(error);
    }
    Ok(())
}
fn week_request(w: &WorldState) -> NativeWeekRequest {
    NativeWeekRequest {
        expected_revision: w.campaign.revision,
        period: format!(
            "native-week-{}-{:02}-{:02}-{}",
            w.campaign.year, w.campaign.month, w.campaign.day, w.campaign.revision
        ),
        ..Default::default()
    }
}
pub fn continuation_probe(checkpoint: &Path, out: &Path) -> Result<()> {
    ensure!(!out.exists(), "Output directory already exists");
    let parent = out
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    fs::create_dir(out)?;
    let heartbeat = Heartbeat::new(1);
    let load = Instant::now();
    let (mut w, manifest) = load_checkpoint(checkpoint)?;
    ensure!(
        manifest.identity.native_fingerprint == native_fingerprint()?,
        "Checkpoint native executable differs"
    );
    validate_history(&w)?;
    let load_ms = load.elapsed().as_millis();
    let request = week_request(&w);
    let week = Instant::now();
    kingmaker_sim::native_week::execute_native_week(&mut w, &request, |phase| {
        heartbeat.phase(1, phase)
    })?;
    let week_ms = week.elapsed().as_millis();
    heartbeat.phase(1, "сохранение контрольного продолжения");
    let save = Instant::now();
    save_checkpoint(&w, &out.join("final"), &manifest.identity)?;
    let save_ms = save.elapsed().as_millis();
    heartbeat.phase(1, "потоковое сравнение");
    kingmaker_io::economic_comparison::save_economic_state(&w, &out.join("economic.stream"))?;
    status(
        out,
        &json!({"status":"completed_probe","additional_verification_weeks":1,"load_elapsed_ms":load_ms,"week_elapsed_ms":week_ms,"save_elapsed_ms":save_ms,"end_campaign":w.campaign,"source_writes":0}),
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "profile_tests.rs"]
mod profile_tests;
