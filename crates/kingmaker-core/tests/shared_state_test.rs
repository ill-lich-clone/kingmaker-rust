use kingmaker_core::{history::*, row_store::*, ExternalId};
use serde_json::json;
use std::sync::Arc;

#[test]
fn rows_share_payload_preserve_typed_identity_and_reject_duplicates() {
    let number = ExternalId::from(1);
    let string = ExternalId::from("1");
    let table = RowTable::from_rows([(number.clone(), json!({"extra":"x".repeat(65536)})), (string.clone(), json!(2))]).unwrap();
    let copy = table.clone();
    assert!(Arc::ptr_eq(table.shared(&number).unwrap(), copy.shared(&number).unwrap()));
    assert_ne!(table.get(&number), table.get(&string));
    let encoded = serde_json::to_string(&table).unwrap();
    let decoded: RowTable<ExternalId, serde_json::Value> = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded.get(&string), table.get(&string));
    assert_eq!(decoded.get(&number), table.get(&number));
    assert!(RowTable::from_rows([(number.clone(), 1), (number, 2)]).is_err());
    assert_eq!(copy.iter().map(|(key, _)| key.clone()).collect::<Vec<_>>(), [1.into(), "1".into()]);
}

#[test]
fn history_is_append_only_typed_and_receipts_resolve_shared_records() {
    let mut history = MemoryHistory::default();
    let key = HistoryKey { namespace: "operations".into(), id: 1.into() };
    let record = HistoryRecord { key: key.clone(), value: json!({"quantity":2}) };
    history.append(record.clone()).unwrap();
    history.append(record.clone()).unwrap();
    let a = history.get(&key).unwrap().unwrap();
    let b = history.get(&key).unwrap().unwrap();
    assert!(Arc::ptr_eq(&a, &b));
    assert!(history.append(HistoryRecord { key: key.clone(), value: json!(3) }).is_err());
    assert!(history.get(&HistoryKey { namespace:"operations".into(), id:"1".into() }).unwrap().is_none());
    assert_eq!(history.scan().count(), 1);
    assert_eq!(HistoryReferences(vec![key]).resolve(&history).unwrap()[0].as_ref(), &record);
}

#[test]
fn world_view_borrows_author_payload_and_indexes_current_typed_rows() {
    use kingmaker_core::{world_read::{BorrowedWorld, WorldRead}, *};
    let mut w = WorldState::default();
    w.stockpiles.push(StockpileRecord { external_id:"stock".into(), extra:serde_json::from_value(json!({"notes":"x".repeat(65536)})).unwrap(), ..Default::default() });
    w.accounts.push(AccountBalance { owner_id:"owner".into(), currency_id:"coin".into(), purpose:"savings".into(), balance:Money(7), ..Default::default() });
    w.market_data.institutions.push(json!({"notes":"y".repeat(65536)}));
    let view = BorrowedWorld::new(&w).unwrap();
    assert!(std::ptr::eq(view.stockpiles().get(&"stock".into()).unwrap(), &w.stockpiles[0]));
    assert!(std::ptr::eq(view.institutions().get(&0).unwrap(), &w.market_data.institutions[0]));
    assert_eq!(view.accounts().get(&w.accounts[0].address()).unwrap().balance, Money(7));
    assert!(std::ptr::eq(view.production_catalog(), &w.production_data.catalog));
}

#[test]
fn external_text_identity_is_shared_without_changing_json_type_or_contents() {
    let a:kingmaker_core::ExternalId=serde_json::from_str("\"long-shared-id\"").unwrap();
    let b=a.clone();
    let c:kingmaker_core::ExternalId=serde_json::from_str("\"long-shared-id\"").unwrap();
    if let (kingmaker_core::ExternalId::String(a),kingmaker_core::ExternalId::String(b),kingmaker_core::ExternalId::String(c))=(&a,&b,&c) {
        assert!(std::sync::Arc::ptr_eq(a,b));
        assert!(std::sync::Arc::ptr_eq(a,c));
    } else {panic!("String identity lost");}
    assert_eq!(serde_json::to_string(&a).unwrap(),"\"long-shared-id\"");
    assert_ne!(kingmaker_core::ExternalId::from("7"),kingmaker_core::ExternalId::from(7u32));
}

#[test]
fn author_extensions_share_payload_and_keep_copy_on_write_and_json_contract() {
    use kingmaker_core::production::Extra;
    let raw=serde_json::json!({"formula":"level * 2000","priorities":[],"quantity":7,"nullable":null,"large":{"source":"x".repeat(32768)}});
    let first:Extra=serde_json::from_value(raw.clone()).unwrap();
    let mut copy=first.clone();
    let separate:Extra=serde_json::from_value(raw.clone()).unwrap();
    assert!(std::ptr::eq(first.get("large").unwrap(),copy.get("large").unwrap()));
    assert!(std::ptr::eq(first.get("large").unwrap(),separate.get("large").unwrap()));
    copy.get_mut("large").unwrap()["source"]=serde_json::json!("edited");
    copy.insert("quantity".into(),serde_json::json!(9));
    assert_eq!(serde_json::to_value(&first).unwrap(),raw);
    assert_eq!(serde_json::to_value(&separate).unwrap(),raw);
    assert_eq!(copy["quantity"],9);
    assert!(copy.contains_key("nullable"));
    assert!(!copy.contains_key("missing"));
}
