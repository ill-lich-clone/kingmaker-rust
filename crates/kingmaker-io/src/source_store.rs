//! Private immutable source backing; no connection to the mutable author database.
use kingmaker_core::diagnostics::{add,Metric};
use kingmaker_core::{source_store::{SourceError, SourceRead, SourceReferenceStats,DictionaryEntry,SharedEntry}, StoredRecord};
use rusqlite::{params, Connection, OptionalExtension};
use std::{collections::{HashMap, VecDeque}, fs::OpenOptions, path::PathBuf, sync::{Arc, Mutex, atomic::{AtomicU64, AtomicUsize, Ordering}}};

use serde_json::Value;
pub const SOURCE_CACHE_BYTES: usize = 64 * 1024 * 1024;
fn err(e: impl std::fmt::Display) -> SourceError { SourceError(e.to_string()) }
struct OwnedDatabase { db: Option<Connection>, path: PathBuf }
impl Drop for OwnedDatabase {
    fn drop(&mut self) {
        // Close SQLite before removing this instance's exclusively created file (Windows).
        drop(self.db.take());
        let _ = std::fs::remove_file(&self.path);
    }
}
pub struct SourceStoreBuilder { owned: OwnedDatabase }
impl SourceStoreBuilder {
    pub fn temporary() -> Result<Self, SourceError> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(err)?.as_nanos();
        let path = std::env::temp_dir().join(format!("kingmaker-source-{}-{stamp}-{}.sqlite", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        OpenOptions::new().write(true).create_new(true).open(&path).map_err(err)?;
        let mut owned = OwnedDatabase { db: None, path };
        owned.db = Some(Connection::open(&owned.path).map_err(err)?);
        owned.db.as_ref().unwrap().execute_batch("PRAGMA cache_size=-2048; PRAGMA temp_store=FILE; CREATE TABLE source(seq INTEGER PRIMARY KEY,table_name TEXT NOT NULL,id_key TEXT NOT NULL,external_id TEXT,ordinal TEXT NOT NULL,sequence TEXT NOT NULL,payload TEXT NOT NULL,UNIQUE(table_name,id_key)); CREATE INDEX source_scan ON source(table_name,seq); CREATE TABLE refs(kind TEXT NOT NULL,namespace TEXT NOT NULL,id TEXT NOT NULL,payload TEXT NOT NULL,PRIMARY KEY(kind,namespace,id)); BEGIN IMMEDIATE;").map_err(err)?;
        Ok(Self { owned })
    }
    pub fn push(&mut self, row: &StoredRecord) -> Result<(), SourceError> {
        self.owned.db.as_ref().unwrap().prepare_cached("INSERT INTO source(table_name,id_key,external_id,ordinal,sequence,payload) VALUES(?1,?2,?3,?4,?5,?6)").map_err(err)?.execute(params![row.table_name,row.id_key,row.external_id_json,row.ordinal.to_string(),row.sequence.to_string(),row.payload]).map_err(err)?;
        Ok(())
    }
    fn push_reference(&mut self,kind:&str,namespace:&str,id:u64,value:&Value)->Result<(),SourceError> {
        let text=serde_json::to_string(value).map_err(err)?;
        self.owned.db.as_ref().unwrap().prepare_cached("INSERT INTO refs(kind,namespace,id,payload) VALUES(?1,?2,?3,?4)").map_err(err)?.execute(params![kind,namespace,format!("{id:020}"),text]).map_err(err)?;
        Ok(())
    }
    pub fn push_dictionary(&mut self,id:u64,value:&Value)->Result<(),SourceError> {self.push_reference("dictionary","",id,value)}
    pub fn push_shared(&mut self,kind:&str,id:u64,value:&Value)->Result<(),SourceError> {self.push_reference("shared",kind,id,value)}
    pub fn finish(self, cache_byte_limit: usize) -> Result<SourceStore, SourceError> {
        self.owned.db.as_ref().unwrap().execute_batch("COMMIT; PRAGMA query_only=ON;").map_err(err)?;
        let db=self.owned.db.as_ref().unwrap();
        let dictionary=db.query_row("SELECT COUNT(*) FROM refs WHERE kind='dictionary'",[],|r|r.get(0)).map_err(err)?;
        let shared=db.query_row("SELECT COUNT(*) FROM refs WHERE kind='shared'",[],|r|r.get(0)).map_err(err)?;
        let maximum:Option<String>=db.query_row("SELECT id FROM refs WHERE kind='dictionary' AND namespace='' ORDER BY id DESC LIMIT 1",[],|r|r.get(0)).optional().map_err(err)?;
        let stats=SourceReferenceStats{dictionary,shared,maximum:maximum.map(|s|s.parse()).transpose().map_err(err)?};
        Ok(SourceStore { owned: Mutex::new(self.owned), cache: Mutex::new(Cache::default()), refs:Mutex::new(ReferenceCache::default()), stats, prefetch:Arc::new(AtomicUsize::new(0)), limit: cache_byte_limit })
    }
}
#[derive(Default)]
struct Cache { rows: HashMap<(String,String),Arc<StoredRecord>>, fifo: VecDeque<((String,String),usize)>, bytes: usize }
#[derive(Default)]
struct ReferenceCache {rows:HashMap<(bool,String,u64),Arc<Value>>,fifo:VecDeque<((bool,String,u64),usize)>,bytes:usize}
pub struct SourceStore { owned: Mutex<OwnedDatabase>, cache: Mutex<Cache>, refs:Mutex<ReferenceCache>, stats:SourceReferenceStats, prefetch:Arc<AtomicUsize>, limit: usize }
impl std::fmt::Debug for SourceStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.debug_struct("SourceStore").field("cache_limit", &self.limit).finish_non_exhaustive() }
}
fn read_u64(r: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    r.get::<_,String>(index)?.parse().map_err(|e| rusqlite::Error::FromSqlConversionFailure(index,rusqlite::types::Type::Text,Box::new(e)))
}
fn read_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<StoredRecord> {
    Ok(StoredRecord { table_name:r.get(0)?,id_key:r.get(1)?,external_id_json:r.get(2)?,ordinal:read_u64(r,3)?,sequence:read_u64(r,4)?,payload:r.get(5)? })
}
impl SourceStore {
    pub fn cache_bytes(&self) -> usize { self.cache.lock().unwrap().bytes+self.refs.lock().unwrap().bytes }
}
impl SourceRead for SourceStore {
    fn reference_stats(&self)->SourceReferenceStats {self.stats}
    fn dictionary_value(&self,id:u64)->Result<Option<Arc<Value>>,SourceError> {self.reference(false,"",id)}
    fn shared_value(&self,kind:&str,id:u64)->Result<Option<Arc<Value>>,SourceError> {self.reference(true,kind,id)}
    fn dictionary_entries(&self)->Box<dyn Iterator<Item=Result<DictionaryEntry,SourceError>>+'_> {Box::new(self.scan_references("dictionary").map(|row|row.map(|((_,id),value)|(id,value))))}
    fn shared_entries(&self)->Box<dyn Iterator<Item=Result<SharedEntry,SourceError>>+'_> {self.scan_references("shared")}

    fn record_count(&self, table: &str) -> Result<u64,SourceError> {
        let owned = self.owned.lock().map_err(err)?;
        add(Metric::SourceSqlQueries,1);
        owned.db.as_ref().unwrap().query_row("SELECT COUNT(*) FROM source WHERE table_name=?1",[table],|r|r.get(0)).map_err(err)
    }
    fn record(&self, table: &str, key: &str) -> Result<Option<Arc<StoredRecord>>,SourceError> {
        let limit=self.limit/2;
        let key = (table.to_owned(),key.to_owned());
        // Hold the cache lock through lookup: concurrent misses cannot duplicate accounting.
        let mut cache = self.cache.lock().map_err(err)?;
        if let Some(row) = cache.rows.get(&key) { add(Metric::SourceCacheHits,1); return Ok(Some(row.clone())); }
        let owned = self.owned.lock().map_err(err)?;
        add(Metric::SourceCacheMisses,1);add(Metric::SourceSqlQueries,1);
        let row = owned.db.as_ref().unwrap().prepare_cached("SELECT table_name,id_key,external_id,ordinal,sequence,payload FROM source WHERE table_name=?1 AND id_key=?2").map_err(err)?.query_row(params![table,key.1],read_row).optional().map_err(err)?.map(Arc::new);
        if let Some(row) = &row {add(Metric::SourcePayloadBytes,row.payload.len() as u64);
            let cost = 1024 + 4 * (row.table_name.capacity() + row.id_key.capacity()) + row.payload.capacity() + row.external_id_json.as_ref().map_or(0,String::capacity);
            if cost <= limit {
                while cache.bytes > limit-cost {
                    let (old,cost) = cache.fifo.pop_front().expect("source cache accounting");
                    cache.rows.remove(&old); cache.bytes -= cost;
                }
                cache.fifo.push_back((key.clone(),cost)); cache.rows.insert(key,row.clone()); cache.bytes += cost;
            }
        }
        Ok(row)
    }
    fn scan(&self, table: &str) -> Box<dyn Iterator<Item=Result<Arc<StoredRecord>,SourceError>> + '_> {
        let table=table.to_owned();let mut seq=0i64;
        self.batched(move |limit,queue| {
            let owned=self.owned.lock().map_err(err)?;
            add(Metric::SourceSqlQueries,1);
            let mut stmt=owned.db.as_ref().unwrap().prepare_cached("SELECT table_name,id_key,external_id,ordinal,sequence,payload,seq FROM source WHERE table_name=?1 AND seq>?2 ORDER BY seq LIMIT ?3").map_err(err)?;
            let mut rows=stmt.query(params![table,seq,limit as i64]).map_err(err)?;let mut bytes=0;
            while let Some(r)=rows.next().map_err(err)? {
                let row=read_row(r).map_err(err)?;
                let cost=record_bytes(&row);
                if !queue.is_empty() && bytes+cost>SCAN_BYTES {break;}
                seq=r.get(6).map_err(err)?;bytes+=cost;
                add(Metric::SourcePayloadBytes,row.payload.len() as u64);
                queue.push_back(Ok(Arc::new(row)));
                if bytes>=SCAN_BYTES {break;}
            }
            Ok(())
        })
    }
    fn table_names(&self) -> Result<Vec<String>,SourceError> {
        let owned = self.owned.lock().map_err(err)?;
        add(Metric::SourceSqlQueries,1);
        let mut stmt = owned.db.as_ref().unwrap().prepare("SELECT DISTINCT table_name FROM source ORDER BY table_name").map_err(err)?;
        let result = stmt.query_map([],|r|r.get(0)).map_err(err)?.collect::<Result<Vec<_>,_>>().map_err(err);
        result
    }
}

fn value_bytes(value:&Value)->usize {
    64+match value {
        Value::String(s)=>s.capacity(),
        Value::Array(a)=>a.capacity()*std::mem::size_of::<Value>()+a.iter().map(value_bytes).sum::<usize>(),
        Value::Object(o)=>o.iter().map(|(k,v)|512+k.capacity()+value_bytes(v)).sum(),
        _=>0,
    }
}
impl SourceStore {
    fn reference(&self,shared:bool,namespace:&str,id:u64)->Result<Option<Arc<Value>>,SourceError> {
        let key=(shared,namespace.to_owned(),id);
        let mut cache=self.refs.lock().map_err(err)?;
        if let Some(value)=cache.rows.get(&key) {add(Metric::SourceCacheHits,1); return Ok(Some(value.clone()));}
        let owned=self.owned.lock().map_err(err)?;
        let kind=if shared {"shared"} else {"dictionary"};
        add(Metric::SourceCacheMisses,1);add(Metric::SourceSqlQueries,1);
        let text:Option<String>=owned.db.as_ref().unwrap().prepare_cached("SELECT payload FROM refs WHERE kind=?1 AND namespace=?2 AND id=?3").map_err(err)?.query_row(params![kind,namespace,format!("{id:020}")],|r|r.get(0)).optional().map_err(err)?;
        let value:Option<Arc<Value>>=text.map(|s|{add(Metric::SourcePayloadBytes,s.len() as u64);serde_json::from_str(&s).map(Arc::new)}).transpose().map_err(err)?;
        if let Some(value)=&value {
            let bytes=1024+4*key.1.capacity()+value_bytes(value);
            let limit=self.limit-self.limit/2;
            if bytes<=limit {
                while cache.bytes>limit-bytes {let (key,bytes)=cache.fifo.pop_front().expect("source reference cache accounting");cache.rows.remove(&key);cache.bytes-=bytes;}
                cache.fifo.push_back((key.clone(),bytes));cache.rows.insert(key,value.clone());cache.bytes+=bytes;
            }
        }
        Ok(value)
    }
    fn scan_references(&self,kind:&'static str)->Box<dyn Iterator<Item=Result<SharedEntry,SourceError>>+'_> {
        let mut namespace=String::new();let mut key=String::new();
        self.batched(move |limit,queue| {
            let owned=self.owned.lock().map_err(err)?;
            add(Metric::SourceSqlQueries,1);
            let mut stmt=owned.db.as_ref().unwrap().prepare_cached("SELECT namespace,id,payload FROM refs WHERE kind=?1 AND (namespace,id)>(?2,?3) ORDER BY namespace,id LIMIT ?4").map_err(err)?;
            let mut rows=stmt.query(params![kind,namespace,key,limit as i64]).map_err(err)?;let mut bytes=0;
            while let Some(r)=rows.next().map_err(err)? {
                let next_namespace:String=r.get(0).map_err(err)?;let next_key:String=r.get(1).map_err(err)?;let text:String=r.get(2).map_err(err)?;
                let id=next_key.parse().map_err(err)?;let value:Value=serde_json::from_str(&text).map_err(err)?;
                let cost=1024+next_namespace.capacity()+value_bytes(&value);
                if !queue.is_empty() && bytes+cost>SCAN_BYTES {break;}
                namespace=next_namespace;key=next_key;bytes+=cost;
                add(Metric::SourcePayloadBytes,text.len() as u64);
                queue.push_back(Ok(((namespace.clone(),id),Arc::new(value))));
                if bytes>=SCAN_BYTES {break;}
            }
            Ok(())
        })
    }
    fn batched<'a,T:'a>(&'a self,mut fill:impl FnMut(usize,&mut VecDeque<Result<T,SourceError>>)->Result<(),SourceError>+'a)->Box<dyn Iterator<Item=Result<T,SourceError>>+'a> {
        let mut queue=VecDeque::new();let mut done=false;let mut permit=None;
        Box::new(std::iter::from_fn(move || {
            if queue.is_empty() {
                permit.take();
                if done {return None;}
                permit=ScanPermit::acquire(self.prefetch.clone());
                // With all prefetch slots occupied, stream one row without retaining a batch.
                let limit=if permit.is_some(){1024}else{1};
                if let Err(e)=fill(limit,&mut queue) {queue.push_back(Err(e));done=true;}
                if queue.is_empty(){done=true;permit.take();return None;}
            }
            let row=queue.pop_front();if queue.is_empty(){permit.take();}row
        }))
    }
}
const SCAN_BYTES:usize=4*1024*1024;
const PREFETCH_BYTES:usize=64*1024*1024;
struct ScanPermit(Arc<AtomicUsize>);
impl ScanPermit {
    fn acquire(pool:Arc<AtomicUsize>)->Option<Self>{
        pool.fetch_update(Ordering::Relaxed,Ordering::Relaxed,|n|if n<=PREFETCH_BYTES-SCAN_BYTES{Some(n+SCAN_BYTES)}else{None}).ok().map(|_|Self(pool))
    }
}
impl Drop for ScanPermit {fn drop(&mut self){self.0.fetch_sub(SCAN_BYTES,Ordering::Relaxed);}}
fn record_bytes(row:&StoredRecord)->usize {
    1024+4*(row.table_name.capacity()+row.id_key.capacity())+row.payload.capacity()+row.external_id_json.as_ref().map_or(0,String::capacity)
}

#[cfg(test)]
mod remediation_tests {
    use super::*;
    #[test]
    fn remediation_oversized_reference_and_namespaces_keep_order() {
        let mut b=SourceStoreBuilder::temporary().unwrap();
        b.push_shared("b",7,&serde_json::json!("last")).unwrap();
        b.push_shared("a",9,&serde_json::json!("x".repeat(5*1024*1024))).unwrap();
        b.push_shared("a",2,&serde_json::json!("first")).unwrap();
        let s=b.finish(0).unwrap();let rows:Vec<_>=s.shared_entries().map(Result::unwrap).collect();
        assert_eq!(rows.len(),3);
        assert_eq!(rows[0].0,("a".to_string(),2));
        assert_eq!(rows[1].0,("a".to_string(),9));
        assert_eq!(rows[1].1.as_str().unwrap().len(),5*1024*1024);
        assert_eq!(rows[2].0,("b".to_string(),7));
        assert_eq!(s.prefetch.load(Ordering::Relaxed),0);
    }
    #[test]
    fn remediation_batch_error_follows_valid_rows_and_never_becomes_eof() {
        let mut b=SourceStoreBuilder::temporary().unwrap();
        for id in 0..3 {b.push_dictionary(id,&serde_json::json!({"id":id})).unwrap();}
        b.owned.db.as_ref().unwrap().execute("UPDATE refs SET payload='invalid-json' WHERE id=?1",[format!("{:020}",1)]).unwrap();
        let s=b.finish(0).unwrap();let mut scan=s.dictionary_entries();
        assert_eq!(scan.next().unwrap().unwrap().0,0);assert!(scan.next().unwrap().is_err());assert!(scan.next().is_none());
        drop(scan);assert_eq!(s.prefetch.load(Ordering::Relaxed),0);
    }
    #[test]
    fn remediation_prefetch_slots_are_shared_and_released_on_drop() {
        let mut b=SourceStoreBuilder::temporary().unwrap();
        for id in 0..4 {b.push_dictionary(id,&serde_json::json!(id)).unwrap();}
        let s=b.finish(0).unwrap();let mut scans:Vec<_>=(0..20).map(|_|s.dictionary_entries()).collect();
        for scan in &mut scans {assert_eq!(scan.next().unwrap().unwrap().0,0);}
        assert!(s.prefetch.load(Ordering::Relaxed)<=PREFETCH_BYTES);
        for scan in &mut scans {assert_eq!(scan.next().unwrap().unwrap().0,1);}
        drop(scans);assert_eq!(s.prefetch.load(Ordering::Relaxed),0);
    }
}
