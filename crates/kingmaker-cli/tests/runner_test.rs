#[path = "../../kingmaker-sim/tests/common/native_week.rs"]
mod common;
use kingmaker_core::*;
use kingmaker_io::checkpoint::{load_checkpoint, save_checkpoint, CheckpointIdentity};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    io::{BufRead, BufReader, Read},
    path::PathBuf,
    process::Command,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "kingmaker-runner-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}

#[test]
fn operator_stop_after_progress_preserves_every_completed_week_and_no_partial_next_one() {
    let temp = Temp::new();
    let start = start(&temp);
    let out = temp.0.join("mid-run");
    let mut child = Command::new(env!("CARGO_BIN_EXE_kingmaker-cli"))
        .args([
            "--resume",
            start.to_str().unwrap(),
            "--run-weeks",
            "20",
            "--output",
            out.to_str().unwrap(),
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut requested = false;
    for line in BufReader::new(stdout).lines() {
        let line = line.unwrap();
        if !requested && line.contains("Готово 1/20") {
            std::fs::write(out.join("STOP"), "stop").unwrap();
            requested = true;
        }
    }
    let result = child.wait_with_output().unwrap();
    assert!(requested);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let run: Value = serde_json::from_slice(&std::fs::read(out.join("run.json")).unwrap()).unwrap();
    assert_eq!(run["status"], "cancelled");
    let completed = run["completed_weeks"].as_u64().unwrap();
    assert!((1..20).contains(&completed));
    let (w, _) = load_checkpoint(&out.join("cancelled")).unwrap();
    assert_eq!(w.campaign.revision, 20 + completed);
    assert_eq!(w.week_data.phase_receipts.len(), completed as usize);
    assert_eq!(
        std::fs::read_to_string(out.join("weeks.jsonl"))
            .unwrap()
            .lines()
            .count(),
        completed as usize
    );
}
impl Drop for Temp {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!("Retained failing runner reproduction: {}", self.0.display());
            return;
        }
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
fn native_hash() -> String {
    let mut file = std::fs::File::open(env!("CARGO_BIN_EXE_kingmaker-cli")).unwrap();
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = file.read(&mut buffer).unwrap();
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    hash.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn obstruct_after_first_week(
    start: &std::path::Path,
    out: &std::path::Path,
    paths: &[&str],
    verify: bool,
) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_kingmaker-cli"));
    command.args([
        "--resume",
        start.to_str().unwrap(),
        "--run-weeks",
        "10",
        "--output",
        out.to_str().unwrap(),
    ]);
    if verify {
        command.arg("--verify-save");
    }
    let mut child = command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut obstructed = false;
    for line in BufReader::new(child.stdout.take().unwrap()).lines() {
        if line.unwrap().contains("Готово 1/10") && !obstructed {
            for path in paths {
                std::fs::create_dir(out.join(path)).unwrap();
            }
            obstructed = true;
        }
    }
    assert!(obstructed);
    child.wait_with_output().unwrap()
}

#[test]
fn failed_final_save_records_terminal_failure_and_last_recoverable_snapshot() {
    let temp = Temp::new();
    let start = start(&temp);
    let out = temp.0.join("save-fails");
    let result = obstruct_after_first_week(&start, &out, &["final"], false);
    assert!(!result.status.success());
    let run: Value = serde_json::from_slice(&std::fs::read(out.join("run.json")).unwrap()).unwrap();
    assert_eq!(run["status"], "failed_snapshot");
    assert_eq!(run["completed_weeks"], 10);
    assert_eq!(run["last_recoverable_checkpoint"], "initial");
    assert!(run["error"].as_str().is_some_and(|e| !e.is_empty()));
    load_checkpoint(&out.join("initial")).unwrap();
}

#[test]
fn failed_continuation_log_creation_records_terminal_failure_and_valid_final_snapshot() {
    let temp = Temp::new();
    let start = start(&temp);
    let out = temp.0.join("log-fails");
    let result = obstruct_after_first_week(&start, &out, &["continuation-process.log"], true);
    assert!(!result.status.success());
    let run: Value = serde_json::from_slice(&std::fs::read(out.join("run.json")).unwrap()).unwrap();
    assert_eq!(run["status"], "failed_continuation");
    assert_eq!(run["new_process_continuation"]["status"], "failed");
    assert_eq!(run["last_recoverable_checkpoint"], "final");
    assert!(run["error"].as_str().is_some_and(|e| !e.is_empty()));
    load_checkpoint(&out.join("final")).unwrap();
}

#[test]
fn failed_optional_repro_stream_does_not_hide_original_child_failure() {
    let temp = Temp::new();
    let start = start(&temp);
    let out = temp.0.join("evidence-fails");
    let result = obstruct_after_first_week(
        &start,
        &out,
        &["restored-continuation", "continuous-failure.stream"],
        true,
    );
    assert!(!result.status.success());
    let run: Value = serde_json::from_slice(&std::fs::read(out.join("run.json")).unwrap()).unwrap();
    assert_eq!(run["status"], "failed_continuation");
    assert!(run["error"].as_str().unwrap().contains("subprocess failed"));
    assert!(run["error_artifact_error"]
        .as_str()
        .is_some_and(|e| !e.is_empty()));
    assert_eq!(run["last_recoverable_checkpoint"], "final");
}

#[test]
fn actual_verification_week_overflow_records_failed_phase_and_request_after_successful_main_week() {
    let temp = Temp::new();
    let mut w = world();
    w.pops.push(PopRecord {
        id: PopId(200),
        external_id: "adventurer".into(),
        province_id: ProvinceId(1),
        state_id: Some("s".into()),
        currency_id: Some("COIN".into()),
        size: 1,
        workforce: 0.,
        pop_type_id: "adventurers".into(),
        pop_type: "adventurers".into(),
        ..Default::default()
    });
    w.accounts.push(AccountBalance {
        id: AccountId(200),
        external_id: "adv-cash".into(),
        owner_id: "adventurer".into(),
        currency_id: "COIN".into(),
        purpose: "personal_savings".into(),
        balance: Money(i64::MAX - 10),
    });
    w.budget_data.adventurer_income_rates.push(serde_json::from_value(json!({"id":"adv-rate","pop_id":"adventurer",
        "currency_id":"COIN","numerator":"10","denominator":"1","source":"explicit overflow miniature"})).unwrap());
    w.rebuild_account_index().unwrap();
    let start = start_world(&temp, &w);
    let out = temp.0.join("phase-fails");
    let result = Command::new(env!("CARGO_BIN_EXE_kingmaker-cli"))
        .args([
            "--resume",
            start.to_str().unwrap(),
            "--run-weeks",
            "1",
            "--output",
            out.to_str().unwrap(),
            "--verify-save",
        ])
        .output()
        .unwrap();
    assert!(!result.status.success());
    let run: Value = serde_json::from_slice(&std::fs::read(out.join("run.json")).unwrap()).unwrap();
    assert_eq!(
        run["completed_weeks"],
        1,
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(run["status"], "failed_continuation");
    assert_eq!(run["repro_request"]["expected_revision"], 21);
    assert!(run["failed_phase"].as_str().is_some_and(|p| !p.is_empty()));
    assert_eq!(run["last_recoverable_checkpoint"], "final");
    let (saved, _) = load_checkpoint(&out.join("final")).unwrap();
    assert_eq!(saved.campaign.revision, 21);
}
fn world() -> WorldState {
    let mut w = common::factory(false);
    add_raw(&mut w);
    attach_source(&mut w);
    w
}
fn add_raw(w: &mut WorldState) {
    for p in &mut w.provinces {
        p.goods_id = Some(Goods::Grain);
    }
    w.production_data.catalog.as_mut().unwrap().goods.push(
        serde_json::from_value(json!({"goods_id":"grain","kind":"raw_resource","unit":"t"}))
            .unwrap(),
    );
    w.production_data.catalog.as_mut().unwrap().methods.push(serde_json::from_value(json!({"method_id":"rgo:grain","kind":"rgo","status":"ready","output_goods_id":"grain","output_unit":"t","weekly_productivity":1,"staffing":["farmers"],"inputs":[],"capacity":{"base_people":100,"source":"explicit mini"}})).unwrap());
    let owner = w
        .production_data
        .factories
        .first()
        .and_then(|f| f.base.owner_id.clone())
        .or_else(|| {
            w.accounts
                .iter()
                .find(|a| a.purpose == "employer_operating_capital")
                .map(|a| a.owner_id.clone())
        })
        .unwrap();
    for p in &w.provinces {
        w.production_data.resource_operations.push(serde_json::from_value(json!({"id":format!("raw:{}",p.id.0),"owner_id":owner,"province_id":p.id.0,"method_id":"rgo:grain","resource_id":"grain","resource_status":"source","land_access":true,"assignments":[]})).unwrap());
    }
}
fn attach_source(w: &mut WorldState) {
    let mut source = SourceSnapshot::default();
    source.database_path = "absent-original.sqlite".into();
    source.tables.insert(
        "unknown-author-table".into(),
        vec![StoredRecord {
            table_name: "unknown-author-table".into(),
            id_key: "[\"number\",7]".into(),
            external_id_json: Some("7".into()),
            ordinal: 9,
            sequence: 11,
            payload: "{\"author_rule\":\"never replace\"}".into(),
        }],
    );
    w.source = Some(Arc::new(source));
}
fn start(temp: &Temp) -> PathBuf {
    let w = world();
    start_world(temp, &w)
}

#[test]
fn rejected_initial_data_reports_initial_audit_even_after_the_repro_snapshot() {
    let temp = Temp::new();
    let mut w = world();
    w.production_data.resource_operations.clear();
    let start = start_world(&temp, &w);
    let out = temp.0.join("invalid-initial");
    let result = Command::new(env!("CARGO_BIN_EXE_kingmaker-cli"))
        .args([
            "--resume",
            start.to_str().unwrap(),
            "--run-weeks",
            "1",
            "--output",
            out.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!result.status.success());
    let run: Value = serde_json::from_slice(&std::fs::read(out.join("run.json")).unwrap()).unwrap();
    assert_eq!(run["status"], "failed_initial_data");
    assert_eq!(run["failed_phase"], "initial_audit");
    assert_eq!(run["completed_weeks"], 0);
    let (snapshot, _) = load_checkpoint(&out.join("initial")).unwrap();
    assert_eq!(snapshot.campaign.revision, 20);
}
fn start_world(temp: &Temp, w: &WorldState) -> PathBuf {
    let p = temp.0.join("start");
    save_checkpoint(
        w,
        &p,
        &CheckpointIdentity {
            native_fingerprint: native_hash(),
            engine_hash: w.campaign.engine_hash.clone(),
            rules_hash: w.campaign.rules_hash.clone(),
            source_hash: w.campaign.source_hash.clone(),
        },
    )
    .unwrap();
    p
}

#[test]
fn new_process_preserves_dated_cargo_calendar_rollover_and_distinct_currency_identities() {
    let temp = Temp::new();
    let mut w = common::factory(true);
    add_raw(&mut w);
    attach_source(&mut w);
    w.campaign.month = 12;
    w.campaign.day = 28;
    for (ordinal, owner, currency) in [(700, json!(7), json!(5)), (701, json!("7"), json!("5"))] {
        w.accounts.push(AccountBalance {
            id: AccountId(ordinal),
            external_id: format!("distinct:{ordinal}").into(),
            owner_id: ExternalId::from_json(owner).unwrap(),
            currency_id: ExternalId::from_json(currency).unwrap(),
            purpose: "personal_savings".into(),
            balance: Money(99),
        });
    }
    w.rebuild_account_index().unwrap();
    let start = start_world(&temp, &w);
    let out = temp.0.join("remote-verify");
    let result = Command::new(env!("CARGO_BIN_EXE_kingmaker-cli"))
        .args([
            "--resume",
            start.to_str().unwrap(),
            "--run-weeks",
            "1",
            "--output",
            out.to_str().unwrap(),
            "--verify-save",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let (main, _) = load_checkpoint(&out.join("final")).unwrap();
    assert_eq!(
        (main.campaign.year, main.campaign.month, main.campaign.day),
        (307, 1, 5)
    );
    assert!(main
        .mobility_data
        .shipments
        .iter()
        .any(|s| s["status"] == "in_transit"));
    let (continued, _) = load_checkpoint(&out.join("restored-continuation/final")).unwrap();
    assert!(continued
        .mobility_data
        .shipments
        .iter()
        .any(|s| s["status"] == "arrived"));
    for currency in [
        ExternalId::from_json(json!(5)).unwrap(),
        ExternalId::from_json(json!("5")).unwrap(),
    ] {
        assert!(continued
            .accounts
            .iter()
            .any(|a| a.currency_id == currency && a.balance == Money(99)));
    }
}

#[test]
fn missing_required_history_refuses_new_process_before_another_week() {
    let temp = Temp::new();
    let start = start(&temp);
    let out = temp.0.join("one");
    let result = Command::new(env!("CARGO_BIN_EXE_kingmaker-cli"))
        .args([
            "--resume",
            start.to_str().unwrap(),
            "--run-weeks",
            "1",
            "--output",
            out.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let (mut w, manifest) = load_checkpoint(&out.join("final")).unwrap();
    w.mobility_data.profession_receipts.clear();
    let bad = temp.0.join("bad-history");
    save_checkpoint(&w, &bad, &manifest.identity).unwrap();
    let before = std::fs::read(bad.join("world.json")).unwrap();
    let probe = temp.0.join("rejected");
    let result = Command::new(env!("CARGO_BIN_EXE_kingmaker-cli"))
        .args([
            "--verify-continuation",
            bad.to_str().unwrap(),
            "--output",
            probe.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("subphase"));
    assert!(!probe.join("final").exists());
    assert_eq!(std::fs::read(bad.join("world.json")).unwrap(), before);
}

#[test]
fn new_process_continuation_retains_real_investment_payroll_and_completed_construction() {
    #[path = "../../kingmaker-sim/tests/common/construction.rs"]
    mod construction;
    let temp = Temp::new();
    let mut w = construction::opening(7);
    add_raw(&mut w);
    // Nonproductive laborer-only raw job keeps this explicit construction scenario's builders free.
    w.production_data
        .catalog
        .as_mut()
        .unwrap()
        .methods
        .last_mut()
        .unwrap()
        .staffing = vec!["laborers".into()];
    attach_source(&mut w);
    let start = start_world(&temp, &w);
    let out = temp.0.join("construction-verify");
    let result = Command::new(env!("CARGO_BIN_EXE_kingmaker-cli"))
        .args([
            "--resume",
            start.to_str().unwrap(),
            "--run-weeks",
            "1",
            "--output",
            out.to_str().unwrap(),
            "--verify-save",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let (saved, _) = load_checkpoint(&out.join("final")).unwrap();
    assert!(saved
        .production_data
        .construction_projects
        .iter()
        .any(|p| p.status == "completed"));
    assert!(saved
        .production_data
        .committed_operations
        .iter()
        .any(|o| o.value["type"] == "capitalist_investment"));
    let (continued, _) = load_checkpoint(&out.join("restored-continuation/final")).unwrap();
    assert!(continued
        .production_data
        .construction_projects
        .iter()
        .any(|p| p.status == "completed"));
    assert_eq!(continued.campaign.revision, 22);
}
#[test]
fn one_process_runs_two_real_weeks_in_ram_and_preserves_the_input_checkpoint() {
    let temp = Temp::new();
    let start = start(&temp);
    let initial = std::fs::read(start.join("world.json")).unwrap();
    let out = temp.0.join("run");
    let result = Command::new(env!("CARGO_BIN_EXE_kingmaker-cli"))
        .args([
            "--resume",
            start.to_str().unwrap(),
            "--run-weeks",
            "2",
            "--output",
            out.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let text = String::from_utf8_lossy(&result.stdout);
    assert!(text.contains("1/2 недель"));
    assert!(text.contains("2/2 недель"));
    assert!(text.contains("время хода"));
    let run: Value = serde_json::from_slice(&std::fs::read(out.join("run.json")).unwrap()).unwrap();
    assert_eq!(run["completed_weeks"], 2);
    assert_eq!(run["status"], "completed");
    let (w, _) = load_checkpoint(&out.join("final")).unwrap();
    assert_eq!(w.campaign.revision, 22);
    assert_eq!(w.campaign.day, 15);
    assert_eq!(w.week_data.phase_receipts.len(), 2);
    assert_eq!(
        w.source.as_ref().unwrap(),
        world().source.as_ref().unwrap()
    );
    assert_eq!(std::fs::read(start.join("world.json")).unwrap(), initial);
    let rows = std::fs::read_to_string(out.join("weeks.jsonl")).unwrap();
    assert_eq!(rows.lines().count(), 2);
}
#[test]
fn invalid_arguments_or_existing_output_refuse_before_loading_source() {
    let temp = Temp::new();
    for args in [
        vec!["--run-weeks", "0"],
        vec!["--run-weeks", "2"],
        vec!["--run-weeks", "-1"],
        vec!["--run-weeks", "1", "--output", temp.0.to_str().unwrap()],
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_kingmaker-cli"))
            .arg("--runtime")
            .arg("nonexistent-runtime.json")
            .args(args)
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert!(!String::from_utf8_lossy(&result.stderr).contains("Read runtime"));
    }
}
#[test]
fn stop_file_at_phase_boundary_saves_the_last_complete_world_without_partial_week() {
    let temp = Temp::new();
    let start = start(&temp);
    let out = temp.0.join("cancel");
    let stop = temp.0.join("STOP");
    std::fs::write(&stop, "stop").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_kingmaker-cli"))
        .args([
            "--resume",
            start.to_str().unwrap(),
            "--run-weeks",
            "2",
            "--output",
            out.to_str().unwrap(),
            "--stop-file",
            stop.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let run: Value = serde_json::from_slice(&std::fs::read(out.join("run.json")).unwrap()).unwrap();
    assert_eq!(run["status"], "cancelled");
    assert_eq!(run["completed_weeks"], 0);
    let (w, _) = load_checkpoint(&out.join("cancelled")).unwrap();
    assert_eq!(w.campaign.revision, 20);
    assert_eq!(w.campaign.day, 1);
    assert!(w.week_data.phase_receipts.is_empty());
}

#[test]
fn fresh_process_continuation_matches_continuous_economy_and_required_history() {
    let temp = Temp::new();
    let start = start(&temp);
    let out = temp.0.join("verify");
    let result = Command::new(env!("CARGO_BIN_EXE_kingmaker-cli"))
        .args([
            "--resume",
            start.to_str().unwrap(),
            "--run-weeks",
            "2",
            "--output",
            out.to_str().unwrap(),
            "--verify-save",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let run: Value = serde_json::from_slice(&std::fs::read(out.join("run.json")).unwrap()).unwrap();
    assert_eq!(run["completed_weeks"], 2);
    assert_eq!(run["new_process_continuation"]["status"], "passed");
    let stdout = String::from_utf8(result.stdout).unwrap();
    let saved = stdout
        .lines()
        .find(|line| line.starts_with("Сохранение отдельно:"))
        .unwrap();
    let seconds: f64 = saved.split_whitespace().nth(2).unwrap().parse().unwrap();
    let expected = run["final_save_elapsed_ms"].as_u64().unwrap() as f64 / 1000.;
    assert!(
        (seconds - expected).abs() < 0.001,
        "save output includes verification: {saved}; expected {expected}"
    );
    assert_eq!(
        run["new_process_continuation"]["additional_verification_weeks"],
        1
    );
    let (main, _) = load_checkpoint(&out.join("final")).unwrap();
    let (probe, _) = load_checkpoint(&out.join("restored-continuation/final")).unwrap();
    assert_eq!(main.campaign.revision, 22);
    assert_eq!(probe.campaign.revision, 23);
    assert!(out.join("restored-continuation/economic.stream").exists());
    assert_eq!(
        main.source.as_ref().unwrap(),
        probe.source.as_ref().unwrap()
    );
}

#[test]
fn profile_is_opt_in_and_preserves_saved_economy() {
 let temp=Temp::new();let input=start(&temp);let mut outputs=vec![];
 for enabled in [false,true]{let out=temp.0.join(if enabled{"profile"}else{"plain"});let mut cmd=Command::new(env!("CARGO_BIN_EXE_kingmaker-cli"));cmd.arg("--resume").arg(&input).args(["--run-weeks","1","--output"]).arg(&out);if enabled{cmd.arg("--profile");}let result=cmd.output().unwrap();assert!(result.status.success(),"{}",String::from_utf8_lossy(&result.stderr));assert_eq!(out.join("profile.jsonl").exists(),enabled);outputs.push(out);}
 let (plain,_)=load_checkpoint(&outputs[0].join("final")).unwrap();let (profiled,_)=load_checkpoint(&outputs[1].join("final")).unwrap();let stream=temp.0.join("expected.stream");kingmaker_io::economic_comparison::save_economic_state(&plain,&stream).unwrap();kingmaker_io::economic_comparison::compare_economic_state(&profiled,&stream).unwrap();
 let trace=std::fs::read_to_string(outputs[1].join("profile.jsonl")).unwrap();for name in ["run.load","run.initial_audit","run.prepare","run.apply","run.final_save"]{assert!(trace.contains(name),"missing {name}");}let last:Value=serde_json::from_str(trace.lines().last().unwrap()).unwrap();assert_eq!(last["complete"],true);
}
