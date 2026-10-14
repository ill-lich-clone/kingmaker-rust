use crate::SqliteLoader;
use anyhow::{bail, Context, Result};
use kingmaker_core::WorldState;
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub file: PathBuf,
    pub project_root: PathBuf,
    pub db_path: PathBuf,
    pub campaign_id: String,
    pub storage: String,
    pub workers: u64,
    pub max_heap_mib: u64,
    pub checkpoint_directory: Option<PathBuf>,
    pub checkpoint_path: Option<PathBuf>,
    pub engine_hash: Option<String>,
    pub raw: Value,
}
impl RuntimeConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let file = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };
        let directory = file
            .parent()
            .context("Runtime file must have a directory")?;
        // Same relative path rule as JS: <project>/data/kingmaker/runtime.json.
        let project_root = if directory.file_name().is_some_and(|n| n == "kingmaker")
            && directory
                .parent()
                .and_then(Path::file_name)
                .is_some_and(|n| n == "data")
        {
            directory
                .parent()
                .and_then(Path::parent)
                .context("Missing project root")?
                .to_path_buf()
        } else {
            directory.to_path_buf()
        }; // Explicit self-contained diagnostic config.
        let text = std::fs::read_to_string(&file)
            .with_context(|| format!("Read runtime {}", file.display()))?;
        let raw: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
            .context("Invalid runtime JSON")?;
        if raw["schema"].as_u64() != Some(1) {
            bail!("Unsupported runtime schema");
        }
        let database = raw["dbPath"]
            .as_str()
            .filter(|s| !s.is_empty())
            .context("Runtime dbPath is required")?;
        let db_path = resolve(&project_root, database);
        let campaign_id = match raw.get("campaignId") {
            None => "world-306".into(),
            Some(v) => v
                .as_str()
                .filter(|s| !s.is_empty())
                .context("Invalid runtime campaignId")?
                .to_string(),
        };
        let storage = raw
            .get("storage")
            .map(|v| v.as_str().context("Invalid runtime storage"))
            .transpose()?
            .unwrap_or("sql")
            .to_string();
        if !["memory", "sql"].contains(&storage.as_str()) {
            bail!("Unsupported requested runtime storage {storage}");
        }
        let workers = integer(&raw, "workers", 2)?;
        let max_heap_mib = integer(&raw, "maxHeapMiB", 16384)?;
        let checkpoint_directory = optional_path(&raw, "checkpointDirectory", &project_root)?;
        let checkpoint_path = optional_path(&raw, "checkpointPath", &project_root)?;
        let engine_hash = raw
            .get("engine_hash")
            .map(|v| {
                v.as_str()
                    .map(str::to_string)
                    .context("Invalid runtime engine_hash")
            })
            .transpose()?;
        Ok(Self {
            file,
            project_root,
            db_path,
            campaign_id,
            storage,
            workers,
            max_heap_mib,
            checkpoint_directory,
            checkpoint_path,
            engine_hash,
            raw,
        })
    }
    pub fn load_world(&self) -> Result<WorldState> {
        let world = SqliteLoader::load_from_path(&self.db_path, &self.campaign_id)?;
        if self
            .engine_hash
            .as_ref()
            .is_some_and(|hash| Some(hash) != world.campaign.engine_hash.as_ref())
        {
            bail!("Runtime source engine_hash differs from selected campaign");
        }
        Ok(world)
    }
}
fn integer(v: &Value, key: &str, default: u64) -> Result<u64> {
    let result = match v.get(key) {
        None => default,
        Some(value) => value
            .as_u64()
            .with_context(|| format!("Runtime {key} must be a positive integer"))?,
    };
    if result == 0 {
        bail!("Runtime {key} must be positive");
    }
    Ok(result)
}
fn resolve(root: &Path, value: &str) -> PathBuf {
    let p = Path::new(value);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        root.join(p)
    }
}
fn optional_path(v: &Value, key: &str, root: &Path) -> Result<Option<PathBuf>> {
    v.get(key)
        .map(|value| {
            value
                .as_str()
                .filter(|s| !s.is_empty())
                .map(|s| resolve(root, s))
                .with_context(|| format!("Invalid runtime {key}"))
        })
        .transpose()
}

pub fn default_runtime_path() -> PathBuf {
    const RELATIVE: &str = "Проект карты Тейванкаля/data/kingmaker/runtime.json";
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent().and_then(Path::parent).and_then(Path::parent) {
            let path = parent.join(RELATIVE);
            if path.is_file() {
                return path;
            }
        }
    }
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .join(RELATIVE)
}
