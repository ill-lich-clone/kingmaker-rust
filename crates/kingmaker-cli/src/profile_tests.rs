
#[path = "../../kingmaker-sim/tests/common/native_week.rs"] mod common;
use super::*;
use kingmaker_core::*;
use std::{cell::Cell, rc::Rc};
fn world() -> WorldState {
    let mut w = common::factory(false);
    add_raw(&mut w);
    attach_source(&mut w);
    w
}
fn add_raw(w: &mut WorldState) {
    for p in &mut w.provinces {
        p.goods_id = Some(Goods::Grain);
    }
    w.production_data.catalog.as_mut().unwrap().goods.push(
        serde_json::from_value(json!({"goods_id":"grain","kind":"raw_resource","unit":"t"}))
            .unwrap(),
    );
    w.production_data.catalog.as_mut().unwrap().methods.push(serde_json::from_value(json!({"method_id":"rgo:grain","kind":"rgo","status":"ready","output_goods_id":"grain","output_unit":"t","weekly_productivity":1,"staffing":["farmers"],"inputs":[],"capacity":{"base_people":100,"source":"explicit mini"}})).unwrap());
    let owner = w
        .production_data
        .factories
        .first()
        .and_then(|f| f.base.owner_id.clone())
        .or_else(|| {
            w.accounts
                .iter()
                .find(|a| a.purpose == "employer_operating_capital")
                .map(|a| a.owner_id.clone())
        })
        .unwrap();
    for p in &w.provinces {
        w.production_data.resource_operations.push(serde_json::from_value(json!({"id":format!("raw:{}",p.id.0),"owner_id":owner,"province_id":p.id.0,"method_id":"rgo:grain","resource_id":"grain","resource_status":"source","land_access":true,"assignments":[]})).unwrap());
    }
}
fn attach_source(w: &mut WorldState) {
    let mut source = SourceSnapshot::default();
    source.database_path = "absent-original.sqlite".into();
    source.tables.insert(
        "unknown-author-table".into(),
        vec![StoredRecord {
            table_name: "unknown-author-table".into(),
            id_key: "[\"number\",7]".into(),
            external_id_json: Some("7".into()),
            ordinal: 9,
            sequence: 11,
            payload: "{\"author_rule\":\"never replace\"}".into(),
        }],
    );
    w.source = Some(Arc::new(source));
}

#[test]
fn failed_profile_before_week_preserves_zero_completed_weeks() {
    struct Failing(Rc<Cell<bool>>);
    impl Write for Failing {
        fn write(&mut self,b:&[u8])->std::io::Result<usize>{if self.0.get(){Err(std::io::Error::other("injected profile failure"))}else{Ok(b.len())}}
        fn flush(&mut self)->std::io::Result<()>{Ok(())}
    }
    let stamp=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir=std::env::temp_dir().join(format!("kingmaker-profile-failure-{}-{stamp}",std::process::id()));
    fs::create_dir(&dir).unwrap();
    let world=world();
    assert!(audit_initial_world(&world).can_run());
    let identity=CheckpointIdentity{native_fingerprint:native_fingerprint().unwrap(),engine_hash:world.campaign.engine_hash.clone(),rules_hash:world.campaign.rules_hash.clone(),source_hash:world.campaign.source_hash.clone()};
    let start=dir.join("start");save_checkpoint(&world,&start,&identity).unwrap();
    let out=dir.join("out");fs::create_dir(&out).unwrap();
    let failed=Rc::new(Cell::new(false));
    let session=ProfileSession::install(Box::new(Failing(failed.clone()))).unwrap();failed.set(true);
    let result=run_inner(Path::new("unused"),Some(&start),&out,1,None,false,Some(&session));
    assert!(result.is_err());assert!(session.finish().is_err());
    let run:Value=serde_json::from_slice(&fs::read(out.join("run.json")).unwrap()).unwrap();
    let (saved,_)=load_checkpoint(&out.join("final")).unwrap();
    fs::remove_dir_all(&dir).unwrap();
    assert_eq!(run["completed_weeks"],0);
    assert_eq!(saved.campaign.revision,world.campaign.revision);
}
