//! Stable-order indexed rows. Cloning a published table shares all payloads.
use std::{collections::HashMap, hash::Hash, sync::Arc};

pub trait RowRead<K, T> {
    fn len(&self) -> usize;
    fn at(&self, index: usize) -> Option<&T>;
    fn get(&self, key: &K) -> Option<&T>;
    fn iter(&self) -> Box<dyn Iterator<Item = (K, &T)> + '_>;
    fn values(&self) -> Box<dyn Iterator<Item = &T> + '_>;
}
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Row<K, T> { pub key: K, pub value: T }
#[derive(Debug, Clone)]
pub struct RowTable<K, T> {
    rows: Arc<Vec<Arc<Row<K, T>>>>,
    positions: Arc<HashMap<K, usize>>,
}
impl<K, T> Default for RowTable<K, T> {
    fn default() -> Self { Self { rows: Arc::new(vec![]), positions: Arc::new(HashMap::new()) } }
}
impl<K: Eq + Hash + Clone, T> RowTable<K, T> {
    pub fn from_rows(rows: impl IntoIterator<Item = (K, T)>) -> Result<Self, String> {
        let mut data = vec![];
        let mut positions = HashMap::new();
        for (key, value) in rows {
            if positions.insert(key.clone(), data.len()).is_some() { return Err("Duplicate row identity".into()); }
            data.push(Arc::new(Row { key, value }));
        }
        Ok(Self { rows: Arc::new(data), positions: Arc::new(positions) })
    }
    pub fn shared(&self, key: &K) -> Option<&Arc<Row<K, T>>> {
        self.positions.get(key).map(|&i| &self.rows[i])
    }
}
impl<K: Eq + Hash + Clone, T> RowRead<K, T> for RowTable<K, T> {
    fn len(&self)->usize {self.rows.len()}
    fn at(&self,index:usize)->Option<&T>{self.rows.get(index).map(|r|&r.value)}
    fn get(&self, key: &K) -> Option<&T> { self.shared(key).map(|r| &r.value) }
    fn iter(&self) -> Box<dyn Iterator<Item = (K, &T)> + '_> {
        Box::new(self.rows.iter().map(|r| (r.key.clone(), &r.value)))
    }
    fn values(&self) -> Box<dyn Iterator<Item = &T> + '_> { Box::new(self.rows.iter().map(|r|&r.value)) }
}
impl<K: serde::Serialize, T: serde::Serialize> serde::Serialize for RowTable<K, T> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        let mut seq = serializer.serialize_seq(Some(self.rows.len()))?;
        for row in self.rows.iter() { seq.serialize_element(&**row)?; }
        seq.end()
    }
}
impl<'de, K: serde::Deserialize<'de> + Eq + Hash + Clone, T: serde::Deserialize<'de>> serde::Deserialize<'de> for RowTable<K, T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor<K, T>(std::marker::PhantomData<(K, T)>);
        impl<'de, K: serde::Deserialize<'de> + Eq + Hash + Clone, T: serde::Deserialize<'de>> serde::de::Visitor<'de> for Visitor<K, T> {
            type Value = RowTable<K, T>;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { f.write_str("unique typed rows") }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut rows = vec![];
                let mut positions = HashMap::new();
                while let Some(row) = seq.next_element::<Row<K, T>>()? {
                    if positions.insert(row.key.clone(), rows.len()).is_some() { return Err(serde::de::Error::custom("Duplicate row identity")); }
                    rows.push(Arc::new(row));
                }
                Ok(RowTable { rows: Arc::new(rows), positions: Arc::new(positions) })
            }
        }
        deserializer.deserialize_seq(Visitor(std::marker::PhantomData))
    }
}

/// Transitional adapter indexes identities only; original row payloads remain borrowed.
pub struct BorrowedRows<'a, K, T> { rows: Vec<(K, &'a T)>, positions: HashMap<K, usize> }
impl<'a, K: Eq + Hash + Clone, T> BorrowedRows<'a, K, T> {
    pub fn new(rows: impl IntoIterator<Item = (K, &'a T)>) -> Result<Self, String> {
crate::diagnostics::measure("index.rows", crate::diagnostics::Visibility::Detail, || {
        crate::diagnostics::add(crate::diagnostics::Metric::IndexBuilds,1);
        let mut out = Self { rows: vec![], positions: HashMap::new() };
        for (key, row) in rows {
            if out.positions.insert(key.clone(), out.rows.len()).is_some() { return Err("Duplicate row identity".into()); }
            out.rows.push((key, row));crate::diagnostics::add(crate::diagnostics::Metric::IndexRows,1);
        }
        Ok(out)
    
})
}
}
impl<K: Eq + Hash + Clone, T> RowRead<K, T> for BorrowedRows<'_, K, T> {
    fn len(&self)->usize {self.rows.len()}
    fn at(&self,index:usize)->Option<&T>{self.rows.get(index).map(|r|r.1)}
    fn get(&self, key: &K) -> Option<&T> { self.positions.get(key).map(|&i| self.rows[i].1) }
    fn iter(&self) -> Box<dyn Iterator<Item = (K, &T)> + '_> { Box::new(self.rows.iter().map(|(k, v)| (k.clone(), *v))) }
    fn values(&self) -> Box<dyn Iterator<Item = &T> + '_> { Box::new(self.rows.iter().map(|(_,v)|*v)) }
}

/// A copyable read handle supports both published slices and indexed overlays.
/// Returned row references borrow the data, not this short-lived handle.
pub enum RowView<'a,K,T> {
    Indexed(&'a dyn RowRead<K,T>),
    Slice { rows:&'a [T], key:fn(usize,&T)->K },
}
impl<K,T> Copy for RowView<'_,K,T> {}
impl<K,T> Clone for RowView<'_,K,T> { fn clone(&self)->Self {*self} }
impl<'a,K:PartialEq,T> RowView<'a,K,T> {
    pub fn empty()->Self {Self::Slice{rows:&[],key:|_,_|unreachable!("empty rows")}}
    pub fn contains(&self,row:&T)->bool where T:PartialEq {self.values().any(|value|value==row)}
    pub fn get(&self,id:&K)->Option<&'a T> {
        match *self {
            Self::Indexed(rows)=>rows.get(id),
            Self::Slice{rows,key}=>rows.iter().enumerate().find(|(i,r)|key(*i,r)==*id).map(|(_,r)|r),
        }
    }
    pub fn keyed(&self)->Box<dyn Iterator<Item=(K,&'a T)>+'a> {
        match *self {Self::Indexed(rows)=>rows.iter(),Self::Slice{rows,key}=>Box::new(rows.iter().enumerate().map(move |(i,r)|(key(i,r),r)))}
    }
    pub fn values(&self)->Box<dyn Iterator<Item=&'a T>+'a> {
        match *self {Self::Indexed(rows)=>rows.values(),Self::Slice{rows,..}=>Box::new(rows.iter())}
    }
    pub fn iter(&self)->Box<dyn Iterator<Item=&'a T>+'a> {self.values()}
    pub fn len(&self)->usize {match *self {Self::Indexed(rows)=>rows.len(),Self::Slice{rows,..}=>rows.len()}}
    pub fn is_empty(&self)->bool {self.values().next().is_none()}
    pub fn at(&self,index:usize)->Option<&'a T> {match *self {Self::Indexed(rows)=>rows.at(index),Self::Slice{rows,..}=>rows.get(index)}}
}
impl<K:PartialEq,T> RowRead<K,T> for RowView<'_,K,T> {
    fn len(&self)->usize {RowView::len(self)}
    fn at(&self,index:usize)->Option<&T>{RowView::at(self,index)}
    fn get(&self,key:&K)->Option<&T> {RowView::get(self,key)}
    fn iter(&self)->Box<dyn Iterator<Item=(K,&T)>+'_> {RowView::keyed(self)}
    fn values(&self)->Box<dyn Iterator<Item=&T>+'_> {RowView::values(self)}
}
impl<T> RowView<'_,u64,T> {
    pub fn next_key(&self)->Result<u64,String> {
        self.keyed().map(|(k,_)|k).max().map_or(Ok(0),|k|k.checked_add(1).ok_or_else(||"Row ordinal overflow".into()))
    }
}
impl<'a,K:PartialEq,T> IntoIterator for RowView<'a,K,T> {
    type Item=&'a T;
    type IntoIter=Box<dyn Iterator<Item=&'a T>+'a>;
    fn into_iter(self)->Self::IntoIter {self.values()}
}
impl<'a,K:PartialEq,T> IntoIterator for &RowView<'a,K,T> {
    type Item=&'a T;
    type IntoIter=Box<dyn Iterator<Item=&'a T>+'a>;
    fn into_iter(self)->Self::IntoIter {self.values()}
}
impl<K:PartialEq,T:serde::Serialize> serde::Serialize for RowView<'_,K,T> {
    fn serialize<S:serde::Serializer>(&self,serializer:S)->Result<S::Ok,S::Error> {
        use serde::ser::SerializeSeq;
        let mut seq=serializer.serialize_seq(None)?;
        for row in self.values() {seq.serialize_element(row)?;}
        seq.end()
    }
}

/// Read-only helpers can consume either existing slices or a transaction view.
pub trait ReadSequence<T> {
    fn iter(&self)->Box<dyn Iterator<Item=&T>+'_>;
    fn get(&self,index:usize)->Option<&T>;
    fn len(&self)->usize;
    fn is_empty(&self)->bool {self.len()==0}
}
impl<T,const N:usize> ReadSequence<T> for [T;N] {
    fn iter(&self)->Box<dyn Iterator<Item=&T>+'_> {Box::new(self.as_slice().iter())}
    fn get(&self,index:usize)->Option<&T> {self.as_slice().get(index)}
    fn len(&self)->usize {N}
}
impl<T> ReadSequence<T> for [T] {
    fn iter(&self)->Box<dyn Iterator<Item=&T>+'_> {Box::new(<[T]>::iter(self))}
    fn get(&self,index:usize)->Option<&T> {<[T]>::get(self,index)}
    fn len(&self)->usize {<[T]>::len(self)}
}
impl<T> ReadSequence<T> for Vec<T> {
    fn iter(&self)->Box<dyn Iterator<Item=&T>+'_> {Box::new(self.as_slice().iter())}
    fn get(&self,index:usize)->Option<&T> {self.as_slice().get(index)}
    fn len(&self)->usize {Vec::len(self)}
}
impl<K:PartialEq,T> ReadSequence<T> for RowView<'_,K,T> {
    fn iter(&self)->Box<dyn Iterator<Item=&T>+'_> {self.values()}
    fn get(&self,index:usize)->Option<&T> {self.at(index)}
    fn len(&self)->usize {RowView::len(self)}
}
impl<K:PartialEq,T:PartialEq> PartialEq for RowView<'_,K,T> {
    fn eq(&self,other:&Self)->bool {self.values().eq(other.values())}
}
impl<K:PartialEq,T:PartialEq> PartialEq<RowView<'_,K,T>> for Vec<T> {
    fn eq(&self,other:&RowView<'_,K,T>)->bool {self.as_slice().iter().eq(other.values())}
}
impl<K:PartialEq,T:PartialEq> PartialEq<Vec<T>> for RowView<'_,K,T> {
    fn eq(&self,other:&Vec<T>)->bool {self.values().eq(other.as_slice().iter())}
}
