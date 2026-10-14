#[path = "../../kingmaker-io/tests/common/mod.rs"]
mod common;
use serde_json::json;
use std::process::Command;
fn configuration(f: &common::Fixture) -> std::path::PathBuf {
    let path = f.path.with_extension("runtime.json");
    std::fs::write(&path,json!({"schema":1,"dbPath":f.path,"campaignId":"fixture","storage":"memory","workers":1,"maxHeapMiB":16384}).to_string()).unwrap();
    path
}
#[test]
fn inspect_uses_selected_runtime_and_preserves_date() {
    let f = common::Fixture::new();
    let config = configuration(&f);
    let output = Command::new(env!("CARGO_BIN_EXE_kingmaker-cli"))
        .args(["--runtime", config.to_str().unwrap(), "--inspect"])
        .output()
        .unwrap();
    let _ = std::fs::remove_file(config);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(v["mode"], "read_only");
    assert_eq!(v["campaign"]["year"], 307);
    assert_eq!(v["campaign"]["revision"], 123);
    assert_eq!(v["counts"]["stockpiles"], 1);
    assert_eq!(v["requested"]["workers"], 1);
    assert_eq!(v["native_isolated_budget"], true);
    assert_eq!(v["native_ram_week_available"], true);
    assert_eq!(v["native_week_ready"], true);
    assert_eq!(v["native_checkpoint_available"], true);
    assert_eq!(v["native_measured_runner_available"], true);
    assert_eq!(v["missing_phases"], json!([]));
    assert_eq!(v["budget_data_counts"]["employment"], 0);
}
#[test]
fn whole_week_request_refuses_before_read_or_write() {
    let f = common::Fixture::new();
    let config = configuration(&f);
    let before = std::fs::read(&f.path).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_kingmaker-cli"))
        .args(["--runtime", config.to_str().unwrap()])
        .output()
        .unwrap();
    let _ = std::fs::remove_file(config);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("EXPLICIT_RUN_REQUIRED"));
    assert_eq!(std::fs::read(&f.path).unwrap(), before);
}
