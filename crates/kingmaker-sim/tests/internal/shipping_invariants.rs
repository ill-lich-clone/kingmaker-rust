#[path = "../common/shipping.rs"]
mod common;
use crate::shipping_invariants::verify_arrivals;
use kingmaker_core::operations::*;
use serde_json::json;

#[test]
fn shipping_debt_changes_require_exact_operation_evidence() {
    let before = common::world(3.);
    let debt:Obligation=serde_json::from_value(json!({"obligation_id":"rent-refund","payer_id":"landlord","payee_id":"buyer","currency_id":"COIN","purpose":"operating_capital","amount_minor":"1","category":"lease_refund"})).unwrap();
    let mut after = before.clone();
    after.production_data.obligations.push(debt.clone());
    assert!(
        verify_arrivals(&before, &after, &[]).is_err(),
        "unexplained debt accepted"
    );
    let op: EconomicOperation = serde_json::from_value(
        json!({"event_id":"refund","type":"storage_rent_refund","obligation":debt}),
    )
    .unwrap();
    assert!(verify_arrivals(&before, &after, &[op.clone()])
        .unwrap()
        .all_required_passed());
    after.production_data.obligations[0].payee_id = "thief".into();
    assert!(
        verify_arrivals(&before, &after, &[op]).is_err(),
        "changed debt title accepted"
    );
}

use kingmaker_core::{row_store::{RowRead,RowView}, world_read::{WorldRead,BorrowedWorld}, ExternalId, StockpileRecord, WorldState};
use std::cell::Cell;
struct CountStocks<'a> { rows:RowView<'a,ExternalId,StockpileRecord>, visits:Cell<usize> }
impl RowRead<ExternalId,StockpileRecord> for CountStocks<'_> {
 fn len(&self)->usize{self.rows.len()}
 fn at(&self,index:usize)->Option<&StockpileRecord>{self.rows.at(index)}
    fn get(&self,key:&ExternalId)->Option<&StockpileRecord>{ self.rows.get(key) }
    fn iter(&self)->Box<dyn Iterator<Item=(ExternalId,&StockpileRecord)>+'_>{Box::new(self.rows.keyed().inspect(|_|self.visits.set(self.visits.get()+1)))}
    fn values(&self)->Box<dyn Iterator<Item=&StockpileRecord>+'_>{Box::new(self.rows.values().inspect(|_|self.visits.set(self.visits.get()+1)))}
}
struct CountWorld<'a> { base:&'a dyn WorldRead, stocks:CountStocks<'a> }
macro_rules! counted_row {
    (stockpiles,$key:ty,$ty:ty)=>{fn stockpiles(&self)->RowView<'_,$key,$ty>{RowView::Indexed(&self.stocks)}};
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
fn remediation_arrivals_linear_work() {
    for n in [1000,2000,4000] {
        let mut w=WorldState::default();
        w.stockpiles=(0..n).map(|i|StockpileRecord{external_id:format!("s{i}").into(),quantity:1.,..Default::default()}).collect();
        let indexed=BorrowedWorld::new(&w).unwrap();
        let counted=CountWorld{base:&indexed,stocks:CountStocks{rows:indexed.stockpiles(),visits:Cell::new(0)}};
        assert!(verify_arrivals(&counted,&counted,&[]).unwrap().all_required_passed());
        assert!(counted.stocks.visits.get()<=8*(n+1),"n={n}, visits={}",counted.stocks.visits.get());
    }
}
#[test]
fn remediation_arrivals_preserves_old_stock_fields_and_identity() {
    let mut before=WorldState::default();
    before.stockpiles=vec![StockpileRecord{external_id:ExternalId::from_json(json!(7)).unwrap(),quantity:3.,..Default::default()},StockpileRecord{external_id:"7".into(),quantity:5.,..Default::default()}];
    assert!(verify_arrivals(&before,&before,&[]).unwrap().all_required_passed());
    for field in ["owner_id","unit","goods","external_id","quantity"] {
        let mut after=before.clone();
        let mut changed=serde_json::to_value(&after.stockpiles[0]).unwrap();
        match field {
            "quantity"=>{changed[field]=json!(4.);after.stockpiles[1].quantity=4.;},
            "goods"=>changed[field]=json!("iron"),
            "external_id"=>changed[field]=json!("replacement"),
            _=>changed[field]=json!("changed"),
        }
        after.stockpiles[0]=serde_json::from_value(changed).unwrap();
        assert!(verify_arrivals(&before,&after,&[]).is_err(),"accepted changed {field}");
    }
    let mut after=before.clone();after.stockpiles.remove(0);after.stockpiles[0].quantity=8.;
    assert!(verify_arrivals(&before,&after,&[]).is_err());
}
#[test]
fn remediation_arrivals_repeated_changes_keep_sum_order() {
    let mut before=WorldState::default();
    before.stockpiles=vec![StockpileRecord{external_id:"a".into(),quantity:10.,..Default::default()},StockpileRecord{external_id:"b".into(),quantity:10.,..Default::default()}];
    let mut after=before.clone();after.stockpiles[0].quantity=11.;after.stockpiles[1].quantity=9.;
    let ops:Vec<EconomicOperation>=[json!({"event_id":"1","type":"shipment_arrival","stock_changes":[{"stock_id":"a","delta":1e16},{"stock_id":"b","delta":-1.}]}),json!({"event_id":"2","type":"shipment_arrival","stock_changes":[{"stock_id":"a","delta":-1e16},{"stock_id":"a","delta":1.}]})].into_iter().map(|v|serde_json::from_value(v).unwrap()).collect();
    assert!(verify_arrivals(&before,&after,&ops).unwrap().all_required_passed());
}



#[test]
fn remediation_arrivals_changes_linear_and_duplicates_rejected() {
    use kingmaker_core::diagnostics::{self as d,ProfileSession};
    use std::{io::Write,rc::Rc,cell::RefCell};
    #[derive(Clone)]struct Capture(Rc<RefCell<Vec<u8>>>);
    impl Write for Capture {fn write(&mut self,b:&[u8])->std::io::Result<usize>{self.0.borrow_mut().extend_from_slice(b);Ok(b.len())}fn flush(&mut self)->std::io::Result<()>{Ok(())}}
    for n in [1000,2000,4000] {
        let mut before=WorldState::default();before.stockpiles=(0..n).map(|i|StockpileRecord{external_id:format!("s{i}").into(),quantity:10.,..Default::default()}).collect();
        let ops:Vec<EconomicOperation>=(0..n).map(|i|serde_json::from_value(json!({"event_id":format!("e{i}"),"type":"shipment_arrival","stock_changes":[{"stock_id":format!("s{i}"),"delta":1.},{"stock_id":format!("s{i}"),"delta":-1.}]})).unwrap()).collect();
        let bytes=Capture(Rc::new(RefCell::new(Vec::new())));let session=ProfileSession::install(Box::new(bytes.clone())).unwrap();
        assert!(verify_arrivals(&before,&before,&ops).unwrap().all_required_passed());session.finish().unwrap();
        let trace=String::from_utf8(bytes.0.borrow().clone()).unwrap();
        let visits:u64=trace.lines().map(|l|serde_json::from_str::<serde_json::Value>(l).unwrap()).filter(|v|v["event"]=="end").map(|v|v["counters"]["audit_stock_changes"].as_u64().unwrap_or(0)).sum();
        assert_eq!(visits,2*n as u64);
        let mut after=before.clone();after.stockpiles.push(before.stockpiles[0].clone());assert!(verify_arrivals(&before,&after,&[]).is_err());
    }
}
