use kingmaker_core::{source_store::SourceRead,StoredRecord};
use kingmaker_io::source_store::SourceStoreBuilder;
#[test]
fn source_backing_preserves_raw_records_and_bounds_cache_without_retaining_scan_keys() {
    let mut writer=SourceStoreBuilder::temporary().unwrap();
    for n in 0..1000 {
        writer.push(&StoredRecord {table_name:"unported".into(),id_key:format!("[\"number\",{n}]"),external_id_json:Some(n.to_string()),ordinal:n,sequence:n,payload:format!("{{\"unknown\":\"{}\",\"id\":{n}}}","x".repeat(2048))}).unwrap();
    }
    let backing=writer.finish(8192).unwrap();
    let original=backing.record("unported","[\"number\",0]").unwrap().unwrap();
    for n in 0..1000 {backing.record("unported",&format!("[\"number\",{n}]")).unwrap();assert!(backing.cache_bytes()<=8192);}
    assert_eq!(*backing.record("unported","[\"number\",0]").unwrap().unwrap(),*original);
    assert_eq!(backing.scan("unported").map(|r|r.unwrap()).count(),1000);
    assert!(backing.record("unported","[\"string\",\"0\"]").unwrap().is_none());
    assert_eq!(backing.table_names().unwrap(),vec!["unported"]);
}

#[test]
fn raw_source_keeps_unsigned_metadata_and_long_identity_outside_small_cache() {
    let mut writer=SourceStoreBuilder::temporary().unwrap();
    let row=StoredRecord {table_name:"opaque".into(),id_key:"id".repeat(8192),external_id_json:None,ordinal:u64::MAX,sequence:u64::MAX,payload:" [] ".into()};
    writer.push(&row).unwrap();
    let backing=writer.finish(8192).unwrap();
    assert_eq!(*backing.record("opaque",&row.id_key).unwrap().unwrap(),row);
    assert_eq!(backing.cache_bytes(),0);
    assert_eq!(*backing.scan("opaque").next().unwrap().unwrap(),row);
}

#[test]
fn source_reference_dictionary_and_shared_values_survive_without_resident_maps() {
    use serde_json::json;
    use std::sync::Arc;
    let mut writer=SourceStoreBuilder::temporary().unwrap();
    writer.push_dictionary(7,&json!(7)).unwrap();
    writer.push_dictionary(8,&json!("7")).unwrap();
    writer.push_dictionary(u64::MAX,&json!("largest")).unwrap();
    writer.push_shared("norms",7,&json!({"number":{"__kingmaker_codec_v2__":"id","value":7},"string":{"__kingmaker_codec_v2__":"id","value":8},"formula":"level * 2000"})).unwrap();
    let source=kingmaker_core::SourceSnapshot{backing:Some(Arc::new(writer.finish(8192).unwrap())),..Default::default()};
    assert!(source.dictionary.is_empty() && source.shared.is_empty());
    assert_eq!(source.max_dictionary_id(),Some(u64::MAX));
    assert_eq!(source.dictionary_entries().map(|r|r.unwrap().0).collect::<Vec<_>>(),[7,8,u64::MAX]);
    let codec=kingmaker_io::codec::CodecContext::from_source(&source);
    let envelope=json!({"storage_schema_version":2,"table":"opaque","bigint_fields":[],"extensions":{},"value":{"__kingmaker_codec_v2__":"shared","table":"norms","index":7}});
    assert_eq!(codec.decode_record(&envelope).unwrap(),json!({"number":7,"string":"7","formula":"level * 2000"}));
    assert_eq!(*source.dictionary_value(8).unwrap().unwrap(),json!("7"));
    assert!(source.dictionary_value(9).unwrap().is_none());
}

#[test]
fn reference_scan_counts_rows_and_terminal_query_without_recounting_exhaustion() {
    use kingmaker_core::diagnostics::{self as profile, Outcome, Visibility, ProfileSession};
    use std::{io::Write, sync::{Arc,Mutex}};
    #[derive(Clone)] struct Capture(Arc<Mutex<Vec<u8>>>);
    impl Write for Capture {
        fn write(&mut self,b:&[u8])->std::io::Result<usize>{self.0.lock().unwrap().extend_from_slice(b);Ok(b.len())}
        fn flush(&mut self)->std::io::Result<()>{Ok(())}
    }
    let mut builder=SourceStoreBuilder::temporary().unwrap();
    builder.push_dictionary(1,&serde_json::json!("entry")).unwrap();
    builder.push_shared("rules",1,&serde_json::json!(42)).unwrap();
    let source=builder.finish(8192).unwrap();
    let bytes=Capture(Arc::new(Mutex::new(Vec::new())));
    let session=ProfileSession::install(Box::new(bytes.clone())).unwrap();
    let scope=profile::scope("references",Visibility::Boundary);
    let mut dictionary=source.dictionary_entries();
    assert!(dictionary.next().unwrap().is_ok());assert!(dictionary.next().is_none());assert!(dictionary.next().is_none());
    assert_eq!(source.shared_entries().count(),1);
    scope.finish(Outcome::Ok);session.finish().unwrap();
    let trace=String::from_utf8(bytes.0.lock().unwrap().clone()).unwrap();
    let end=trace.lines().map(|l|serde_json::from_str::<serde_json::Value>(l).unwrap()).find(|v|v["event"]=="end").unwrap();
    assert_eq!(end["counters"]["source_sql_queries"],4);
}

#[test]
fn remediation_source_batch_queries_and_order() {
    use kingmaker_core::diagnostics::{self as d,ProfileSession,Outcome,Visibility};
    use std::{cell::RefCell,rc::Rc,io::{self,Write}};
    #[derive(Clone)]struct Capture(Rc<RefCell<Vec<u8>>>);
    impl Write for Capture{fn write(&mut self,b:&[u8])->io::Result<usize>{self.0.borrow_mut().extend_from_slice(b);Ok(b.len())}fn flush(&mut self)->io::Result<()>{Ok(())}}
    for n in [0u64,1,1023,1024,1025,2048,4096] {
        let mut builder=SourceStoreBuilder::temporary().unwrap();
        for i in 0..n {
            builder.push(&StoredRecord{table_name:"rows".into(),id_key:i.to_string(),external_id_json:Some(i.to_string()),ordinal:i*2,sequence:i*3,payload:format!("{{\"authored\":{i}}}")}).unwrap();
            builder.push_dictionary(i*2,&serde_json::json!({"value":i})).unwrap();
        }
        let store=builder.finish(0).unwrap();let bytes=Rc::new(RefCell::new(Vec::new()));
        let profile=ProfileSession::install(Box::new(Capture(bytes.clone()))).unwrap();let span=d::scope("scan",Visibility::Boundary);
        let scan_clock=std::time::Instant::now();
        let rows:Vec<_>=store.scan("rows").map(Result::unwrap).collect();let refs:Vec<_>=store.dictionary_entries().map(Result::unwrap).collect();
        let scan_elapsed=scan_clock.elapsed().as_secs_f64();
        span.finish(Outcome::Ok);profile.finish().unwrap();
        assert_eq!(rows.len(),n as usize);assert_eq!(refs.len(),n as usize);
        for i in 0..n as usize {assert_eq!(rows[i].ordinal,i as u64*2);assert_eq!(rows[i].sequence,i as u64*3);assert_eq!(refs[i].0,i as u64*2);assert_eq!(refs[i].1["value"],i);}
        let trace=String::from_utf8(bytes.borrow().clone()).unwrap();let end:serde_json::Value=trace.lines().map(|l|serde_json::from_str::<serde_json::Value>(l).unwrap()).find(|v|v["event"]=="end").unwrap();
        let queries=end["counters"]["source_sql_queries"].as_u64().unwrap();
        assert!(queries<=2*(n.div_ceil(1024)+1),"n={n}, queries={queries}");
        println!("{}",serde_json::json!({"probe":"source_records_and_refs","rows_each":n,"sql_queries":queries,"single_sample_s":scan_elapsed}));
    }
}
