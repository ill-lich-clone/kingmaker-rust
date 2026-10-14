#[path="common/native_week.rs"] mod common;
use kingmaker_core::{history::*,*};
use kingmaker_sim::native_week::execute_native_week;
use std::sync::Arc;
fn archive(w:&mut WorldState) {
    let mut history=MemoryHistory::default();
    for (i,row) in std::mem::take(&mut w.production_data.committed_operations).into_iter().enumerate() {
        let _=i;
        history.append(HistoryRecord{key:HistoryKey{namespace:"committed_operations".into(),id:row.event_id.clone()},value:serde_json::to_value(row).unwrap()}).unwrap();
    }
    for (i,row) in std::mem::take(&mut w.production_data.phase_receipts).into_iter().enumerate() {
        let _=i;
        history.append(HistoryRecord{key:HistoryKey{namespace:"production_phase_receipts".into(),id:ExternalId::from_json(serde_json::json!(i)).unwrap()},value:serde_json::to_value(row).unwrap()}).unwrap();
    }
    for (i,row) in std::mem::take(&mut w.market_data.phase_receipts).into_iter().enumerate() {
        let _=i;
        history.append(HistoryRecord{key:HistoryKey{namespace:"market_phase_receipts".into(),id:ExternalId::from_json(serde_json::json!(i)).unwrap()},value:serde_json::to_value(row).unwrap()}).unwrap();
    }
    for (i,row) in std::mem::take(&mut w.budget_data.phase_receipts).into_iter().enumerate() {
        let _=i;
        history.append(HistoryRecord{key:HistoryKey{namespace:"budget_phase_receipts".into(),id:ExternalId::from_json(serde_json::json!(i)).unwrap()},value:serde_json::to_value(row).unwrap()}).unwrap();
    }
    for (i,row) in std::mem::take(&mut w.budget_data.opening_receipts).into_iter().enumerate() {
        let _=i;
        history.append(HistoryRecord{key:HistoryKey{namespace:"budget_opening_receipts".into(),id:ExternalId::from_json(serde_json::json!(i)).unwrap()},value:serde_json::to_value(row).unwrap()}).unwrap();
    }
    for (i,row) in std::mem::take(&mut w.labor_data.phase_receipts).into_iter().enumerate() {
        let _=i;
        history.append(HistoryRecord{key:HistoryKey{namespace:"labor_phase_receipts".into(),id:ExternalId::from_json(serde_json::json!(i)).unwrap()},value:serde_json::to_value(row).unwrap()}).unwrap();
    }
    for (i,row) in std::mem::take(&mut w.transport_data.phase_receipts).into_iter().enumerate() {
        let _=i;
        history.append(HistoryRecord{key:HistoryKey{namespace:"transport_phase_receipts".into(),id:ExternalId::from_json(serde_json::json!(i)).unwrap()},value:serde_json::to_value(row).unwrap()}).unwrap();
    }
    for (i,row) in std::mem::take(&mut w.income_data.phase_receipts).into_iter().enumerate() {
        let _=i;
        history.append(HistoryRecord{key:HistoryKey{namespace:"income_phase_receipts".into(),id:ExternalId::from_json(serde_json::json!(i)).unwrap()},value:serde_json::to_value(row).unwrap()}).unwrap();
    }
    for (i,row) in std::mem::take(&mut w.construction_data.phase_receipts).into_iter().enumerate() {
        let _=i;
        history.append(HistoryRecord{key:HistoryKey{namespace:"construction_phase_receipts".into(),id:ExternalId::from_json(serde_json::json!(i)).unwrap()},value:serde_json::to_value(row).unwrap()}).unwrap();
    }
    for (i,row) in std::mem::take(&mut w.week_data.phase_receipts).into_iter().enumerate() {
        let _=i;
        history.append(HistoryRecord{key:HistoryKey{namespace:"week_phase_receipts".into(),id:ExternalId::from_json(serde_json::json!(i)).unwrap()},value:serde_json::to_value(row).unwrap()}).unwrap();
    }
    for (i,row) in std::mem::take(&mut w.mobility_data.history_receipts).into_iter().enumerate() {
        let _=i;
        history.append(HistoryRecord{key:HistoryKey{namespace:"mobility_history_receipts".into(),id:ExternalId::from_json(serde_json::json!(i)).unwrap()},value:serde_json::to_value(row).unwrap()}).unwrap();
    }
    for (i,row) in std::mem::take(&mut w.mobility_data.opening_receipts).into_iter().enumerate() {
        let _=i;
        history.append(HistoryRecord{key:HistoryKey{namespace:"mobility_opening_receipts".into(),id:ExternalId::from_json(serde_json::json!(i)).unwrap()},value:serde_json::to_value(row).unwrap()}).unwrap();
    }
    for (i,row) in std::mem::take(&mut w.mobility_data.migration_receipts).into_iter().enumerate() {
        let _=i;
        history.append(HistoryRecord{key:HistoryKey{namespace:"mobility_migration_receipts".into(),id:ExternalId::from_json(serde_json::json!(i)).unwrap()},value:serde_json::to_value(row).unwrap()}).unwrap();
    }
    for (i,row) in std::mem::take(&mut w.mobility_data.profession_receipts).into_iter().enumerate() {
        let _=i;
        history.append(HistoryRecord{key:HistoryKey{namespace:"mobility_profession_receipts".into(),id:ExternalId::from_json(serde_json::json!(i)).unwrap()},value:serde_json::to_value(row).unwrap()}).unwrap();
    }
    for (i,row) in std::mem::take(&mut w.mobility_data.cohort_receipts).into_iter().enumerate() {
        let _=i;
        history.append(HistoryRecord{key:HistoryKey{namespace:"mobility_cohort_receipts".into(),id:ExternalId::from_json(serde_json::json!(i)).unwrap()},value:serde_json::to_value(row).unwrap()}).unwrap();
    }
    w.history=Some(HistoryBacking::new(Arc::new(history)));
}
#[test]
fn archived_week_replay_and_next_week_preserve_actual_money_goods_and_evidence() {
    let mut stored=common::factory(false);
    let first=common::request(&stored);
    let receipt=execute_native_week(&mut stored,&first, |_|{}).unwrap();
    let mut continuous=stored.clone();
    archive(&mut stored);
    assert!(stored.production_data.committed_operations.is_empty());
    let before=serde_json::to_value(&stored).unwrap();
    assert_eq!(execute_native_week(&mut stored,&first, |_|{}).unwrap(),receipt);
    assert_eq!(serde_json::to_value(&stored).unwrap(),before);
    let second=common::request(&stored);
    let actual=execute_native_week(&mut stored,&second, |_|{}).unwrap();
    let expected=execute_native_week(&mut continuous,&second, |_|{}).unwrap();
    assert_eq!(stored.accounts,continuous.accounts);
    assert_eq!(stored.stockpiles,continuous.stockpiles);
    assert_eq!(stored.pops,continuous.pops);
    assert_eq!(stored.campaign,continuous.campaign);
    assert_eq!(actual.operation_evidence,expected.operation_evidence);
    assert_eq!(actual.registry_evidence,expected.registry_evidence);
}
