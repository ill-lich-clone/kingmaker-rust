//! Optional, thread-local diagnostics. Never part of economic state or evidence.
use serde::Serialize;
use serde_json::json;
use std::{cell::RefCell, io::{self, BufWriter, Write}, rc::Rc, time::Instant};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Visibility { Boundary, Detail }
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all="snake_case")]
pub enum Outcome { Ok, Error, Cancelled, Abandoned }
#[derive(Clone, Copy)]
#[repr(usize)]
pub enum Metric { HashCalls, HashBytes, IndexBuilds, IndexRows, SourceCacheHits, SourceCacheMisses, SourceSqlQueries, SourcePayloadBytes, HistoryCacheHits, HistoryCacheMisses, HistorySqlQueries, HistoryPayloadBytes, AuditStockChanges }
const METRIC_NAMES: [&str; 13] = ["hash_calls","hash_bytes","index_builds","index_rows","source_cache_hits","source_cache_misses","source_sql_queries","source_payload_bytes","history_cache_hits","history_cache_misses","history_sql_queries","history_payload_bytes","audit_stock_changes"];
const MAX_DEPTH: usize = 64;
struct Frame { id:u64, start:Instant, children_ns:u64, counters:[u64;13], visibility:Visibility }
impl Frame { fn self_ns(&self,elapsed:u64)->u64 {elapsed.saturating_sub(self.children_ns)} }
struct Recorder { writer:BufWriter<Box<dyn Write>>, start:Instant, next:u64, stack:Vec<Frame>, error:Option<String>, closed:bool }
impl Recorder {
 fn fail(&mut self,message:impl ToString){if self.error.is_none(){self.error=Some(message.to_string().chars().take(1024).collect());}}
 fn emit(&mut self,record:&impl Serialize,flush:bool){
  // Writer callbacks may themselves be instrumented. No TLS borrow is held;
  // their try_borrow_mut of this recorder fails harmlessly instead of recursing.
  let result=(||{serde_json::to_writer(&mut self.writer,record)?;self.writer.write_all(b"\n")?;if flush{self.writer.flush()?;}Ok::<_,io::Error>(())})();
  if let Err(e)=result {self.fail(e);}
 }
 fn close(&mut self){
  if self.closed{return;}
  if !self.stack.is_empty(){self.fail("profile finished with open scopes");}
  self.closed=true;
  self.emit(&json!({"event":"footer","complete":self.error.is_none(),"error":self.error,"allocation_bytes":null,"physical_disk_bytes":null,"unmeasured_reason":"global allocator and physical disk IO not instrumented"}),true);
 }
}
thread_local! { static ACTIVE:RefCell<Option<Rc<RefCell<Recorder>>>>=const{RefCell::new(None)}; }
fn current()->Option<Rc<RefCell<Recorder>>>{ACTIVE.with(|a|a.borrow().clone())}
fn detach(recorder:&Rc<RefCell<Recorder>>){ACTIVE.with(|a|{let mut a=a.borrow_mut();if a.as_ref().is_some_and(|r|Rc::ptr_eq(r,recorder)){*a=None;}});}

pub struct ProfileSession { recorder:Rc<RefCell<Recorder>> }
impl ProfileSession {
 pub fn install(writer:Box<dyn Write>)->io::Result<Self>{
  if current().is_some(){return Err(io::Error::other("profile already installed on this thread"));}
  let mut recorder=Recorder{writer:BufWriter::with_capacity(64*1024,writer),start:Instant::now(),next:0,stack:Vec::with_capacity(MAX_DEPTH),error:None,closed:false};
  recorder.emit(&json!({"event":"header","schema":1,"pid":std::process::id(),"workers":1}),true);
  if let Some(e)=recorder.error.take(){return Err(io::Error::other(e));}
  let recorder=Rc::new(RefCell::new(recorder));
  ACTIVE.with(|a|*a.borrow_mut()=Some(recorder.clone()));Ok(Self{recorder})
 }
 pub fn is_healthy(&self)->bool{self.recorder.try_borrow().is_ok_and(|r|r.error.is_none()&&!r.closed)}
 pub fn finish(self)->io::Result<()>{
  detach(&self.recorder);let mut r=self.recorder.borrow_mut();r.close();
  match &r.error{Some(e)=>Err(io::Error::other(e.clone())),None=>Ok(())}
 }
}
impl Drop for ProfileSession {fn drop(&mut self){detach(&self.recorder);if let Ok(mut r)=self.recorder.try_borrow_mut(){r.close();}}}

pub struct ProfileSpan { recorder:Option<Rc<RefCell<Recorder>>>, id:u64 }
pub fn scope(name:&'static str,visibility:Visibility)->ProfileSpan{
 let mut span=ProfileSpan{recorder:None,id:0};let Some(rc)=current() else{return span;};
 let Ok(mut r)=rc.try_borrow_mut() else{return span;};if r.closed{return span;}
 if r.stack.len()==MAX_DEPTH{r.fail("profile maximum depth exceeded");return span;}
 let Some(id)=r.next.checked_add(1) else{r.fail("profile scope ID overflow");return span;};r.next=id;
 let parent=r.stack.last().map(|f|f.id);let start=Instant::now();let relative_ns=start.duration_since(r.start).as_nanos().min(u64::MAX as u128) as u64;
 r.stack.push(Frame{id,start,children_ns:0,counters:[0;13],visibility});
 r.emit(&json!({"event":"begin","id":id,"parent_id":parent,"name":name,"relative_ns":relative_ns}),visibility==Visibility::Boundary);
 drop(r);span.recorder=Some(rc);span.id=id;span
}
pub fn add(metric:Metric,value:u64){
 let Some(rc)=current() else{return;};let Ok(mut r)=rc.try_borrow_mut() else{return;};if r.closed{return;}
 if let Some(f)=r.stack.last_mut(){let slot=&mut f.counters[metric as usize];if let Some(sum)=slot.checked_add(value){*slot=sum;}else{*slot=u64::MAX;r.fail("profile counter overflow");}}
}
impl ProfileSpan {
 pub fn finish(mut self,outcome:Outcome){self.end(outcome);}
 fn end(&mut self,outcome:Outcome){
  let Some(rc)=self.recorder.take() else{return;};let Ok(mut r)=rc.try_borrow_mut() else{return;};if r.closed{return;}
  if r.stack.last().map(|f|f.id)!=Some(self.id){r.fail("profile scopes closed out of order");return;}
  let f=r.stack.pop().expect("checked top");let elapsed=f.start.elapsed().as_nanos().min(u64::MAX as u128) as u64;
  if let Some(parent)=r.stack.last_mut(){if let Some(n)=parent.children_ns.checked_add(elapsed){parent.children_ns=n;}else{r.fail("profile time overflow");}}
  let counters:serde_json::Map<String,serde_json::Value>=METRIC_NAMES.iter().zip(f.counters).map(|(k,v)|((*k).to_owned(),v.into())).collect();
  r.emit(&json!({"event":"end","id":f.id,"outcome":outcome,"elapsed_ns":elapsed,"self_ns":f.self_ns(elapsed),"counters":counters}),f.visibility==Visibility::Boundary);
 }
}
impl Drop for ProfileSpan {fn drop(&mut self){self.end(Outcome::Abandoned);}}
/// Preserve the caller's result, including errors, while closing a measured area.
pub fn measure<T,E>(name:&'static str,visibility:Visibility,f:impl FnOnce()->Result<T,E>)->Result<T,E>{
 let span=scope(name,visibility);let result=f();span.finish(if result.is_ok(){Outcome::Ok}else{Outcome::Error});result
}
#[cfg(test)] mod tests {
 use super::*;
 #[test]fn synthetic_clock_arithmetic(){let f=Frame{id:1,start:Instant::now(),children_ns:30,counters:[0;13],visibility:Visibility::Detail};assert_eq!(f.self_ns(100),70);}
 #[test]fn completed_scopes_do_not_accumulate(){let session=ProfileSession::install(Box::new(io::sink())).unwrap();for _ in 0..100000{scope("repeated",Visibility::Detail).finish(Outcome::Ok);assert_eq!(session.recorder.borrow().stack.len(),0);}assert!(session.recorder.borrow().stack.capacity()<=MAX_DEPTH);session.finish().unwrap();}
}
