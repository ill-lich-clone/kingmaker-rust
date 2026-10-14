#[path = "../../kingmaker-sim/tests/common/native_week.rs"]
mod common;
use kingmaker_core::*;
use kingmaker_io::checkpoint::{load_checkpoint, save_checkpoint, CheckpointIdentity};
use kingmaker_sim::{native_week::execute_native_week, week_history::validate_week_history};
use serde_json::json;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "kingmaker-checkpoint-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
fn identity() -> CheckpointIdentity {
    CheckpointIdentity {
        native_fingerprint: "explicit-mini-native".into(),
        engine_hash: Some("author-engine".into()),
        rules_hash: Some("author-rules".into()),
        source_hash: Some("author-world".into()),
    }
}
fn source() -> SourceSnapshot {
    let mut source = SourceSnapshot::default();
    source.campaign_id = "original-author-campaign".into();
    source.revision = 19;
    source.database_path = "unavailable-author-database.sqlite".into();
    source.campaign_payload = " {\"future_rule\": [7, \"7\"], \"date\":\"untouched\"}\n".into();
    source.static_payload = "{\"unknown\":\"кириллица 🎲\", \"formula\":\"level * 2000\"}".into();
    source.dictionary = Arc::new(
        [
            (7, json!(7)),
            (8, json!("7")),
            (9, json!({"unknown":[null,false,1.25]})),
        ]
        .into(),
    );
    source.shared = Arc::new(
        [
            (("a".into(), 7), json!({"author":[7,"7"]})),
            (("b".into(), 7), json!(["different tuple", 1])),
        ]
        .into(),
    );
    source.tables = BTreeMap::from([
        ("empty-author-table".into(), vec![]),
        (
            "unported".into(),
            vec![
                StoredRecord {
                    table_name: "unported".into(),
                    id_key: "[\"number\",7]".into(),
                    external_id_json: Some("7".into()),
                    ordinal: 12,
                    sequence: 35,
                    payload: " {\"unchanged\":true,\"rule\":\"a + b\"}\n".into(),
                },
                StoredRecord {
                    table_name: "unported".into(),
                    id_key: "[\"string\",\"7\"]".into(),
                    external_id_json: Some("\"7\"".into()),
                    ordinal: 13,
                    sequence: 38,
                    payload: "{\"untouched\":\"текст\"}".into(),
                },
            ],
        ),
    ]);
    source.table_meta = BTreeMap::from([
        ("empty-author-table".into(), (-1, 0)),
        ("unported".into(), (13, 39)),
    ]);
    let records = source.tables.get_mut("unported").unwrap();
    for (ordinal, sequence, external_id_json) in [
        (4, 12, None),
        (3, 11, Some(String::new())),
        (1, 9, Some("null".into())),
    ] {
        records.push(StoredRecord {
            table_name: "unported".into(),
            id_key: format!("opaque-author-{ordinal}"),
            external_id_json,
            ordinal,
            sequence,
            payload: "[null, false, \"неизвестный формат\"]".into(),
        });
    }
    source
}
#[test]
fn standalone_checkpoint_retains_raw_unknown_source_and_real_week_history() {
    let temp = Temp::new();
    let mut w = common::factory(false);
    w.source = Some(Arc::new(source()));
    let r = common::request(&w);
    let receipt = execute_native_week(&mut w, &r, |_| {}).unwrap();
    let before = w.clone();
    let path = temp.0.join("complete");
    let manifest = save_checkpoint(&w, &path, &identity()).unwrap();
    assert_eq!(w, before, "save is read-only");
    assert_eq!(manifest.schema, 1);
    let (loaded, restored_manifest) = load_checkpoint(&path).unwrap();
    assert_eq!(restored_manifest, manifest);
    assert_eq!(loaded, w);
    assert_eq!(
        loaded.source.as_ref().unwrap().database_path,
        "unavailable-author-database.sqlite"
    );
    assert_eq!(validate_week_history(&loaded, &r).unwrap(), Some(receipt));
}
#[test]
fn checkpoint_without_source_retains_typed_owners_currencies_and_rebuilds_account_index() {
    let temp = Temp::new();
    let mut w = common::empty();
    for (id, owner, currency, cash) in [(1, json!(7), json!(5), 9), (2, json!("7"), json!("5"), 10)]
    {
        w.accounts.push(AccountBalance {
            id: AccountId(id),
            external_id: format!("cash:{id}").into(),
            owner_id: ExternalId::from_json(owner).unwrap(),
            currency_id: ExternalId::from_json(currency).unwrap(),
            purpose: "personal_savings".into(),
            balance: Money(cash),
        });
    }
    w.rebuild_account_index().unwrap();
    let path = temp.0.join("no-source");
    save_checkpoint(&w, &path, &identity()).unwrap();
    let (loaded, _) = load_checkpoint(&path).unwrap();
    assert_eq!(loaded, w);
    assert!(loaded.source.is_none());
    assert!(!path.join("source.jsonl").exists());
    for a in &w.accounts {
        assert_eq!(loaded.account_by_address(&a.address()), Some(a));
    }
}
#[test]
fn existing_checkpoint_is_never_overwritten_and_invalid_accounts_never_publish() {
    let temp = Temp::new();
    let w = common::factory(false);
    let path = temp.0.join("original");
    save_checkpoint(&w, &path, &identity()).unwrap();
    let original = std::fs::read(path.join("world.json")).unwrap();
    let mut altered = w.clone();
    altered.campaign.revision += 1;
    assert!(save_checkpoint(&altered, &path, &identity()).is_err());
    assert_eq!(std::fs::read(path.join("world.json")).unwrap(), original);
    let mut duplicated = w.clone();
    let mut duplicate = w.accounts[0].clone();
    duplicate.id = AccountId(999);
    duplicate.external_id = "different-source-ID".into();
    duplicated.accounts.push(duplicate);
    let bad = temp.0.join("bad-accounts");
    assert!(save_checkpoint(&duplicated, &bad, &identity()).is_err());
    assert!(!bad.exists());
    altered.stockpiles[0].quantity = f64::NAN;
    let bad = temp.0.join("nan");
    assert!(save_checkpoint(&altered, &bad, &identity()).is_err());
    assert!(!bad.exists());
}
#[test]
fn incomplete_corrupt_schema_header_and_manifest_paths_refuse_atomically() {
    let temp = Temp::new();
    let mut w = common::empty();
    w.source = Some(Arc::new(source()));
    for case in 0..8 {
        let path = temp.0.join(format!("case-{case}"));
        save_checkpoint(&w, &path, &identity()).unwrap();
        match case {
            0 => std::fs::remove_file(path.join("manifest.json")).unwrap(),
            1 => std::fs::remove_file(path.join("world.json")).unwrap(),
            2 => std::fs::write(path.join("world.json"), "{}").unwrap(),
            3 => std::fs::remove_file(path.join("source.jsonl")).unwrap(),
            4 => std::fs::write(path.join("source.jsonl"), "{\"kind\":\"header\"}\n").unwrap(),
            _ => {
                let mut manifest: serde_json::Value =
                    serde_json::from_slice(&std::fs::read(path.join("manifest.json")).unwrap())
                        .unwrap();
                match case {
                    5 => manifest["schema"] = json!(2),
                    6 => manifest["campaign"]["revision"] = json!(21),
                    _ => manifest["world"]["name"] = json!("../outside.json"),
                }
                std::fs::write(
                    path.join("manifest.json"),
                    serde_json::to_vec(&manifest).unwrap(),
                )
                .unwrap();
            }
        }
        assert!(load_checkpoint(&path).is_err(), "case {case}");
        assert_eq!(w.source.as_ref().unwrap().tables, source().tables);
    }
}
#[test]
fn checkpoint_source_bytes_are_stable_after_a_lossless_roundtrip() {
    let temp = Temp::new();
    let mut w = common::empty();
    w.source = Some(Arc::new(source()));
    let first = temp.0.join("first");
    save_checkpoint(&w, &first, &identity()).unwrap();
    let (loaded, _) = load_checkpoint(&first).unwrap();
    let second = temp.0.join("second");
    save_checkpoint(&loaded, &second, &identity()).unwrap();
    assert_eq!(
        std::fs::read(first.join("source.jsonl")).unwrap(),
        std::fs::read(second.join("source.jsonl")).unwrap()
    );
    assert_eq!(
        std::fs::read(first.join("world.json")).unwrap(),
        std::fs::read(second.join("world.json")).unwrap()
    );
}

#[test]
fn failure_after_native_file_write_never_publishes_or_replaces_a_checkpoint() {
    let temp = Temp::new();
    let mut w = common::empty();
    let original = temp.0.join("successful");
    save_checkpoint(&w, &original, &identity()).unwrap();
    let existing = std::fs::read(original.join("world.json")).unwrap();
    let mut invalid = source();
    invalid.tables.get_mut("unported").unwrap()[0].table_name = "wrong-container".into();
    w.source = Some(Arc::new(invalid));
    let incomplete = temp.0.join("incomplete");
    assert!(save_checkpoint(&w, &incomplete, &identity()).is_err());
    assert!(!incomplete.exists());
    assert_eq!(
        std::fs::read(original.join("world.json")).unwrap(),
        existing
    );
    let pending = std::fs::read_dir(&temp.0)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".incomplete.pending-")
        })
        .unwrap();
    assert!(
        pending.join("world.json").exists(),
        "exercise a genuine late failure"
    );
    assert!(!pending.join("manifest.json").exists());
    assert!(load_checkpoint(&pending).is_err());
}

#[test]
fn streaming_comparison_excludes_only_week_timings_and_keeps_all_native_and_raw_source_fields() {
    use kingmaker_io::economic_comparison::{compare_economic_state, write_economic_state};
    let temp = Temp::new();
    let mut w = common::factory(false);
    w.source = Some(Arc::new(source()));
    let r = common::request(&w);
    execute_native_week(&mut w, &r, |_| {}).unwrap();
    let path = temp.0.join("economic.stream");
    write_economic_state(&w, std::fs::File::create(&path).unwrap()).unwrap();
    compare_economic_state(&w, &path).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let first_line = bytes.split(|b| *b == b'\n').next().unwrap();
    let normalized: serde_json::Value = serde_json::from_slice(first_line).unwrap();
    let original = serde_json::to_value(&w).unwrap();
    assert_eq!(
        normalized.as_object().unwrap().keys().collect::<Vec<_>>(),
        original.as_object().unwrap().keys().collect::<Vec<_>>(),
        "comparison must cover every serialized WorldState field"
    );
    let mut changed = w.clone();
    changed.week_data.phase_receipts[0].timings[0].1 += 900;
    compare_economic_state(&changed, &path).unwrap();
    changed = w.clone();
    changed.accounts[0].balance.0 += 1;
    assert!(compare_economic_state(&changed, &path).is_err());
    changed = w.clone();
    changed.week_data.phase_receipts[0]
        .diagnostics
        .push(json!({"economic_reason":"keep mandatory diagnostic"}));
    assert!(compare_economic_state(&changed, &path).is_err());
    changed = w.clone();
    Arc::make_mut(changed.source.as_mut().unwrap())
        .tables
        .get_mut("unported")
        .unwrap()[0]
        .payload
        .push(' ');
    assert!(compare_economic_state(&changed, &path).is_err());
}

#[test]
fn checkpoint_restores_disk_source_after_its_original_temporary_store_is_closed() {
    let temp=Temp::new();
    let expected=source();
    let mut disk=expected.clone();
    let mut writer=kingmaker_io::source_store::SourceStoreBuilder::temporary().unwrap();
    for (id,value) in disk.dictionary.iter() {writer.push_dictionary(*id,value).unwrap();}
    for ((kind,id),value) in disk.shared.iter() {writer.push_shared(kind,*id,value).unwrap();}
    for rows in disk.tables.values_mut() {for row in rows.iter() {writer.push(row).unwrap();} rows.clear();}
    disk.backing=Some(Arc::new(writer.finish(8192).unwrap()));
    disk.dictionary=Default::default();disk.shared=Default::default();
    let mut w=common::empty();w.source=Some(Arc::new(disk));
    let path=temp.0.join("independent-source");
    save_checkpoint(&w,&path,&identity()).unwrap();
    drop(w); // Last store handle closes and deletes its exclusively owned temporary database.
    let (loaded,_)=load_checkpoint(&path).unwrap();
    assert_eq!(loaded.source.as_deref().unwrap(),&expected);
    assert_eq!(*loaded.source.as_ref().unwrap().dictionary_value(8).unwrap().unwrap(),json!("7"));
}

#[test]
fn remediation_cancel_checkpoint_keeps_prior_snapshot() {
    let temp=Temp::new();let mut w=common::empty();w.provinces.push(kingmaker_core::ProvinceRecord{id:kingmaker_core::ProvinceId(1),pop_range:(0,10000),..Default::default()});w.pops=(0..10000).map(|i|kingmaker_core::PopRecord{id:kingmaker_core::PopId(i),province_id:kingmaker_core::ProvinceId(1),external_id:format!("p{i}").into(),..Default::default()}).collect();let previous=temp.0.join("previous");
    save_checkpoint(&w,&previous,&identity()).unwrap();
    let pending=temp.0.join("cancelled");let calls=std::cell::Cell::new(0);
    let result=kingmaker_io::checkpoint::save_checkpoint_controlled(&w,&pending,&identity(),&|label|{
        if label=="checkpoint_write" {calls.set(calls.get()+1);if calls.get()>=4 {anyhow::bail!("STOP during write");}}Ok(())
    });
    assert!(result.is_err());assert!(!pending.exists());assert_eq!(calls.get(),4);
    assert_eq!(load_checkpoint(&previous).unwrap().0.campaign,w.campaign);
}
