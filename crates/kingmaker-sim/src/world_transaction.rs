//! One row overlay with nested checkpoints; detached changes validate before publication.
use crate::{operation_session::{OpResult, OperationError}, read_guard::*};
use kingmaker_core::{row_store::{RowRead, RowView, BorrowedRows}, world_read::{WorldRead, BorrowedWorld}, WorldState};
use serde::Serialize;
use std::{cell::RefCell, collections::HashMap, hash::Hash, sync::{Arc, atomic::{AtomicU64, Ordering}}};
fn error(s:&str) -> OperationError { OperationError(s.into()) }
#[derive(Debug)]
pub(crate) struct LocalPrepareToken { owner:u64, mutation_epoch:u64 }
static NEXT_TRANSACTION: AtomicU64 = AtomicU64::new(1);
pub(crate) fn execute_atomic<R>(world:&mut WorldState,run:impl FnOnce(&mut WorldTransaction<'_>)->OpResult<R>)->OpResult<R> {
    let base=BorrowedWorld::new(world).map_err(OperationError)?;
    let mut transaction=WorldTransaction::from_borrowed(&base);
    let result=run(&mut transaction)?;
    let delta=transaction.finish()?;
    apply_world_delta(world,delta)?;
    Ok(result)
}

#[derive(Clone)]
struct RowOrder<K> { keys:Vec<K>, members:std::collections::HashSet<K> }
#[derive(Clone)]
struct RowDelta<K,T> { writes:HashMap<K,Option<Arc<T>>>, order:Vec<K>, replacement:Option<RowOrder<K>> }
impl<K,T> Default for RowDelta<K,T> {
    fn default() -> Self { Self { writes:HashMap::new(), order:vec![], replacement:None } }
}
struct Overlay<'a,K,T> {
    base:RowView<'a,K,T>,
    base_index:Option<BorrowedRows<'a,K,T>>,
    delta:RowDelta<K,T>,
    guards:RefCell<RowGuards<K>>,
    positions:std::cell::OnceCell<Vec<K>>,
    content:std::cell::OnceCell<Result<Fingerprint,String>>,
    parent_content:Option<&'a std::cell::OnceCell<Result<Fingerprint,String>>>,
    cacheable:bool,
}
impl<'a,K: Clone+Eq+Hash+Serialize,T:Serialize> Overlay<'a,K,T> {
    fn new(base:RowView<'a,K,T>) -> Self { Self {base_index:slice_index(base),base, delta:Default::default(), guards:RefCell::new(Default::default()), positions:Default::default(), content:Default::default(),parent_content:None,cacheable:false} }
    fn child(parent:&'a Overlay<'_,K,T>)->Self {
        let mut child=Self::new(RowView::Indexed(parent));
        child.cacheable=parent.cacheable;
        if parent.cacheable {child.parent_content=Some(&parent.content);}
        child
    }
    fn scan(&self) {
        let base=self.base_view();let mut guards=self.guards.borrow_mut();
        if let Some(cache)=self.parent_content {guards.scan_owned(&base,cache);} else {guards.scan(&base);}
    }
    fn base_view(&self)->RowView<'_,K,T> {self.base_index.as_ref().map_or(self.base,|rows|RowView::Indexed(rows))}
    fn write(&mut self,key:K,value:Option<T>) {
        if self.get(&key).is_some()!=value.is_some() {self.positions.take();}
        self.content.take();
        self.guards.borrow_mut().read(&self.base_view(),&key);
        if let Some(order)=&mut self.delta.replacement {
            if value.is_some() && order.members.insert(key.clone()) {order.keys.push(key.clone());}
        }
        if !self.delta.writes.contains_key(&key) { self.delta.order.push(key.clone()); }
        self.delta.writes.insert(key,value.map(Arc::new));
    }
    fn edit(&mut self,key:&K)->OpResult<&mut T> where T:Clone {
        self.content.take();
        self.guards.borrow_mut().read(&self.base_view(),key);
        if !self.delta.writes.contains_key(key) {
            let row=self.base_view().get(key).ok_or_else(||error("Missing edited row"))?.clone();
            self.delta.order.push(key.clone());self.delta.writes.insert(key.clone(),Some(Arc::new(row)));
        }
        self.delta.writes.get_mut(key).and_then(Option::as_mut).map(Arc::make_mut).ok_or_else(||error("Missing edited row"))
    }
    fn reorder(&mut self,keys:Vec<K>)->OpResult<()> {
        self.positions.take();self.content.take();
        let members:std::collections::HashSet<_>=keys.iter().cloned().collect();
        let actual:std::collections::HashSet<_>=self.iter().map(|(key,_)|key).collect();
        if members.len()!=keys.len() || members!=actual {return Err(error("Reorder changes row membership"));}
        self.delta.replacement=Some(RowOrder{keys,members});Ok(())
    }
    fn apply(&mut self, mut delta:RowDelta<K,T>) {
        if delta.writes.is_empty() && delta.replacement.is_none() {return;}
        if delta.replacement.is_some() || delta.writes.iter().any(|(k,v)|self.get(k).is_some()!=v.is_some()) {self.positions.take();}
        self.content.take();
        for key in delta.order {
            self.guards.borrow_mut().read(&self.base_view(),&key);
            let value=delta.writes.remove(&key).expect("ordered delta");
            if let Some(order)=&mut self.delta.replacement {
                if value.is_some() && order.members.insert(key.clone()) {order.keys.push(key.clone());}
            }
            if !self.delta.writes.contains_key(&key) {self.delta.order.push(key.clone());}
            self.delta.writes.insert(key,value);
        }
        if let Some(order)=delta.replacement {self.delta.replacement=Some(order);}
    }
    fn replace(&mut self, rows:Vec<T>, key:impl Fn(usize,&T)->K)->OpResult<()> where T:PartialEq {
        self.positions.take();self.content.take();
        let rows:Vec<_>=rows.into_iter().enumerate().map(|(i,r)|(key(i,&r),r)).collect();
        let members:std::collections::HashSet<_>=rows.iter().map(|(k,_)|k.clone()).collect();
        if members.len()!=rows.len() {return Err(error("Duplicate replacement row identity"));}
        self.scan();
        let keys=rows.iter().map(|(k,_)|k.clone()).collect();
        let removed:Vec<_>=self.iter().map(|(k,_)|k).filter(|k|!members.contains(k)).collect();
        for key in removed {self.write(key,None);}
        for (key,row) in rows {if self.get(&key)!=Some(&row) {self.write(key,Some(row));}}
        self.delta.replacement=Some(RowOrder{keys,members});
        Ok(())
    }
}
fn merged<'a,K: Clone+Eq+Hash,T>(base:RowView<'a,K,T>,delta:&'a RowDelta<K,T>) -> Box<dyn Iterator<Item=(K,&'a T)>+'a> {
    if let Some(order)=&delta.replacement {
        return Box::new(order.keys.iter().filter_map(move |key| {
            let value=match delta.writes.get(key) {Some(v)=>v.as_deref(),None=>base.get(key)};
            value.map(|value|(key.clone(),value))
        }));
    }
    Box::new(base.keyed().filter_map(move |(key,row)| match delta.writes.get(&key) {
        Some(value) => value.as_deref().map(|value| (key,value)),
        None => Some((key,row)),
    }).chain(delta.order.iter().filter_map(move |key| {
        if base.get(key).is_some() { None } else { delta.writes[key].as_deref().map(|value|(key.clone(),value)) }
    })))
}
impl<K:Clone+Eq+Hash+Serialize,T:Serialize> RowRead<K,T> for Overlay<'_,K,T> {
    fn len(&self)->usize {self.positions.get_or_init(||self.iter().map(|(k,_)|k).collect()).len()}
    fn at(&self,index:usize)->Option<&T> {
        let keys=self.positions.get_or_init(||self.iter().map(|(k,_)|k).collect());
        keys.get(index).and_then(|k|self.get(k))
    }
    fn values(&self)->Box<dyn Iterator<Item=&T>+'_> {Box::new(self.iter().map(|(_,v)|v))}
    fn get(&self,key:&K) -> Option<&T> {
        self.guards.borrow_mut().read(&self.base_view(),key);
        match self.delta.writes.get(&key) { Some(v) => v.as_deref(), None => self.base_view().get(key) }
    }
    fn iter(&self) -> Box<dyn Iterator<Item=(K,&T)>+'_> {
        self.scan();
        merged(self.base_view(),&self.delta)
    }
}
fn slice_index<'a,K:Clone+Eq+Hash,T>(base:RowView<'a,K,T>)->Option<BorrowedRows<'a,K,T>> {
    match base {RowView::Slice{..}=>BorrowedRows::new(base.keyed()).ok(),_=>None}
}
struct PreviewRows<'a,K,T> { base:RowView<'a,K,T>, base_index:Option<BorrowedRows<'a,K,T>>, delta:&'a RowDelta<K,T>, positions:std::cell::OnceCell<Vec<K>> }
impl<'a,K:Clone+Eq+Hash,T> PreviewRows<'a,K,T> {
    fn new(base:RowView<'a,K,T>,delta:&'a RowDelta<K,T>)->Self {Self{base_index:slice_index(base),base,delta,positions:Default::default()}}
    fn base_view(&self)->RowView<'_,K,T> {self.base_index.as_ref().map_or(self.base,|rows|RowView::Indexed(rows))}
}
impl<K:Clone+Eq+Hash,T> RowRead<K,T> for PreviewRows<'_,K,T> {
    fn len(&self)->usize {self.positions.get_or_init(||self.iter().map(|(k,_)|k).collect()).len()}
    fn at(&self,index:usize)->Option<&T> {
        let keys=self.positions.get_or_init(||self.iter().map(|(k,_)|k).collect());
        keys.get(index).and_then(|k|self.get(k))
    }
    fn values(&self)->Box<dyn Iterator<Item=&T>+'_> {Box::new(self.iter().map(|(_,v)|v))}
    fn get(&self,key:&K) -> Option<&T> { match self.delta.writes.get(&key) {Some(v)=>v.as_deref(),None=>self.base_view().get(key)} }
    fn iter(&self) -> Box<dyn Iterator<Item=(K,&T)>+'_> { merged(self.base_view(),self.delta) }
}
struct Single<'a,T> { base:&'a T, value:Option<Arc<T>>, guard:RefCell<Option<Result<Fingerprint,String>>> }
impl<'a,T:Serialize> Single<'a,T> {
    fn new(base:&'a T) -> Self { Self {base,value:None,guard:RefCell::new(None)} }
    fn get(&self) -> &T {
        if self.guard.borrow().is_none() { *self.guard.borrow_mut()=Some(fingerprint(self.base).map_err(|e|e.to_string())); }
        self.value.as_deref().unwrap_or(self.base)
    }
    fn edit(&mut self)->&mut T where T:Clone {
        self.get();if self.value.is_none() {self.value=Some(Arc::new(self.base.clone()));}
        Arc::make_mut(self.value.as_mut().unwrap())
    }
    fn set(&mut self,value:T) { self.get(); self.value=Some(Arc::new(value)); }
}

// SourceSnapshot contains raw backing metadata and is intentionally outside JSON guards.
// Its immutable Arc identity is checked separately by the enclosing transaction.
struct CommitRows<T> { replacements:HashMap<usize,Option<T>>, appends:Vec<T>, output:Vec<T>, order:Option<Vec<usize>>, slots:Vec<Option<T>> }
impl<T:Clone> CommitRows<T> {
    fn prepare<K:Clone+Eq+Hash>(rows:&[T],delta:RowDelta<K,T>,key:impl Fn(usize,&T)->K,ordinal:bool) -> OpResult<Option<Self>> {
        if delta.writes.is_empty() && delta.replacement.is_none() { return Ok(None); }
        let mut positions:HashMap<K,usize> = HashMap::with_capacity(rows.len());
        for (i,row) in rows.iter().enumerate() {crate::run_control::work("commit.prepare")?;positions.insert(key(i,row),i);}
        let mut writes = delta.writes;
        let mut replacements = HashMap::new();
        let mut appends = vec![];
        for id in delta.order {
            crate::run_control::work("commit.prepare")?;
            let value = writes.remove(&id).expect("delta order");
            if let Some(value) = &value {
                if !ordinal && key(0,value) != id { return Err(error("Row identity differs from registry key")); }
            }
            let value = value.map(|v| Arc::try_unwrap(v).unwrap_or_else(|v| (*v).clone()));
            if let Some(&i) = positions.get(&id) { replacements.insert(i,value); }
            else if let Some(value) = value { positions.insert(id,rows.len()+appends.len());appends.push(value); }
        }
        let output = Vec::with_capacity(rows.len()+appends.len());
        let order=if let Some(o)=delta.replacement {
            let mut order=Vec::with_capacity(o.keys.len());
            for k in o.keys {crate::run_control::work("commit.prepare")?;if let Some(&i)=positions.get(&k) {if !replacements.get(&i).is_some_and(Option::is_none) {order.push(i);}}}
            Some(order)
        } else {None};
        let slots=if order.is_some() {Vec::with_capacity(rows.len()+appends.len())} else {vec![]};
        Ok(Some(Self { replacements, appends, output, order, slots }))
    }
    fn publish(mut self, rows:&mut Vec<T>) {
        if let Some(order)=self.order {
            for (i,row) in std::mem::take(rows).into_iter().enumerate() {
                self.slots.push(self.replacements.remove(&i).unwrap_or(Some(row)));
            }
            self.slots.extend(self.appends.into_iter().map(Some));
            for i in order {self.output.push(self.slots[i].take().expect("validated row order"));}
            *rows=self.output;
            return;
        }
        for (i,row) in std::mem::take(rows).into_iter().enumerate() {
            match self.replacements.remove(&i) { Some(Some(r))=>self.output.push(r),Some(None)=>{},None=>self.output.push(row) }
        }
        self.output.append(&mut self.appends);
        *rows=self.output;
    }
}
macro_rules! define_transaction {
    (rows { $(($name:ident, $put:ident, $remove:ident, $replace:ident, $key:ty, $ty:ty, [$($field:ident),+], |$i:ident, $r:ident| $identity:expr),)* }
     singletons { $(($single:ident, $set:ident, $sty:ty, [$($sfield:ident),+]),)* }) => {
        pub struct WorldTransaction<'a> {
            poisoned:bool,
            owner:u64, mutation_epoch:u64, next_mark:u64, valid_marks:std::collections::HashSet<u64>, next_id:Option<u32>,
            source:Option<Arc<kingmaker_core::SourceSnapshot>>,
            history:Option<kingmaker_core::history::HistoryBacking>,
            $($name:Overlay<'a,$key,$ty>,)*
            $($single:Single<'a,$sty>,)*
        }
        pub struct Savepoint {
            poisoned:bool,
            owner:u64, mark:u64, next_id:Option<u32>,
            $($name:RowDelta<$key,$ty>,)*
            $($single:Option<Arc<$sty>>,)*
        }
        pub struct PreparedWorldDelta {
            source:Option<Arc<kingmaker_core::SourceSnapshot>>,
            history:Option<kingmaker_core::history::HistoryBacking>,
            $($name:(RowDelta<$key,$ty>,RowGuards<$key>),)*
            $($single:(Option<Arc<$sty>>,Option<Result<Fingerprint,String>>),)*
        }
        pub struct PreparedWorldView<'a> {
            source:&'a Option<Arc<kingmaker_core::SourceSnapshot>>,
            history:&'a Option<kingmaker_core::history::HistoryBacking>,
            $($name:PreviewRows<'a,$key,$ty>,)*
            $($single:&'a $sty,)*
        }
        impl<'a> WorldTransaction<'a> {
            pub fn new(base:&'a dyn WorldRead) -> Self {
                let tx = Self {
                    poisoned:false,
                    owner:NEXT_TRANSACTION.fetch_add(1,Ordering::Relaxed),mutation_epoch:0,next_mark:0,valid_marks:Default::default(),next_id:None,
                    source:base.source().clone(),
                    history:base.history().clone(),
                    $($name:Overlay::new(base.$name()),)*
                    $($single:Single::new(base.$single()),)*
                };
                tx.campaign.get();
                tx
            }
            pub(crate) fn from_borrowed(base:&'a BorrowedWorld<'_>)->Self {
                let mut tx=Self::new(base);$(tx.$name.cacheable=true;)*tx
            }
            pub(crate) fn child(&self)->WorldTransaction<'_> {
                let mut tx=WorldTransaction::new(self);
                $(tx.$name=Overlay::child(&self.$name);)*tx
            }
            pub(crate) fn local_prepare_token(&self)->LocalPrepareToken {LocalPrepareToken{owner:self.owner,mutation_epoch:self.mutation_epoch}}
            pub(crate) fn validate_local_prepare(&self,token:&LocalPrepareToken)->OpResult<()> {
                if self.poisoned || token.owner!=self.owner || token.mutation_epoch!=self.mutation_epoch {return Err(error("Stale local preparation"));}
                Ok(())
            }
            fn touch(&mut self) {self.mutation_epoch=self.mutation_epoch.checked_add(1).expect("transaction epoch exhausted");}
            $(pub fn $put(&mut self,key:$key,value:$ty)->OpResult<()> {self.touch();self.$name.write(key,Some(value));Ok(())}
              pub fn $remove(&mut self,key:$key)->OpResult<()> {self.touch();self.$name.write(key,None);Ok(())})*
            $(pub fn $replace(&mut self,rows:Vec<$ty>)->OpResult<()> {self.touch();
                let result=self.$name.replace(rows,|$i,$r| {let _=(&$i,&$r);$identity});
                if result.is_err() {self.poison();}
                result
            })*
            $(pub fn $set(&mut self,value:$sty)->OpResult<()> {self.touch();self.$single.set(value);Ok(())})*
            // Hiring forecasts reserve cash privately; real funding is applied through OperationSession.
            pub(crate) fn discard_forecast_accounts(&mut self) {self.touch();self.accounts.delta=Default::default();self.accounts.positions.take();self.accounts.content.take();}
            pub(crate) fn refresh_enterprise_views(&mut self)->OpResult<()> {self.touch();
                let rows=(self as &dyn WorldRead).derive_enterprise_views(self.accounts().iter(),self.factories().iter(),0).map_err(OperationError)?;
                self.replace_enterprises(rows)
            }
            pub fn poison(&mut self) {self.touch();self.poisoned=true;}
            pub fn apply_phase(&mut self,changes:crate::operation_session::PreparedChanges)->OpResult<()> {self.touch();
                if self.poisoned {return Err(error("Poisoned world transaction"));}
                let result=crate::operation_session::apply_prepared_phase(self,changes);
                if result.is_err() {self.poison();}
                result
            }
            pub(crate) fn adopt_projection(&mut self,base:&dyn WorldRead,delta:PreparedWorldDelta,fields:&[&str])->OpResult<()> {self.touch();
                delta.validate(base)?;
                $(if fields.contains(&stringify!($name)) {self.$name.apply(delta.$name.0);})*
                $(if fields.contains(&stringify!($single)) {if let Some(value)=delta.$single.0 {self.$single.get();self.$single.value=Some(value);}})*
                Ok(())
            }
            pub(crate) fn edit_pop_mobility(&mut self)->&mut Option<serde_json::Value> {self.touch();self.pop_mobility.edit()}
            pub(crate) fn edit_needs(&mut self)->&mut Option<kingmaker_core::market::NeedsData> {self.touch();self.needs.edit()}
            pub(crate) fn edit_pop(&mut self,id:&kingmaker_core::ExternalId)->OpResult<&mut kingmaker_core::PopRecord> {self.touch();self.pops.edit(id)}
            pub(crate) fn reorder_pops(&mut self,keys:Vec<kingmaker_core::ExternalId>)->OpResult<()> {self.touch();self.pops.reorder(keys)}
            pub fn apply_delta(&mut self,delta:PreparedWorldDelta)->OpResult<()> {self.touch();
                if self.poisoned {return Err(error("Poisoned world transaction"));}
                if let Err(e)=delta.validate_with_owner(self,Some(self)) {self.poison();return Err(e);}
                $(self.$name.apply(delta.$name.0);)*
                $(if let Some(value)=delta.$single.0 {self.$single.get();self.$single.value=Some(value);})*
                Ok(())
            }
            pub fn savepoint(&mut self) -> Savepoint {self.touch();
                self.next_mark+=1;
                self.valid_marks.insert(self.next_mark);
                Savepoint {owner:self.owner,mark:self.next_mark,next_id:self.next_id,poisoned:self.poisoned,
                    $($name:self.$name.delta.clone(),)*
                    $($single:self.$single.value.clone(),)*
                }
            }
            pub fn rollback_to(&mut self,mark:Savepoint) -> OpResult<()> {self.touch();
                if mark.owner!=self.owner || !self.valid_marks.contains(&mark.mark) {return Err(error("Invalid transaction savepoint"));}
                self.valid_marks.retain(|m|*m<mark.mark);
                self.next_id=mark.next_id;
                self.poisoned=mark.poisoned;
                $(self.$name.delta=mark.$name;self.$name.positions.take();self.$name.content.take();)*
                $(self.$single.value=mark.$single;)*
                // Reads in discarded branches remain conservative guards, never economic mutations.
                Ok(())
            }
            pub fn allocate_id(&mut self) -> OpResult<u32> {self.touch();
                let next = match self.next_id {
                    Some(next)=>next,
                    None=> {
                        let max = self.accounts.iter().map(|(_,r)|u64::from(r.id.0))
                            .chain(self.stockpiles.iter().map(|(_,r)|u64::from(r.id)))
                            .chain(self.pops.iter().map(|(_,r)|u64::from(r.id.0)))
                            .chain(self.enterprises.iter().map(|(_,r)|u64::from(r.id.0)))
                            .chain(self.source.iter().flat_map(|s|s.max_dictionary_id())).max().unwrap_or(0);
                        u32::try_from(max.checked_add(1).ok_or_else(||error("ID overflow"))?).map_err(|_|error("ID overflow"))?
                    }
                };
                self.next_id=Some(next.checked_add(1).ok_or_else(||error("ID overflow"))?);
                Ok(next)
            }
            pub fn finish(self) -> OpResult<PreparedWorldDelta> {
                if self.poisoned {return Err(error("Poisoned world transaction"));}
                $(if let Some(e)=&self.$name.guards.borrow().error {return Err(OperationError(e.clone()));})*
                $(if let Some(Err(e))=&*self.$single.guard.borrow() {return Err(OperationError(e.clone()));})*
                Ok(PreparedWorldDelta { source:self.source, history:self.history,
                    $($name:(self.$name.delta,self.$name.guards.into_inner()),)*
                    $($single:(self.$single.value,self.$single.guard.into_inner()),)*
                })
            }
        }
        impl WorldRead for WorldTransaction<'_> {
            fn source(&self)->&Option<Arc<kingmaker_core::SourceSnapshot>> {&self.source}
            fn history(&self)->&Option<kingmaker_core::history::HistoryBacking> {&self.history}
            $(fn $name(&self)->RowView<'_,$key,$ty> {RowView::Indexed(&self.$name)})*
            $(fn $single(&self)->&$sty {self.$single.get()})*
        }
        impl PreparedWorldDelta {
            pub fn view<'a>(&'a self,base:&'a dyn WorldRead)->PreparedWorldView<'a> {
                PreparedWorldView {source:&self.source,history:&self.history,
                    $($name:PreviewRows::new(base.$name(),&self.$name.0),)*
                    $($single:self.$single.0.as_deref().unwrap_or_else(||base.$single()),)*
                }
            }
            fn validate(&self,base:&dyn WorldRead)->OpResult<()> {self.validate_with_owner(base,None)}
            fn validate_with_owner(&self,base:&dyn WorldRead,owner:Option<&WorldTransaction<'_>>)->OpResult<()> {
                if self.history.as_ref().map(|h|h.token())!=base.history().as_ref().map(|h|h.token()) {return Err(error("Stale history backing"));}
                match (&self.source,base.source()) {
                    (None,None)=>{},(Some(a),Some(b)) if Arc::ptr_eq(a,b)=>{},_=>return Err(error("Stale source backing")),
                }
                $(match owner.filter(|tx|tx.$name.cacheable) {
                    Some(tx)=>self.$name.1.validate_owned(&tx.$name,&tx.$name.content)?,
                    None=>self.$name.1.validate(&base.$name())?,
                })*
                $(if let Some(hash)=&self.$single.1 {
                    let hash=hash.as_ref().map_err(|e|OperationError(e.clone()))?;
                    if fingerprint(base.$single())?!=*hash {return Err(error("Stale singleton"));}
                })*
                Ok(())
            }
        }
        impl WorldRead for PreparedWorldView<'_> {
            fn source(&self)->&Option<Arc<kingmaker_core::SourceSnapshot>> {self.source}
            fn history(&self)->&Option<kingmaker_core::history::HistoryBacking> {self.history}
            $(fn $name(&self)->RowView<'_,$key,$ty> {RowView::Indexed(&self.$name)})*
            $(fn $single(&self)->&$sty {self.$single})*
        }
        pub fn apply_world_delta(world:&mut WorldState,delta:PreparedWorldDelta)->OpResult<()> {
            crate::run_control::check("publish")?;
            let base=BorrowedWorld::new(world).map_err(OperationError)?;
            delta.validate(&base)?;
            let preview=delta.view(&base);
            let mut account_index=HashMap::new();
            for (i,a) in preview.accounts().iter().enumerate() {
                crate::run_control::work("commit.accounts")?;
                if account_index.insert(a.address(),i).is_some() {return Err(error("Duplicate financial address"));}
            }
            drop(preview);
            drop(base);
            // Allocate all replacement rows, roots and indexes before touching published values.
            $(let $name=CommitRows::prepare(&world.$($field).+,delta.$name.0,|$i,$r| {let _=(&$i,&$r);$identity},stringify!($key)=="u64")?;)*
            $(let $single=delta.$single.0.map(|v|Arc::try_unwrap(v).unwrap_or_else(|v|(*v).clone()));)*
            // Last fallible check: cancellation after this point waits for the atomic publication.
            crate::run_control::check("publish")?;
            $(if let Some(rows)=$name {rows.publish(&mut world.$($field).+);})*
            $(if let Some(value)=$single {world.$($sfield).+ = value;})*
            world.account_index=account_index;
            Ok(())
        }
    }
}
kingmaker_core::world_fields!(define_transaction);

/// Reorder only keys; POP payloads stay shared while province ranges are recalculated.
pub(crate) fn regroup_population(state:&mut WorldTransaction<'_>,base:&dyn WorldRead,children:Option<&HashMap<kingmaker_core::ExternalId,kingmaker_core::ExternalId>>)->OpResult<()> {
    let ranks:HashMap<_,_>=state.provinces().iter().enumerate().map(|(i,p)|(p.id,i)).collect();
    let metadata:HashMap<_,_>=state.pops().iter().map(|p|(p.external_id.clone(),p.province_id)).collect();
    if metadata.len()!=state.pops().len() || metadata.len()>u32::MAX as usize || metadata.values().any(|p|!ranks.contains_key(p)) {return Err(error("Invalid population province/ranges"));}
    let mut keys:Vec<_>=if let Some(children)=children {
        let mut keys=Vec::with_capacity(metadata.len());
        for p in base.pops().iter() {
            if !metadata.contains_key(&p.external_id) {return Err(error("Source POP vanished"));}
            keys.push(p.external_id.clone());
            if let Some(id)=children.get(&p.external_id) {
                if !metadata.contains_key(id) {return Err(error("Child POP vanished"));}keys.push(id.clone());
            }
        }
        if keys.len()!=metadata.len() {return Err(error("Unexplained POP addition"));} keys
    } else {state.pops().keyed().map(|(key,_)|key).collect()};
    keys.sort_by_key(|id|ranks[&metadata[id]]);
    let mut cursor=0;let mut provinces=vec![];
    for p in state.provinces().iter() {
        let start=cursor;while cursor<keys.len() && metadata[&keys[cursor]]==p.id {cursor+=1;}
        let range=(start as u32,cursor as u32);
        if p.pop_range!=range {let mut p=p.clone();p.pop_range=range;provinces.push(p);}
    }
    state.reorder_pops(keys)?;
    for p in provinces {state.put_provinces(p.id,p)?;}
    Ok(())
}

#[cfg(test)]
mod remediation_guard_tests {
    use super::*;
    #[test]
    fn remediation_cancel_after_commit_preparation_keeps_world() {
        let mut w=WorldState::default();w.pops.push(kingmaker_core::PopRecord{external_id:"p".into(),..Default::default()});
        let mut tx=WorldTransaction::new(&w);let mut pop=w.pops[0].clone();pop.size=99;tx.put_pops("p".into(),pop).unwrap();
        let delta=tx.finish().unwrap();let before=serde_json::to_value(&w).unwrap();
        let checks=Arc::new(std::sync::atomic::AtomicUsize::new(0));let count=checks.clone();
        let control=crate::run_control::RunControl::new(move |label| {
            if label=="publish" && count.fetch_add(1,Ordering::Relaxed)==1 {return Err(error("STOP at final publication gate"));}Ok(())
        });
        assert!(control.run(||apply_world_delta(&mut w,delta)).is_err());
        assert_eq!(checks.load(Ordering::Relaxed),2);assert_eq!(serde_json::to_value(w).unwrap(),before);
    }
    #[test]
    fn remediation_external_rows_never_receive_owned_cache() {
        use std::cell::Cell;
        struct Switching {a:[u64;1],b:[u64;1],changed:Cell<bool>}
        impl RowRead<u64,u64> for Switching {
            fn len(&self)->usize{1}
            fn at(&self,i:usize)->Option<&u64>{if self.changed.get(){self.b.get(i)}else{self.a.get(i)}}
            fn get(&self,k:&u64)->Option<&u64>{if *k==0{self.at(0)}else{None}}
            fn iter(&self)->Box<dyn Iterator<Item=(u64,&u64)>+'_>{Box::new(std::iter::once((0,self.at(0).unwrap())))}
            fn values(&self)->Box<dyn Iterator<Item=&u64>+'_>{Box::new(std::iter::once(self.at(0).unwrap()))}
        }
        let rows=Switching{a:[1],b:[2],changed:Cell::new(false)};
        let parent=Overlay::new(RowView::Indexed(&rows));
        let first=selection(&Overlay::child(&parent)).unwrap();
        rows.changed.set(true);
        let second=selection(&Overlay::child(&parent)).unwrap();
        assert_ne!(first,second);assert!(parent.content.get().is_none());
        assert!(parent.guards.borrow().validate(&RowView::Indexed(&rows)).is_err());
    }
    #[test]
    fn remediation_slice_overlay_and_preview_linear() {
        use std::sync::atomic::{AtomicUsize,Ordering};
        static KEYS:AtomicUsize=AtomicUsize::new(0);
        fn key(_:usize,v:&u64)->u64 {KEYS.fetch_add(1,Ordering::Relaxed);*v}
        for n in [1000,2000,4000] {
            let rows:Vec<u64>=(0..n).collect();KEYS.store(0,Ordering::Relaxed);
            let base=RowView::Slice{rows:&rows,key};
            let overlay=Overlay::new(base);
            for i in 0..n {assert_eq!(overlay.at(i as usize),Some(&i));}
            assert!(KEYS.load(Ordering::Relaxed)<=8*n as usize,"overlay {}",KEYS.load(Ordering::Relaxed));
            KEYS.store(0,Ordering::Relaxed);
            let delta=RowDelta::default();
            let preview=PreviewRows::new(base,&delta);
            for i in 0..n {assert_eq!(preview.at(i as usize),Some(&i));}
            assert!(KEYS.load(Ordering::Relaxed)<=8*n as usize,"preview {}",KEYS.load(Ordering::Relaxed));
        }
    }
    #[test]
    fn remediation_payload_write_keeps_position_cache() {
        let rows=kingmaker_core::row_store::RowTable::from_rows((0..1000u64).map(|i|(i,i))).unwrap();
        let mut overlay=Overlay::new(RowView::Indexed(&rows));
        for i in 0..1000 {
            assert_eq!(overlay.at(i as usize),Some(&i));
            overlay.write(i,Some(i+1));
            assert!(overlay.positions.get().is_some(),"payload replacement rebuilt positions");
        }
        overlay.write(1001,Some(1001));assert!(overlay.positions.get().is_none());
        assert_eq!(overlay.len(),1001);
        overlay.write(0,None);assert!(overlay.positions.get().is_none());
    }
    #[test]
    fn remediation_selection_cache_reuses_only_immutable_owner() {
        use std::{cell::Cell,rc::Rc};
        #[derive(Clone)]struct Counted(Rc<Cell<usize>>,u64);
        impl Serialize for Counted {fn serialize<S:serde::Serializer>(&self,s:S)->Result<S::Ok,S::Error>{self.0.set(self.0.get()+1);s.serialize_u64(self.1)}}
        let count=Rc::new(Cell::new(0));
        let rows=kingmaker_core::row_store::RowTable::from_rows((0..1000u64).map(|i|(i,Counted(count.clone(),i)))).unwrap();
        let mut parent=Overlay::new(RowView::Indexed(&rows));parent.cacheable=true;
        {let child=Overlay::child(&parent);assert_eq!(child.iter().count(),1000);}
        let first=count.get();
        {
            let child=Overlay::child(&parent);assert_eq!(child.iter().count(),1000);
            child.guards.borrow().validate_owned(&parent,&parent.content).unwrap();
            assert_eq!(count.get(),first,"unchanged parent serialized on child apply");
        }
        {let child=Overlay::child(&parent);assert_eq!(child.iter().count(),1000);}
        assert_eq!(count.get(),first,"unchanged parent serialized again");
        parent.write(0,Some(Counted(count.clone(),9999)));
        {let child=Overlay::child(&parent);assert_eq!(child.iter().count(),1000);}
        assert!(count.get()>first,"changed content reused cached guard");
    }
    #[test]
    fn remediation_local_token_all_mutations() {
        let mut w=WorldState::default();w.pops.push(kingmaker_core::PopRecord{external_id:"p".into(),..Default::default()});
        let base=BorrowedWorld::new(&w).unwrap();
        for case in 0..14 {
            let mut tx=WorldTransaction::new(&base);let mark=tx.savepoint();let proof=tx.local_prepare_token();
            match case {
                0=>tx.put_pops("p".into(),w.pops[0].clone()).unwrap(),
                1=>tx.remove_pops("p".into()).unwrap(),
                2=>tx.replace_pops(w.pops.clone()).unwrap(),
                3=>tx.set_campaign(w.campaign.clone()).unwrap(),
                4=>{tx.edit_pop(&"p".into()).unwrap().size=9;},
                5=>tx.reorder_pops(vec!["p".into()]).unwrap(),
                6=>{let child=WorldTransaction::new(&tx);let delta=child.finish().unwrap();tx.apply_delta(delta).unwrap();},
                7=>tx.rollback_to(mark).unwrap(),
                8=>{tx.allocate_id().unwrap();},
                9=>tx.discard_forecast_accounts(),
                10=>{*tx.edit_pop_mobility()=None;},
                11=>{*tx.edit_needs()=None;},
                12=>tx.poison(),
                _=>{let session=crate::operation_session::OperationSession::new(&tx).unwrap();let changes=session.finish().unwrap();tx.apply_phase(changes).unwrap();}
            }
            assert!(tx.validate_local_prepare(&proof).is_err(),"accepted mutation {case}");
        }
        let a=WorldTransaction::new(&base);let b=WorldTransaction::new(&base);
        assert!(a.validate_local_prepare(&a.local_prepare_token()).is_ok());
        assert!(b.validate_local_prepare(&a.local_prepare_token()).is_err());
    }
}

/// Local proofs are private to a transaction owner; public detached preparations
/// always retain content guards, including mutations without a revision bump.
pub(crate) enum PrepareProof {
    Content(Vec<Fingerprint>),
    Local(crate::world_transaction::LocalPrepareToken),
}
impl PrepareProof {
    pub(crate) fn capture(w:&dyn kingmaker_core::world_read::WorldRead,local:Option<crate::world_transaction::LocalPrepareToken>)->OpResult<Self> {
        match local {Some(token)=>Ok(Self::Local(token)),None=>Ok(Self::Content(world_fingerprint(w)?))}
    }
    pub(crate) fn validate(&self,w:&crate::world_transaction::WorldTransaction<'_>)->OpResult<()> {
        match self {
            Self::Local(token)=>w.validate_local_prepare(token),
            Self::Content(expected)=>if *expected==world_fingerprint(w)? {Ok(())}else{Err(OperationError("Stale prepared content".into()))},
        }
    }
}

