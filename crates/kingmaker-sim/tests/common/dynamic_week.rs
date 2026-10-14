#![allow(dead_code)]
#[path = "income.rs"]
mod income;
use kingmaker_core::WorldState;
use serde_json::{json, Value};
use std::{
    io::Write,
    process::{Command, Stdio},
};
pub fn oracle(action: &str, w: &WorldState) -> Value {
    let mut snapshot = income::snapshot(w);
    snapshot["registry"]["consumerFacilityByProvince"] =
        json!(w.market_data.consumer_facility_by_province);
    snapshot["registry"]["completedConstruction"] = json!(w.production_data.completed_construction);
    snapshot["registry"]["completedExpansions"] = json!(w.production_data.expansions);
    let mut child = Command::new("node")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/tests/shipping-oracle.cjs"
        ))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(
            json!({"action":action,"period":"w","snapshot":snapshot})
                .to_string()
                .as_bytes(),
        )
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}
