use kingmaker_io::runtime_config::RuntimeConfig;
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct ConfigFixture {
    root: PathBuf,
    file: PathBuf,
}
impl ConfigFixture {
    fn new(v: Value) -> Self {
        let root = std::env::temp_dir().join(format!(
            "kingmaker-rust-config-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let file = root.join("data/kingmaker/runtime.json");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, v.to_string()).unwrap();
        Self { root, file }
    }
}
impl Drop for ConfigFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.file);
        let _ = std::fs::remove_dir(self.root.join("data/kingmaker"));
        let _ = std::fs::remove_dir(self.root.join("data"));
        let _ = std::fs::remove_dir(&self.root);
    }
}
fn config() -> Value {
    json!({"schema":1,"dbPath":"world.sqlite","campaignId":"fixture","storage":"memory","workers":1,"maxHeapMiB":16384,"checkpointDirectory":"checkpoints/base","future_authored_setting":true})
}
#[test]
fn shared_runtime_relative_paths_match_js_project_root_rules() {
    let f = ConfigFixture::new(config());
    let c = RuntimeConfig::load(&f.file).unwrap();
    assert_eq!(c.db_path, f.root.join("world.sqlite"));
    assert_eq!(
        c.checkpoint_directory,
        Some(f.root.join("checkpoints/base"))
    );
    assert_eq!(c.workers, 1);
    assert_eq!(c.storage, "memory");
    assert_eq!(c.max_heap_mib, 16384);
    assert_eq!(c.campaign_id, "fixture");
    assert_eq!(c.raw["future_authored_setting"], true);
}
#[test]
fn invalid_runtime_does_not_fall_back_to_old_database() {
    for patch in [
        json!({"schema":2}),
        json!({"dbPath":""}),
        json!({"workers":0}),
        json!({"storage":"unknown"}),
    ] {
        let mut v = config();
        for (k, x) in patch.as_object().unwrap() {
            v[k] = x.clone();
        }
        let f = ConfigFixture::new(v);
        assert!(RuntimeConfig::load(&f.file).is_err());
    }
}
#[test]
fn missing_runtime_is_an_error() {
    let f = ConfigFixture::new(config());
    std::fs::remove_file(&f.file).unwrap();
    assert!(RuntimeConfig::load(&f.file).is_err());
}
#[test]
fn sql_storage_setting_is_preserved() {
    let mut v = config();
    v["storage"] = json!("sql");
    let f = ConfigFixture::new(v);
    assert_eq!(RuntimeConfig::load(&f.file).unwrap().storage, "sql");
}
