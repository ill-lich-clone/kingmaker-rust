//! Queries over a published archive and the current transaction's history rows.
use crate::{history::*, row_store::RowView, ExternalId};
use serde::{Serialize, de::DeserializeOwned};
use std::{ops::Deref, sync::Arc};

pub enum HistoryRow<'a,T> { Borrowed(&'a T), Shared(Arc<T>) }
impl<T> Deref for HistoryRow<'_,T> {
    type Target=T;
    fn deref(&self)->&T {match self {Self::Borrowed(row)=>row,Self::Shared(row)=>row}}
}
impl<T:Serialize> Serialize for HistoryRow<'_,T> {
    fn serialize<S:serde::Serializer>(&self,serializer:S)->Result<S::Ok,S::Error> {self.deref().serialize(serializer)}
}
impl<T:PartialEq> PartialEq<&T> for HistoryRow<'_,T> {fn eq(&self,other:&&T)->bool {self.deref()==*other}}
impl<T> AsRef<T> for HistoryRow<'_,T> {fn as_ref(&self)->&T {self}}
pub struct HistoryView<'a,K,T> {
    archive: &'a Option<HistoryBacking>,
    namespace: &'static str,
    active: RowView<'a,K,T>,
}
impl<'a,K:PartialEq,T:DeserializeOwned> HistoryView<'a,K,T> {
    pub fn new(archive:&'a Option<HistoryBacking>,namespace:&'static str,active:RowView<'a,K,T>)->Self {Self{archive,namespace,active}}
    pub fn len(&self)->Result<usize,HistoryError> {
        Ok(self.archive.as_ref().map(|a|a.reader.count(self.namespace)).transpose()?.unwrap_or(0)+self.active.len())
    }
    pub fn scan(&self)->Box<dyn Iterator<Item=Result<HistoryRow<'a,T>,HistoryError>>+'a> {
        let archived:Box<dyn Iterator<Item=Result<HistoryRow<'a,T>,HistoryError>>+'a>=match self.archive {
            None=>Box::new(std::iter::empty()),
            Some(archive)=>Box::new(archive.reader.scan_namespace(self.namespace).map(|row| {
                let row=row?;
                let value=T::deserialize(&row.value).map_err(|e|HistoryError(e.to_string()))?;
                Ok(HistoryRow::Shared(Arc::new(value)))
            })),
        };
        Box::new(archived.chain(self.active.values().map(|row|Ok(HistoryRow::Borrowed(row)))))
    }
    pub fn select(&self,mut predicate:impl FnMut(&T)->bool)->Result<Vec<HistoryRow<'a,T>>,HistoryError> {
        let mut matches=Vec::new();
        for row in self.scan() {let row=row?;if predicate(&row) {matches.push(row);}}
        Ok(matches)
    }
    pub fn any(&self,mut predicate:impl FnMut(&T)->bool)->Result<bool,HistoryError> {
        for row in self.scan() {let row=row?;if predicate(&row) {return Ok(true);}}
        Ok(false)
    }
    pub fn get(&self,key:&K)->Result<Option<HistoryRow<'a,T>>,HistoryError> where K:Serialize {
        let active=self.active.get(key);
        let archived=if let Some(archive)=self.archive {
            let id=ExternalId::from_json(serde_json::to_value(key).map_err(|e|HistoryError(e.to_string()))?).map_err(HistoryError)?;
            archive.reader.get(&HistoryKey{namespace:self.namespace.into(),id})?
        } else {None};
        match (active,archived) {
            (Some(_),Some(_))=>Err(HistoryError("Duplicate active/archive history identity".into())),
            (Some(row),None)=>Ok(Some(HistoryRow::Borrowed(row))),
            (None,Some(row))=>Ok(Some(HistoryRow::Shared(Arc::new(T::deserialize(&row.value).map_err(|e|HistoryError(e.to_string()))?)))),
            (None,None)=>Ok(None),
        }
    }
}
// Archive identity is immutable; active rows still receive exact content guards.
impl<K:PartialEq,T:Serialize> Serialize for HistoryView<'_,K,T> {
    fn serialize<S:serde::Serializer>(&self,serializer:S)->Result<S::Ok,S::Error> {
        (self.archive.as_ref().map(HistoryBacking::token),&self.active).serialize(serializer)
    }
}
