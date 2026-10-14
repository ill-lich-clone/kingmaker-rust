//! Independent streaming snapshots. Native state and raw author source are both required.
use anyhow::{bail, ensure, Context, Result};
use kingmaker_core::{CampaignHeader, SourceSnapshot, StoredRecord, WorldState};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, BufWriter, Read, Write},
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckpointIdentity {
    pub native_fingerprint: String,
    pub engine_hash: Option<String>,
    pub rules_hash: Option<String>,
    pub source_hash: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckpointFile {
    pub name: String,
    pub bytes: u64,
    pub sha256: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckpointManifest {
    pub schema: u32,
    pub campaign: CampaignHeader,
    pub identity: CheckpointIdentity,
    pub world: CheckpointFile,
    pub source: Option<CheckpointFile>,
}
struct Hashed<T> {
    inner: T,
    hash: Sha256,
    bytes: u64,
}
impl<T> Hashed<T> {
    fn new(inner: T) -> Self {
        Self {
            inner,
            hash: Sha256::new(),
            bytes: 0,
        }
    }
    fn metadata(&self, name: &str) -> CheckpointFile {
        CheckpointFile {
            name: name.into(),
            bytes: self.bytes,
            sha256: self
                .hash
                .clone()
                .finalize()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect(),
        }
    }
}
impl<T: Write> Write for Hashed<T> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(bytes)?;
        self.hash.update(&bytes[..n]);
        self.bytes += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}
impl<T: Read> Read for Hashed<T> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(bytes)?;
        self.hash.update(&bytes[..n]);
        self.bytes += n as u64;
        Ok(n)
    }
}
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum SourceRowRef<'a> {
    Header {
        campaign_id: &'a str,
        revision: u64,
        database_path: &'a str,
        campaign_payload: &'a str,
        static_payload: &'a str,
        table_names: Vec<&'a str>,
        table_meta: &'a BTreeMap<String, (i64, i64)>,
    },
    Dictionary {
        key: u64,
        value: &'a Value,
    },
    Shared {
        namespace: &'a str,
        key: u64,
        value: &'a Value,
    },
    Record {
        table_name: &'a str,
        id_key: &'a str,
        external_id_json: &'a Option<String>,
        ordinal: u64,
        sequence: u64,
        payload: &'a str,
    },
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum SourceRow {
    Header {
        campaign_id: String,
        revision: u64,
        database_path: String,
        campaign_payload: String,
        static_payload: String,
        table_names: Vec<String>,
        table_meta: BTreeMap<String, (i64, i64)>,
    },
    Dictionary {
        key: u64,
        value: Value,
    },
    Shared {
        namespace: String,
        key: u64,
        value: Value,
    },
    Record {
        table_name: String,
        id_key: String,
        external_id_json: Option<String>,
        ordinal: u64,
        sequence: u64,
        payload: String,
    },
}
pub(crate) fn write_source(writer: &mut dyn Write, source: &SourceSnapshot) -> Result<()> {
    let mut row = |value: SourceRowRef<'_>| -> Result<()> {
        serde_json::to_writer(&mut *writer, &value)?;
        writer.write_all(b"\n")?;
        Ok(())
    };
    let table_names = source.table_names()?;
    row(SourceRowRef::Header {
        campaign_id: &source.campaign_id,
        revision: source.revision,
        database_path: &source.database_path,
        campaign_payload: &source.campaign_payload,
        static_payload: &source.static_payload,
        table_names: table_names.iter().map(String::as_str).collect(),
        table_meta: &source.table_meta,
    })?;
    // Disk-backed references stream in stable key order without retaining their payloads.
    for entry in source.dictionary_entries() {
        let (key,value)=entry?;
        row(SourceRowRef::Dictionary {key,value:&value})?;
    }
    for entry in source.shared_entries() {
        let ((namespace,key),value)=entry?;
        row(SourceRowRef::Shared {namespace:&namespace,key,value:&value})?;
    }
    for table in &table_names {
        for record in source.records(table) {
            let record = record?;
            ensure!(
                record.table_name == *table,
                "Raw source row table conflicts with its containing table"
            );
            row(SourceRowRef::Record {
                table_name: &record.table_name,
                id_key: &record.id_key,
                external_id_json: &record.external_id_json,
                ordinal: record.ordinal,
                sequence: record.sequence,
                payload: &record.payload,
            })?;
        }
    }
    Ok(())
}
fn read_source(reader: &mut impl BufRead) -> Result<SourceSnapshot> {
    let mut source = None;
    let mut backing = crate::source_store::SourceStoreBuilder::temporary()?;
    let mut line = String::new();
    while reader.read_line(&mut line)? > 0 {
        let row: SourceRow =
            serde_json::from_str(&line).context("Invalid checkpoint source row")?;
        match row {
            SourceRow::Header {
                campaign_id,
                revision,
                database_path,
                campaign_payload,
                static_payload,
                table_names,
                table_meta,
            } => {
                ensure!(
                    source.is_none(),
                    "Duplicate/misplaced source header"
                );
                let mut tables = BTreeMap::new();
                for name in table_names {
                    ensure!(
                        tables.insert(name, vec![]).is_none(),
                        "Duplicate source table name"
                    );
                }
                source = Some(SourceSnapshot {
                    campaign_id,
                    revision,
                    database_path,
                    campaign_payload,
                    static_payload,
                    tables,
                    table_meta,
                    ..Default::default()
                });
            }
            SourceRow::Dictionary { key, value } => {
                ensure!(source.is_some(), "Source header missing");
                backing.push_dictionary(key,&value).context("Source dictionary key")?;
            }
            SourceRow::Shared {
                namespace,
                key,
                value,
            } => {
                ensure!(source.is_some(), "Source header missing");
                backing.push_shared(&namespace,key,&value).context("Source shared key")?;
            }
            SourceRow::Record {
                table_name,
                id_key,
                external_id_json,
                ordinal,
                sequence,
                payload,
            } => {
                let source = source.as_mut().context("Source header missing")?;
                ensure!(source.tables.contains_key(&table_name), "Source row table missing from header");
                backing.push(&StoredRecord {
                        table_name,
                        id_key,
                        external_id_json,
                        ordinal,
                        sequence,
                        payload,
                    })?;
            }
        }
        line.clear();
    }
    let mut source = source.context("Empty checkpoint source")?;
    source.backing = Some(Arc::new(backing.finish(crate::source_store::SOURCE_CACHE_BYTES)?));
    Ok(source)
}
fn validate_native(world: &WorldState) -> Result<()> {
    let report = world.validate_links();
    ensure!(
        report.errors.is_empty(),
        "Checkpoint native links invalid: {:?}",
        report.errors
    );
    let mut addresses = HashSet::new();
    ensure!(
        world.accounts.iter().all(|a| addresses.insert(a.address())),
        "Duplicate checkpoint account address"
    );
    ensure!(
        (1..=12).contains(&world.campaign.month) && (1..=30).contains(&world.campaign.day),
        "Invalid checkpoint campaign date"
    );
    Ok(())
}
fn write_file(
    path: &Path,
    name: &str,
    write: impl FnOnce(&mut dyn Write) -> Result<()>,
) -> Result<CheckpointFile> {
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path.join(name))?;
    let mut writer = BufWriter::with_capacity(1024 * 1024, Hashed::new(file));
    write(&mut writer)?;
    writer.flush()?;
    writer.get_ref().inner.sync_all()?;
    Ok(writer.get_ref().metadata(name))
}
pub fn save_checkpoint(
    world: &WorldState,
    path: &Path,
    identity: &CheckpointIdentity,
) -> Result<CheckpointManifest> {
    save_checkpoint_controlled(world,path,identity,&|_|Ok(()))
}
pub fn save_checkpoint_controlled(
    world:&WorldState,path:&Path,identity:&CheckpointIdentity,
    check:&dyn Fn(&str)->Result<()>,
) -> Result<CheckpointManifest> {
kingmaker_core::diagnostics::measure("io.checkpoint.save", kingmaker_core::diagnostics::Visibility::Boundary, || {
    check("checkpoint_start")?;
    validate_native(world)?;
    ensure!(
        !identity.native_fingerprint.is_empty(),
        "Native checkpoint fingerprint required"
    );
    ensure!(
        !path.exists(),
        "Checkpoint already exists: {}",
        path.display()
    );
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .context("Checkpoint target must name a directory")?;
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let pending = parent.join(format!(
        ".{name}.pending-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&pending).context("Create new checkpoint staging directory")?;
    ensure!(world.history.is_none(), "Archive checkpoint persistence not yet integrated");
    // A failed staging directory stays unpublished for diagnosis. No existing
    // successful snapshot is deleted or reused, even after a partial write.
    let world_file = write_file(&pending, "world.json", |out| {
        checked_write(out,check,|out| {serde_json::to_writer(out, world)?;Ok(())})?;
        Ok(())
    })?;
    let source = world
        .source
        .as_ref()
        .map(|s| write_file(&pending, "source.jsonl", |out| checked_write(out,check,|out|write_source(out,s))))
        .transpose()?;
    let manifest = CheckpointManifest {
        schema: 1,
        campaign: world.campaign.clone(),
        identity: identity.clone(),
        world: world_file,
        source,
    };
    write_file(&pending, "manifest.json", |out| {
        checked_write(out,check,|out| {serde_json::to_writer(out, &manifest)?;Ok(())})?;
        Ok(())
    })?;
    ensure!(!path.exists(), "Checkpoint target appeared during write");
    check("checkpoint_publish")?;
    fs::rename(&pending, path).context("Publish checkpoint directory atomically")?;
    Ok(manifest)

})
}
fn verify_file(actual: CheckpointFile, expected: &CheckpointFile) -> Result<()> {
    ensure!(
        actual == *expected,
        "Checkpoint file checksum/size mismatch: {}",
        expected.name
    );
    Ok(())
}
pub fn load_checkpoint(path: &Path) -> Result<(WorldState, CheckpointManifest)> {
kingmaker_core::diagnostics::measure("io.checkpoint.load", kingmaker_core::diagnostics::Visibility::Boundary, || {
    let manifest: CheckpointManifest =
        serde_json::from_reader(BufReader::new(File::open(path.join("manifest.json"))?))
            .context("Read checkpoint manifest")?;
    ensure!(manifest.schema == 1, "Unsupported checkpoint schema");
    ensure!(
        !manifest.identity.native_fingerprint.is_empty(),
        "Native checkpoint fingerprint required"
    );
    ensure!(
        manifest.world.name == "world.json",
        "Invalid checkpoint world file name"
    );
    ensure!(
        manifest
            .source
            .as_ref()
            .is_none_or(|s| s.name == "source.jsonl"),
        "Invalid checkpoint source file name"
    );
    let mut reader = BufReader::with_capacity(
        1024 * 1024,
        Hashed::new(File::open(path.join("world.json"))?),
    );
    let mut world: WorldState =
        serde_json::from_reader(&mut reader).context("Read checkpoint native world")?;
    verify_file(reader.get_ref().metadata("world.json"), &manifest.world)?;
    ensure!(
        world.campaign == manifest.campaign,
        "Checkpoint campaign header mismatch"
    );
    world.rebuild_account_index().map_err(anyhow::Error::msg)?;
    validate_native(&world)?;
    if let Some(source_file) = &manifest.source {
        let mut reader = BufReader::with_capacity(
            1024 * 1024,
            Hashed::new(File::open(path.join("source.jsonl"))?),
        );
        let source = read_source(&mut reader)?;
        verify_file(reader.get_ref().metadata("source.jsonl"), source_file)?;
        world.source = Some(Arc::new(source));
    } else if path.join("source.jsonl").exists() {
        bail!("Source file exists without checkpoint metadata");
    }
    Ok((world, manifest))

})
}

fn checked_write(out:&mut dyn Write,check:&dyn Fn(&str)->Result<()>,write:impl FnOnce(&mut dyn Write)->Result<()>)->Result<()> {
    struct Controlled<'a> {out:&'a mut dyn Write,check:&'a dyn Fn(&str)->Result<()>,bytes:usize,fragments:usize}
    impl Write for Controlled<'_> {
        fn write(&mut self,bytes:&[u8])->std::io::Result<usize> {
            let part=&bytes[..bytes.len().min(1024*1024)];
            if self.bytes>=64*1024 || self.fragments>=4096 {
                (self.check)("checkpoint_write").map_err(std::io::Error::other)?;self.bytes=0;self.fragments=0;
            }
            let n=self.out.write(part)?;self.bytes+=n;self.fragments+=1;Ok(n)
        }
        fn flush(&mut self)->std::io::Result<()> {self.out.flush()}
    }
    check("checkpoint_write")?;
    write(&mut Controlled{out,check,bytes:0,fragments:0})?;
    check("checkpoint_write")
}
