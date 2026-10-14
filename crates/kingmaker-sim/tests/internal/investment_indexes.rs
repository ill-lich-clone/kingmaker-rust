use kingmaker_core::{row_store::{RowRead,RowView}, world_read::{WorldRead,BorrowedWorld}, AccountAddress, AccountBalance, WorldState};
use std::cell::Cell;
struct CountAccounts<'a> { rows:RowView<'a,AccountAddress,AccountBalance>, visits:Cell<usize> }
impl RowRead<AccountAddress,AccountBalance> for CountAccounts<'_> {
 fn len(&self)->usize{self.rows.len()}
 fn at(&self,index:usize)->Option<&AccountBalance>{self.rows.at(index)}
    fn get(&self,key:&AccountAddress)->Option<&AccountBalance>{ self.rows.get(key) }
    fn iter(&self)->Box<dyn Iterator<Item=(AccountAddress,&AccountBalance)>+'_>{Box::new(self.rows.keyed().inspect(|_|self.visits.set(self.visits.get()+1)))}
    fn values(&self)->Box<dyn Iterator<Item=&AccountBalance>+'_>{Box::new(self.rows.values().inspect(|_|self.visits.set(self.visits.get()+1)))}
}
struct CountWorld<'a> { base:&'a dyn WorldRead, accounts:CountAccounts<'a> }
macro_rules! counted_row {
    (accounts,$key:ty,$ty:ty)=>{fn accounts(&self)->RowView<'_,$key,$ty>{RowView::Indexed(&self.accounts)}};
    ($name:ident,$key:ty,$ty:ty)=>{fn $name(&self)->RowView<'_,$key,$ty>{self.base.$name()}};
}
macro_rules! counted_world {
    (rows { $(($name:ident,$put:ident,$remove:ident,$replace:ident,$key:ty,$ty:ty,[$($field:ident),+],|$i:ident,$r:ident|$identity:expr),)* }
    singletons { $(($single:ident,$set:ident,$sty:ty,[$($sfield:ident),+]),)* })=>{
        impl WorldRead for CountWorld<'_>{
            fn source(&self)->&Option<std::sync::Arc<kingmaker_core::SourceSnapshot>>{self.base.source()}
            fn history(&self)->&Option<kingmaker_core::history::HistoryBacking>{self.base.history()}
            $(counted_row!($name,$key,$ty);)*
            $(fn $single(&self)->&$sty{self.base.$single()})*
        }
    }
}
kingmaker_core::world_fields!(counted_world);

#[test]
fn six_cash_uses_address_lookup() {
 let mut w=WorldState::default();
 for i in 0..1000 {w.accounts.push(kingmaker_core::AccountBalance{id:kingmaker_core::AccountId(i),external_id:format!("a{i}").into(),owner_id:format!("o{i}").into(),currency_id:"C".into(),purpose:"savings".into(),balance:kingmaker_core::Money(7)});}
 let base=BorrowedWorld::new(&w).unwrap();let counted=CountWorld{base:&base,accounts:CountAccounts{rows:base.accounts(),visits:Cell::new(0)}};
 for i in 0..1000 {assert_eq!(super::cash(&counted,&format!("o{i}").into(),&"C".into(),"savings").unwrap().0,7);}
 assert!(counted.accounts.visits.get()<=1000,"account visits={}",counted.accounts.visits.get());
}

struct History { rows:Vec<serde_json::Value>, visits:Cell<usize> }
impl kingmaker_core::row_store::ReadSequence<serde_json::Value> for History {
 fn len(&self)->usize{self.rows.len()} fn get(&self,i:usize)->Option<&serde_json::Value>{self.rows.get(i)}
 fn iter(&self)->Box<dyn Iterator<Item=&serde_json::Value>+'_>{Box::new(self.rows.iter().inspect(|_|self.visits.set(self.visits.get()+1)))}
}
#[test]
fn six_shares_do_not_rescan_unrelated_history() {
 let h=History{rows:(0..1000).map(|i|serde_json::json!({"project_id":format!("p{i}"),"investor_id":"a","amount_minor":"1"})).collect(),visits:Cell::new(0)};
 let mut index=super::ContributionIndex::new(&h).unwrap();
 for i in 0..1000 {let rows=index.shares(&format!("p{i}").into(),&num_bigint::BigInt::from(10)).unwrap();assert_eq!(rows[0]["numerator"],serde_json::json!("1"));}
 assert!(h.visits.get()<=1000,"history visits={}",h.visits.get());
}
