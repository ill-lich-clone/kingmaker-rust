use kingmaker_core::history::*;
use kingmaker_io::history_store::HistoryStore;
use serde_json::json;
#[test]
fn disk_history_keeps_typed_identity_conflicts_and_bounded_cache_across_reopen() {
    let dir = std::env::temp_dir().join(format!("kingmaker-history-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("history.sqlite");
    let key = |id| HistoryKey { namespace:"operations".into(), id };
    {
        let mut history = HistoryStore::open(&path, 8192).unwrap();
        for n in 0..1000u32 {
            history.append(&HistoryRecord { key:key(n.into()), value:json!({"notes":"x".repeat(2048),"n":n}) }).unwrap();
            assert!(history.cache_bytes() <= 8192);
        }
        history.append(&HistoryRecord { key:key("1".into()), value:json!("string") }).unwrap();
        let old = history.get(&key(1.into())).unwrap().unwrap();
        history.append(&old).unwrap();
        assert!(history.append(&HistoryRecord { key:key(1.into()), value:json!("conflict") }).is_err());
        for n in 0..1000u32 {
            assert_eq!(history.get(&key(n.into())).unwrap().unwrap().value["n"], n);
            assert!(history.cache_bytes() <= 8192);
        }
        assert_eq!(history.get(&key(1.into())).unwrap().unwrap().value, old.value);
        assert_eq!(history.get(&key("1".into())).unwrap().unwrap().value, "string");
        assert_eq!(history.scan().map(|r| r.unwrap()).count(), 1001);
        let large = HistoryRecord { key:key("large".into()), value:json!(["y".repeat(65536)]) };
        history.append(&large).unwrap();
        assert_eq!(*history.get(&large.key).unwrap().unwrap(), large);
        assert!(history.cache_bytes() <= 8192);
    }
    let history = HistoryStore::open(&path, 0).unwrap();
    assert_eq!(history.scan().count(), 1002);
    assert_eq!(history.get(&key(0.into())).unwrap().unwrap().value["n"], 0);
    assert_eq!(history.cache_bytes(), 0);
}
