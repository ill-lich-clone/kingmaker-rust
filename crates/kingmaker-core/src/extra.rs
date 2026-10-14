//! Author extensions: shared immutable values, with copy-on-write edits.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{cell::RefCell, collections::{BTreeMap, HashMap, VecDeque}, hash::{Hasher, DefaultHasher}, sync::Arc};

#[derive(Default)]
struct Cache { rows:HashMap<u64,Vec<Arc<Value>>>, fifo:VecDeque<(u64,Arc<Value>,usize)>, bytes:usize }
thread_local! {static CACHE:RefCell<Cache>=RefCell::new(Cache::default());}
struct HashWriter(DefaultHasher);
impl std::io::Write for HashWriter {
    fn write(&mut self,bytes:&[u8])->std::io::Result<usize> {self.0.write(bytes);Ok(bytes.len())}
    fn flush(&mut self)->std::io::Result<()> {Ok(())}
}
fn cost(value:&Value)->usize {
    64+match value {
        Value::String(s)=>s.capacity(),
        Value::Array(a)=>a.capacity()*std::mem::size_of::<Value>()+a.iter().map(cost).sum::<usize>(),
        Value::Object(o)=>o.iter().map(|(k,v)|512+k.capacity()+cost(v)).sum(),
        _=>0,
    }
}
// Number equality alone would merge -0.0 and 0.0. Keep their authored representation.
fn exact(a:&Value,b:&Value)->bool {
    match (a,b) {
        (Value::Number(a),Value::Number(b))=>a.to_string()==b.to_string(),
        (Value::Array(a),Value::Array(b))=>a.len()==b.len() && a.iter().zip(b).all(|(a,b)|exact(a,b)),
        (Value::Object(a),Value::Object(b))=>a.len()==b.len() && a.iter().zip(b).all(|((ka,a),(kb,b))|ka==kb && exact(a,b)),
        _=>a==b,
    }
}
fn shared(value:Value)->Arc<Value> {
    let mut hash=HashWriter(DefaultHasher::new());
    serde_json::to_writer(&mut hash,&value).expect("JSON value hash writer cannot fail");
    let key=hash.0.finish();
    CACHE.with(|cache| {
        let mut cache=cache.borrow_mut();
        if let Some(old)=cache.rows.get(&key).and_then(|rows|rows.iter().find(|old|exact(old,&value))) {return old.clone();}
        let bytes=512+cost(&value);
        let value=Arc::new(value);
        const LIMIT:usize=16*1024*1024;
        if bytes<=LIMIT {
            while cache.bytes>LIMIT-bytes {
                let (key,old,bytes)=cache.fifo.pop_front().expect("extension cache accounting");
                let rows=cache.rows.get_mut(&key).unwrap();
                rows.retain(|row|!Arc::ptr_eq(row,&old));
                if rows.is_empty() {cache.rows.remove(&key);}
                cache.bytes-=bytes;
            }
            cache.rows.entry(key).or_default().push(value.clone());
            cache.fifo.push_back((key,value.clone(),bytes));cache.bytes+=bytes;
        }
        value
    })
}
#[derive(Debug,Clone,Default,PartialEq)]
pub struct Extra(Arc<Vec<(Arc<str>,Arc<Value>)>>);
impl Extra {
    pub fn entry(&mut self,key:String)->Entry<'_> {Entry {map:self,key}}
    pub fn new()->Self {Self::default()}
    pub fn clear(&mut self) {self.0=Arc::new(vec![]);}
    pub fn len(&self)->usize {self.0.len()}
    pub fn is_empty(&self)->bool {self.0.is_empty()}
    fn position(&self,key:&str)->Result<usize,usize> {self.0.binary_search_by(|(k,_)|k.as_ref().cmp(key))}
    pub fn get(&self,key:&str)->Option<&Value> {self.position(key).ok().map(|i|&*self.0[i].1)}
    pub fn contains_key(&self,key:&str)->bool {self.position(key).is_ok()}
    pub fn get_mut(&mut self,key:&str)->Option<&mut Value> {
        let i=self.position(key).ok()?;
        Some(Arc::make_mut(&mut Arc::make_mut(&mut self.0)[i].1))
    }
    pub fn insert(&mut self,key:String,value:Value)->Option<Value> {
        let position=self.position(&key);
        let rows=Arc::make_mut(&mut self.0);
        let value=shared(value);
        match position {
            Ok(i)=>Some(owned(std::mem::replace(&mut rows[i].1,value))),
            Err(i)=>{rows.insert(i,(crate::identity::shared_text(&key),value));None},
        }
    }
    pub fn remove(&mut self,key:&str)->Option<Value> {
        let i=self.position(key).ok()?;
        Some(owned(Arc::make_mut(&mut self.0).remove(i).1))
    }
    pub fn iter(&self)->impl DoubleEndedIterator<Item=(&str,&Value)> {self.0.iter().map(|(k,v)|(k.as_ref(),v.as_ref()))}
    pub fn keys(&self)->impl Iterator<Item=&str> {self.0.iter().map(|(k,_)|k.as_ref())}
    pub fn values(&self)->impl Iterator<Item=&Value> {self.0.iter().map(|(_,v)|v.as_ref())}
    pub fn extend(&mut self,rows:impl IntoIterator<Item=(String,Value)>) {for (k,v) in rows {self.insert(k,v);}}
}
fn owned(value:Arc<Value>)->Value {Arc::try_unwrap(value).unwrap_or_else(|v|(*v).clone())}
impl FromIterator<(String,Value)> for Extra {
    fn from_iter<I:IntoIterator<Item=(String,Value)>>(rows:I)->Self {
        // BTreeMap preserves the previous last-write-wins and lexicographic key contract.
        let rows:BTreeMap<_,_>=rows.into_iter().collect();
        Self(Arc::new(rows.into_iter().map(|(k,v)|(crate::identity::shared_text(&k),shared(v))).collect()))
    }
}
impl<const N:usize> From<[(String,Value);N]> for Extra {fn from(rows:[(String,Value);N])->Self {rows.into_iter().collect()}}
impl From<BTreeMap<String,Value>> for Extra {fn from(rows:BTreeMap<String,Value>)->Self {rows.into_iter().collect()}}
impl std::ops::Index<&str> for Extra {type Output=Value;fn index(&self,key:&str)->&Value {self.get(key).expect("extension key absent")}}
impl std::ops::IndexMut<&str> for Extra {fn index_mut(&mut self,key:&str)->&mut Value {self.get_mut(key).expect("extension key absent")}}
impl IntoIterator for Extra {
    type Item=(String,Value);
    type IntoIter=std::vec::IntoIter<Self::Item>;
    fn into_iter(self)->Self::IntoIter {
        Arc::try_unwrap(self.0).unwrap_or_else(|rows|(*rows).clone()).into_iter().map(|(k,v)|(k.to_string(),owned(v))).collect::<Vec<_>>().into_iter()
    }
}
impl<'a> IntoIterator for &'a Extra {
    type Item=(&'a str,&'a Value);
    type IntoIter=Box<dyn Iterator<Item=Self::Item>+'a>;
    fn into_iter(self)->Self::IntoIter {Box::new(self.iter())}
}
impl Serialize for Extra {
    fn serialize<S:serde::Serializer>(&self,serializer:S)->Result<S::Ok,S::Error> {
        use serde::ser::SerializeMap;
        let mut map=serializer.serialize_map(Some(self.len()))?;
        for (k,v) in self.iter() {map.serialize_entry(k,v)?;}
        map.end()
    }
}
impl<'de> Deserialize<'de> for Extra {
    fn deserialize<D:serde::Deserializer<'de>>(deserializer:D)->Result<Self,D::Error> {Ok(BTreeMap::<String,Value>::deserialize(deserializer)?.into())}
}

pub struct Entry<'a> {map:&'a mut Extra,key:String}
impl<'a> Entry<'a> {
    pub fn or_insert(self,value:Value)->&'a mut Value {
        if !self.map.contains_key(&self.key) {self.map.insert(self.key.clone(),value);}
        self.map.get_mut(&self.key).unwrap()
    }
}
