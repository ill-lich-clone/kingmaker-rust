use crate::construction_invariants::verify_construction;
use serde_json::json;
use kingmaker_core::{row_store::{RowRead,RowView}, world_read::{WorldRead,BorrowedWorld}, ExternalId, WorldState};
use std::cell::Cell;
struct CountProjects<'a> { rows:RowView<'a,ExternalId,kingmaker_core::production::ConstructionProjectLabor>, visits:Cell<usize> }
impl RowRead<ExternalId,kingmaker_core::production::ConstructionProjectLabor> for CountProjects<'_> {
 fn len(&self)->usize{self.rows.len()}
 fn at(&self,index:usize)->Option<&kingmaker_core::production::ConstructionProjectLabor>{self.rows.at(index)}
    fn get(&self,key:&ExternalId)->Option<&kingmaker_core::production::ConstructionProjectLabor>{ self.rows.get(key) }
    fn iter(&self)->Box<dyn Iterator<Item=(ExternalId,&kingmaker_core::production::ConstructionProjectLabor)>+'_>{Box::new(self.rows.keyed().inspect(|_|self.visits.set(self.visits.get()+1)))}
    fn values(&self)->Box<dyn Iterator<Item=&kingmaker_core::production::ConstructionProjectLabor>+'_>{Box::new(self.rows.values().inspect(|_|self.visits.set(self.visits.get()+1)))}
}
struct CountWorld<'a> { base:&'a dyn WorldRead, stocks:CountProjects<'a> }
macro_rules! counted_row {
    (construction_projects,$key:ty,$ty:ty)=>{fn construction_projects(&self)->RowView<'_,$key,$ty>{RowView::Indexed(&self.stocks)}};
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
fn remediation_construction_linear_projects() {
    for n in [1000,2000,4000] {
        let mut w=WorldState::default();
        w.production_data.construction_projects=(0..n).map(|i|serde_json::from_value(json!({"id":format!("p{i}"),"status":"waiting","owner_id":"owner","terms":{"rule":7}})).unwrap()).collect();
        let base=BorrowedWorld::new(&w).unwrap();
        let counted=CountWorld{base:&base,stocks:CountProjects{rows:base.construction_projects(),visits:Cell::new(0)}};
        assert!(verify_construction(&counted,&counted,&[]).unwrap().all_required_passed());
        assert!(counted.stocks.visits.get()<=4*n+1,"n={n},visits={}",counted.stocks.visits.get());
    }
}
#[test]
fn remediation_construction_preserves_owner_terms_and_project() {
    let mut w=WorldState::default();w.production_data.construction_projects=vec![serde_json::from_value(json!({"id":"p","status":"waiting","owner_id":"owner","terms":{"rule":7}})).unwrap()];
    for field in ["owner_id","terms"] {
        let mut after=w.clone();after.production_data.construction_projects[0].extra.insert(field.into(),json!("changed"));
        assert!(verify_construction(&w,&after,&[]).is_err());
    }
    let mut after=w.clone();after.production_data.construction_projects.clear();assert!(verify_construction(&w,&after,&[]).is_err());
}
