use crate::ExternalId;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HistoryKey { pub namespace: String, pub id: ExternalId }
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryRecord { pub key: HistoryKey, pub value: Value }
#[derive(Debug, thiserror::Error)]
#[error("History: {0}")]
pub struct HistoryError(pub String);
pub trait HistoryRead: std::fmt::Debug + Send + Sync {
    fn scan_namespace<'a>(&'a self, namespace: &str) -> Box<dyn Iterator<Item=Result<Arc<HistoryRecord>,HistoryError>>+'a> {
        let namespace=namespace.to_owned();
        Box::new(self.scan().filter(move |row| row.as_ref().map_or(true,|r|r.key.namespace==namespace)))
    }
    fn count(&self, namespace: &str) -> Result<usize,HistoryError> { self.scan_namespace(namespace).try_fold(0usize,|n,row|row.map(|_|n+1)) }
    fn get(&self, key: &HistoryKey) -> Result<Option<Arc<HistoryRecord>>, HistoryError>;
    fn scan(&self) -> Box<dyn Iterator<Item = Result<Arc<HistoryRecord>, HistoryError>> + '_>;
}
#[derive(Debug, Clone, Default)]
pub struct MemoryHistory {
    rows: Vec<Arc<HistoryRecord>>,
    positions: HashMap<HistoryKey, usize>,
}
impl MemoryHistory {
    pub fn append(&mut self, record: HistoryRecord) -> Result<(), HistoryError> {
        if let Some(&i) = self.positions.get(&record.key) {
            return if *self.rows[i] == record { Ok(()) } else { Err(HistoryError("Conflicting history identity".into())) };
        }
        self.positions.insert(record.key.clone(), self.rows.len());
        self.rows.push(Arc::new(record));
        Ok(())
    }
}
impl HistoryRead for MemoryHistory {
    fn get(&self, key: &HistoryKey) -> Result<Option<Arc<HistoryRecord>>, HistoryError> {
        Ok(self.positions.get(key).map(|&i| self.rows[i].clone()))
    }
    fn scan(&self) -> Box<dyn Iterator<Item = Result<Arc<HistoryRecord>, HistoryError>> + '_> {
        Box::new(self.rows.iter().cloned().map(Ok))
    }
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HistoryReferences(pub Vec<HistoryKey>);
impl HistoryReferences {
    pub fn resolve(&self, history: &dyn HistoryRead) -> Result<Vec<Arc<HistoryRecord>>, HistoryError> {
        self.0.iter().map(|key| history.get(key)?.ok_or_else(|| HistoryError("Missing referenced history".into()))).collect()
    }
}

/// An immutable published archive. A new handle invalidates prepared read guards.
#[derive(Clone,Debug)]
pub struct HistoryBacking { pub reader: Arc<dyn HistoryRead>, token: u64 }
impl HistoryBacking {
    pub fn new(reader: Arc<dyn HistoryRead>) -> Self {
        static NEXT: std::sync::atomic::AtomicU64=std::sync::atomic::AtomicU64::new(1);
        Self { reader,token:NEXT.fetch_add(1,std::sync::atomic::Ordering::Relaxed) }
    }
    pub fn token(&self)->u64 {self.token}
}
impl PartialEq for HistoryBacking {
    fn eq(&self,other:&Self)->bool {
        if Arc::ptr_eq(&self.reader,&other.reader) {return true;}
        let mut a=self.reader.scan();let mut b=other.reader.scan();
        loop {match (a.next(),b.next()) {
            (None,None)=>return true,
            (Some(Ok(a)),Some(Ok(b))) if a==b=>(),
            _=>return false,
        }}
    }
}
