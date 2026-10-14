//! Scoped cancellation for long synchronous phases; restored even while unwinding.
use crate::operation_session::{OpResult,OperationError};
use std::{cell::RefCell,collections::HashMap,path::PathBuf,sync::{Arc,atomic::{AtomicBool,Ordering}}};
#[derive(Clone)]
pub struct RunControl {
    probe:Arc<dyn Fn(&str)->OpResult<()>+Send+Sync>,
    cancelled:Arc<AtomicBool>,
}
struct Active {control:RunControl,counts:HashMap<&'static str,u16>}
thread_local! {static ACTIVE:RefCell<Option<Active>>=const {RefCell::new(None)};}
impl RunControl {
    pub fn new(probe:impl Fn(&str)->OpResult<()>+Send+Sync+'static)->Self {Self {probe:Arc::new(probe),cancelled:Arc::new(AtomicBool::new(false))}}
    pub fn from_stop_file(path:PathBuf)->Self {
        let last=std::sync::Mutex::new(None);
        Self::new(move |label| {
            let now=std::time::Instant::now();
            let mut last=last.lock().map_err(|_|OperationError("STOP polling lock poisoned".into()))?;
            if !poll_due(*last,now,matches!(label,"start"|"publish"|"checkpoint_publish")) {return Ok(());}
            *last=Some(now);
            if path.exists() {Err(OperationError("CANCELLED_BY_OPERATOR".into()))} else {Ok(())}
        })
    }
    pub fn is_cancelled(&self)->bool {self.cancelled.load(Ordering::Relaxed)}
    pub fn check(&self,label:&str)->OpResult<()> {
        if self.is_cancelled() {return Err(OperationError("CANCELLED_BY_OPERATOR".into()));}
        let result=(self.probe)(label);
        if result.is_err() {self.cancelled.store(true,Ordering::Relaxed);}
        result
    }
    pub fn run<R>(&self,run:impl FnOnce()->OpResult<R>)->OpResult<R> {
        struct Restore(Option<Active>);
        impl Drop for Restore {fn drop(&mut self) {ACTIVE.with(|slot|*slot.borrow_mut()=self.0.take());}}
        let _restore=Restore(ACTIVE.with(|slot|slot.replace(Some(Active{control:self.clone(),counts:HashMap::new()}))));
        self.check("start")?;
        run()
    }
}
pub(crate) fn check(label:&str)->OpResult<()> {
    let control=ACTIVE.with(|slot|slot.borrow().as_ref().map(|a|a.control.clone()));
    if let Some(control)=control {control.check(label)?;}
    Ok(())
}
pub(crate) fn work(label:&'static str)->OpResult<()> {
    let control=ACTIVE.with(|slot| {
        let mut slot=slot.borrow_mut();let active=slot.as_mut()?;
        let count=active.counts.entry(label).or_default();
        let due=*count==0;*count=(*count+1)%256;
        due.then(||active.control.clone())
    });
    if let Some(control)=control {control.check(label)?;}
    Ok(())
}

fn poll_due(last:Option<std::time::Instant>,now:std::time::Instant,force:bool)->bool {
    force || last.is_none_or(|last|now.duration_since(last)>=std::time::Duration::from_millis(100))
}
#[cfg(test)]
mod remediation_tests {
    use super::*;
    #[test]
    fn remediation_file_poll_budget() {
        let start=std::time::Instant::now();let mut last=None;let mut polls=0;
        for i in 0..100000 {let now=start+std::time::Duration::from_micros(i);if poll_due(last,now,false){polls+=1;last=Some(now);}}
        assert_eq!(polls,1);assert!(poll_due(last,start+std::time::Duration::from_millis(100),false));
        assert!(poll_due(last,start,true));
    }
    #[test]
    fn remediation_stop_file_detected_within_one_second() {
        let path=std::env::temp_dir().join(format!("kingmaker-remediation-stop-{}",std::process::id()));
        assert!(!path.exists());let control=RunControl::from_stop_file(path.clone());control.check("start").unwrap();
        std::fs::write(&path,b"stop").unwrap();let start=std::time::Instant::now();
        while control.check("cpu").is_ok() {assert!(start.elapsed()<std::time::Duration::from_secs(1));std::thread::yield_now();}
        let elapsed=start.elapsed();std::fs::remove_file(path).unwrap();assert!(elapsed<std::time::Duration::from_secs(1));
    }
}
