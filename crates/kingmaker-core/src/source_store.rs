//! Immutable raw source records. Scans retain only the currently yielded row.
use crate::{SourceSnapshot, StoredRecord};
use std::{collections::BTreeSet, sync::Arc};

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct SourceError(pub String);

pub type DictionaryEntry=(u64,Arc<serde_json::Value>);
pub type SharedEntry=((String,u64),Arc<serde_json::Value>);
#[derive(Debug,Default,Clone,Copy)]
pub struct SourceReferenceStats {pub dictionary:usize,pub shared:usize,pub maximum:Option<u64>}
pub trait SourceRead: std::fmt::Debug + Send + Sync {
    fn dictionary_value(&self,_id:u64)->Result<Option<Arc<serde_json::Value>>,SourceError> {Ok(None)}
    fn shared_value(&self,_kind:&str,_id:u64)->Result<Option<Arc<serde_json::Value>>,SourceError> {Ok(None)}
    fn dictionary_entries(&self)->Box<dyn Iterator<Item=Result<DictionaryEntry,SourceError>>+'_> {Box::new(std::iter::empty())}
    fn shared_entries(&self)->Box<dyn Iterator<Item=Result<SharedEntry,SourceError>>+'_> {Box::new(std::iter::empty())}
    fn reference_stats(&self)->SourceReferenceStats {SourceReferenceStats::default()}
    fn record(&self, table: &str, key: &str) -> Result<Option<Arc<StoredRecord>>, SourceError>;
    fn scan(&self, table: &str) -> Box<dyn Iterator<Item = Result<Arc<StoredRecord>, SourceError>> + '_>;
    fn table_names(&self) -> Result<Vec<String>, SourceError>;
    fn record_count(&self, table: &str) -> Result<u64, SourceError> { self.scan(table).try_fold(0u64, |n,row| row.map(|_|n+1)) }
}
impl SourceSnapshot {
    pub fn dictionary_value(&self,id:u64)->Result<Option<Arc<serde_json::Value>>,SourceError> {
        if let Some(v)=self.dictionary.get(&id) {return Ok(Some(Arc::new(v.clone())));}
        self.backing.as_ref().map_or(Ok(None),|s|s.dictionary_value(id))
    }
    pub fn shared_value(&self,kind:&str,id:u64)->Result<Option<Arc<serde_json::Value>>,SourceError> {
        if let Some(v)=self.shared.get(&(kind.into(),id)) {return Ok(Some(Arc::new(v.clone())));}
        self.backing.as_ref().map_or(Ok(None),|s|s.shared_value(kind,id))
    }
    pub fn max_dictionary_id(&self)->Option<u64> {
        self.dictionary.keys().copied().max().into_iter().chain(self.backing.as_ref().and_then(|b|b.reference_stats().maximum)).max()
    }
    pub fn dictionary_count(&self)->usize {if self.dictionary.is_empty() {self.backing.as_ref().map_or(0,|b|b.reference_stats().dictionary)} else {self.dictionary.len()}}
    pub fn dictionary_entries(&self)->Box<dyn Iterator<Item=Result<DictionaryEntry,SourceError>>+'_> {
        if self.dictionary.is_empty() {if let Some(backing)=&self.backing {return backing.dictionary_entries();}}
        let mut rows:Vec<_>=self.dictionary.iter().collect();rows.sort_by_key(|(id,_)|**id);
        Box::new(rows.into_iter().map(|(id,value)|Ok((*id,Arc::new(value.clone())))))
    }
    pub fn shared_entries(&self)->Box<dyn Iterator<Item=Result<SharedEntry,SourceError>>+'_> {
        if self.shared.is_empty() {if let Some(backing)=&self.backing {return backing.shared_entries();}}
        let mut rows:Vec<_>=self.shared.iter().collect();rows.sort_by(|a,b|a.0.cmp(b.0));
        Box::new(rows.into_iter().map(|(id,value)|Ok((id.clone(),Arc::new(value.clone())))))
    }

    pub fn record_count(&self) -> Result<u64, SourceError> {
        if let Some(backing) = &self.backing { return self.table_names()?.iter().try_fold(0u64,|n,t| backing.record_count(t).map(|v|n+v)); }
        Ok(self.tables.values().map(|v|v.len() as u64).sum())
    }
    pub fn records(&self, table: &str) -> Box<dyn Iterator<Item = Result<Arc<StoredRecord>, SourceError>> + '_> {
        if let Some(backing) = &self.backing { return backing.scan(table); }
        Box::new(self.tables.get(table).into_iter().flatten().map(|row| Ok(Arc::new(row.clone()))))
    }
    pub fn table_names(&self) -> Result<Vec<String>, SourceError> {
        let mut names: BTreeSet<String> = self.tables.keys().cloned().collect();
        if let Some(backing) = &self.backing { names.extend(backing.table_names()?); }
        Ok(names.into_iter().collect())
    }
}
impl PartialEq for SourceSnapshot {
    fn eq(&self, other: &Self) -> bool {
        if self.campaign_id != other.campaign_id || self.revision != other.revision
            || self.database_path != other.database_path || self.campaign_payload != other.campaign_payload
            || self.static_payload != other.static_payload || self.table_meta != other.table_meta { return false; }
        if let (Some(a), Some(b)) = (&self.backing, &other.backing) {
            if Arc::ptr_eq(a, b) && self.tables == other.tables && self.dictionary==other.dictionary && self.shared==other.shared { return true; }
        }
        fn equal<T:PartialEq>(mut a:impl Iterator<Item=Result<T,SourceError>>,mut b:impl Iterator<Item=Result<T,SourceError>>)->bool {
            loop {match (a.next(),b.next()) {(None,None)=>return true,(Some(Ok(a)),Some(Ok(b))) if a==b=>(),_=>return false}}
        }
        if !equal(self.dictionary_entries(),other.dictionary_entries()) || !equal(self.shared_entries(),other.shared_entries()) {return false;}
        let (Ok(a), Ok(b)) = (self.table_names(), other.table_names()) else { return false; };
        if a != b { return false; }
        for table in a {
            let mut left = self.records(&table);
            let mut right = other.records(&table);
            loop { match (left.next(), right.next()) {
                (None, None) => break,
                (Some(Ok(a)), Some(Ok(b))) if a == b => (),
                _ => return false,
            } }
        }
        true
    }
}
