use kingmaker_core::diagnostics::{self as d, Metric, Outcome, ProfileSession, Visibility::*};
use std::{cell::RefCell, io::{self, Write}, rc::Rc};
#[derive(Clone,Default)] struct Capture(Rc<RefCell<Vec<u8>>>);
impl Write for Capture {fn write(&mut self,b:&[u8])->io::Result<usize>{self.0.borrow_mut().extend_from_slice(b);Ok(b.len())}fn flush(&mut self)->io::Result<()>{Ok(())}}
fn events(c:&Capture)->Vec<serde_json::Value>{String::from_utf8(c.0.borrow().clone()).unwrap().lines().map(|s|serde_json::from_str(s).unwrap()).collect()}
#[test] fn nested_scopes_do_not_double_count(){
 let c=Capture::default(); let session=ProfileSession::install(Box::new(c.clone())).unwrap();
 let root=d::scope("root",Boundary); let child=d::scope("child",Detail);d::add(Metric::HashBytes,17);child.finish(Outcome::Ok);root.finish(Outcome::Ok);session.finish().unwrap();
 let rows=events(&c);let ends:Vec<_>=rows.iter().filter(|r|r["event"]=="end").collect();assert_eq!(ends.len(),2);
 assert_eq!(ends[0]["counters"]["hash_bytes"],17);assert_eq!(ends[1]["counters"]["hash_bytes"],0);
 assert_eq!(ends[1]["elapsed_ns"].as_u64().unwrap(),ends[1]["self_ns"].as_u64().unwrap()+ends[0]["elapsed_ns"].as_u64().unwrap());
 assert_eq!(rows.last().unwrap()["complete"],true);
}
#[test] fn disabled_and_abandoned_and_reinstall(){
 d::scope("disabled",Boundary).finish(Outcome::Ok);d::add(Metric::HashBytes,99);
 let c=Capture::default();let session=ProfileSession::install(Box::new(c.clone())).unwrap();assert!(ProfileSession::install(Box::new(io::sink())).is_err());
 drop(d::scope("early_return",Boundary));session.finish().unwrap();assert!(events(&c).iter().any(|r|r["outcome"]=="abandoned"));
 ProfileSession::install(Box::new(io::sink())).unwrap().finish().unwrap();
}
#[test] fn depth_overflow_is_sticky_without_panicking(){
 let session=ProfileSession::install(Box::new(io::sink())).unwrap();let mut spans=Vec::new();for _ in 0..65{spans.push(d::scope("deep",Detail));}
 assert!(!session.is_healthy());while let Some(s)=spans.pop(){s.finish(Outcome::Ok);}assert!(session.finish().is_err());
}
struct Failing(Rc<RefCell<bool>>);impl Write for Failing{fn write(&mut self,b:&[u8])->io::Result<usize>{if *self.0.borrow(){Err(io::Error::other("disk failed"))}else{Ok(b.len())}}fn flush(&mut self)->io::Result<()>{Ok(())}}
#[test] fn writer_error_does_not_replace_economic_result(){
 let fail=Rc::new(RefCell::new(false));let session=ProfileSession::install(Box::new(Failing(fail.clone()))).unwrap();*fail.borrow_mut()=true;
 let result:Result<u32,&str>={let s=d::scope("operation",Boundary);s.finish(Outcome::Ok);Ok(42)};
 assert_eq!(result,Ok(42));assert!(!session.is_healthy());assert!(session.finish().is_err());
 ProfileSession::install(Box::new(io::sink())).unwrap().finish().unwrap();
}
#[test] fn stale_spans_cannot_affect_a_new_session(){
 let session=ProfileSession::install(Box::new(io::sink())).unwrap();let old=d::scope("old",Boundary);assert!(session.finish().is_err());
 let c=Capture::default();let next=ProfileSession::install(Box::new(c.clone())).unwrap();drop(old);d::scope("new",Boundary).finish(Outcome::Ok);next.finish().unwrap();
 assert_eq!(events(&c).iter().filter(|r|r["event"]=="end").count(),1);
}
