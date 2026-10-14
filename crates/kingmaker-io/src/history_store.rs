//! Immutable history on disk, indexed by namespace and typed JSON identity.
use kingmaker_core::diagnostics::{add,Metric};
use kingmaker_core::history::*;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use std::{collections::{HashMap, VecDeque}, path::Path, sync::{Arc,Mutex}};

fn err(e: impl std::fmt::Display) -> HistoryError { HistoryError(e.to_string()) }
#[derive(Debug, Default)]
struct Cache {
    rows: HashMap<HistoryKey, Arc<HistoryRecord>>,
    fifo: VecDeque<(HistoryKey, usize)>,
    bytes: usize,
}
#[derive(Debug)]
pub struct HistoryStore { db: Mutex<Connection>, cache: Mutex<Cache>, limit: usize }
impl HistoryStore {
    pub fn open(path: &Path, cache_byte_limit: usize) -> Result<Self, HistoryError> {
        let db = Connection::open(path).map_err(err)?;
        db.execute_batch("PRAGMA cache_size=-1024; CREATE TABLE IF NOT EXISTS history (seq INTEGER PRIMARY KEY, namespace TEXT NOT NULL, typed_id TEXT NOT NULL, payload TEXT NOT NULL, UNIQUE(namespace,typed_id));").map_err(err)?;
        Ok(Self { db:Mutex::new(db), cache: Mutex::new(Cache::default()), limit: cache_byte_limit })
    }
    pub fn cache_bytes(&self) -> usize { self.cache.lock().unwrap().bytes }
    pub fn append(&mut self, record: &HistoryRecord) -> Result<(), HistoryError> {
        let id = serde_json::to_string(&record.key.id).map_err(err)?;
        let body = serde_json::to_string(&record.value).map_err(err)?;
        let mut db = self.db.lock().map_err(err)?;
        let tx = db.transaction().map_err(err)?;
        let old: Option<String> = tx.query_row("SELECT payload FROM history WHERE namespace=?1 AND typed_id=?2", params![record.key.namespace, id], |r| r.get(0)).optional().map_err(err)?;
        if let Some(old) = old {
            let old: Value = serde_json::from_str(&old).map_err(err)?;
            if old != record.value { return Err(HistoryError("Conflicting history identity".into())); }
        } else {
            tx.execute("INSERT INTO history(namespace,typed_id,payload) VALUES(?1,?2,?3)", params![record.key.namespace, id, body]).map_err(err)?;
        }
        tx.commit().map_err(err)
    }
    fn retain(&self, record: Arc<HistoryRecord>) {
        // Includes three key copies, hash-table slack, FIFO capacity and Arc metadata.
        let key_bytes = record.key.namespace.capacity() + match &record.key.id {
            kingmaker_core::ExternalId::String(s) => s.len(), _ => 32,
        };
        let cost = 1024 + 4 * key_bytes + value_bytes(&record.value);
        if cost > self.limit { return; }
        let mut cache = self.cache.lock().unwrap();
        while cache.bytes > self.limit - cost {
            let (key, cost) = cache.fifo.pop_front().expect("cache accounting");
            cache.rows.remove(&key);
            cache.bytes -= cost;
        }
        cache.fifo.push_back((record.key.clone(), cost));
        cache.rows.insert(record.key.clone(), record);
        cache.bytes += cost;
    }
}
// Conservative decoded allocation accounting, including object-tree overhead.
fn value_bytes(value: &Value) -> usize {
    std::mem::size_of::<Value>() + match value {
        Value::String(s) => s.capacity(),
        Value::Array(a) => a.capacity() * std::mem::size_of::<Value>() + a.iter().map(value_bytes).sum::<usize>(),
        Value::Object(o) => o.iter().map(|(k,v)| 256 + k.capacity() + value_bytes(v)).sum(),
        _ => 0,
    }
}
impl HistoryRead for HistoryStore {
    fn count(&self, namespace:&str)->Result<usize,HistoryError> {
        add(Metric::HistorySqlQueries,1);
        self.db.lock().map_err(err)?.query_row("SELECT count(*) FROM history WHERE namespace=?1",[namespace],|r|r.get(0)).map_err(err)
    }
    fn get(&self, key: &HistoryKey) -> Result<Option<Arc<HistoryRecord>>, HistoryError> {
        if let Some(record) = self.cache.lock().unwrap().rows.get(key) { add(Metric::HistoryCacheHits,1); return Ok(Some(record.clone())); }
        let id = serde_json::to_string(&key.id).map_err(err)?;
        add(Metric::HistoryCacheMisses,1);add(Metric::HistorySqlQueries,1);
        let body: Option<String> = self.db.lock().unwrap().query_row("SELECT payload FROM history WHERE namespace=?1 AND typed_id=?2", params![key.namespace,id], |r| r.get(0)).optional().map_err(err)?;
        let record = body.map(|body| {add(Metric::HistoryPayloadBytes,body.len() as u64); serde_json::from_str(&body).map(|value| Arc::new(HistoryRecord { key:key.clone(), value })).map_err(err)}).transpose()?;
        if let Some(record) = &record { self.retain(record.clone()); }
        Ok(record)
    }
    fn scan(&self) -> Box<dyn Iterator<Item = Result<Arc<HistoryRecord>, HistoryError>> + '_> {
        let mut seq = 0i64;
        let mut done = false;
        Box::new(std::iter::from_fn(move || {
            if done { return None; }
            add(Metric::HistorySqlQueries,1);
            let row = self.db.lock().unwrap().query_row("SELECT seq,namespace,typed_id,payload FROM history WHERE seq>?1 ORDER BY seq LIMIT 1", [seq], |r| Ok((r.get::<_,i64>(0)?, r.get::<_,String>(1)?, r.get::<_,String>(2)?, r.get::<_,String>(3)?))).optional();
            match row {
                Ok(Some((n, namespace, id, body))) => {
                    add(Metric::HistoryPayloadBytes,body.len() as u64);
                    seq = n;
                    let record = (|| Ok(Arc::new(HistoryRecord { key: HistoryKey { namespace, id:serde_json::from_str(&id).map_err(err)? }, value:serde_json::from_str(&body).map_err(err)? })))();
                    Some(record)
                },
                Ok(None) => { done = true; None },
                Err(e) => { done = true; Some(Err(err(e))) },
            }
        }))
    }
}
