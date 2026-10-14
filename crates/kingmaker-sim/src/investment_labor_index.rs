//! Ordered, per-person view used only inside one private staff preparation.
use std::collections::{BTreeMap, HashMap, HashSet};
use kingmaker_core::{ExternalId, budget::Employment, production::LaborAssignment, world_read::WorldRead, row_store::ReadSequence};
use crate::{operation_session::OpResult, budget_funding::error};
use serde_json::json;
type Source=(u8,usize);
type Position=(u8,usize,usize);
struct Rows<'a>(&'a BTreeMap<Position,LaborAssignment>);
impl ReadSequence<LaborAssignment> for Rows<'_> {
 fn len(&self)->usize {self.0.len()}
 fn get(&self,i:usize)->Option<&LaborAssignment>{self.0.values().nth(i)}
 fn iter(&self)->Box<dyn Iterator<Item=&LaborAssignment>+'_>{Box::new(self.0.values())}
}
pub(super) struct LaborIndex {
 sources:HashMap<Source,Vec<LaborAssignment>>,
 people:HashMap<ExternalId,BTreeMap<Position,LaborAssignment>>,
 typed:HashMap<ExternalId,usize>,
 fallback:HashMap<ExternalId,(usize,Employment)>,
 fallback_errors:HashMap<ExternalId,String>,
 dirty:HashSet<ExternalId>,
 invalid:HashMap<ExternalId,String>,
}
impl LaborIndex {
 pub fn new()->Self {Self{sources:HashMap::new(),people:HashMap::new(),typed:HashMap::new(),fallback:HashMap::new(),fallback_errors:HashMap::new(),dirty:HashSet::new(),invalid:HashMap::new()}}
 fn put(&mut self,k:Position,a:LaborAssignment) {self.dirty.insert(a.pop_id.clone());self.people.entry(a.pop_id.clone()).or_default().insert(k,a);}
 fn remove(&mut self,k:Position,a:&LaborAssignment) {self.dirty.insert(a.pop_id.clone());if let Some(rows)=self.people.get_mut(&a.pop_id){rows.remove(&k);}}
 fn refresh_fallback(&mut self,id:&ExternalId)->OpResult<()> {
  let Some((i,e))=self.fallback.get(id) else {return Ok(());};
  let (i,e)=(*i,e.clone());
  self.dirty.insert(e.pop_id.clone());
  if let Some(rows)=self.people.get_mut(&e.pop_id){rows.remove(&(4,i,0));}
  self.fallback_errors.remove(id);
  if self.typed.get(id).copied().unwrap_or(0)>0 || e.extra.get("direct_output_labor")==Some(&json!(true)){return Ok(());}
  let workers=e.extra.get("workers_count").and_then(|v|v.as_f64()).unwrap_or(e.fte.ceil());
  let time=e.extra.get("time_fraction").and_then(|v|v.as_f64()).unwrap_or(if workers>0. {e.fte/workers}else{0.});
  let row=serde_json::from_value(json!({"pop_id":e.pop_id,"employment_id":e.employment_id,"contract_id":e.contract_id,"workers_count":workers,"time_fraction":time,"fte":e.fte}));
  match row {Ok(row)=>self.put((4,i,0),row),Err(e)=>{self.fallback_errors.insert(id.clone(),e.to_string());}}
  Ok(())
 }
 pub fn set_source(&mut self,key:Source,rows:Vec<LaborAssignment>)->OpResult<Vec<LaborAssignment>> {
  let old=self.sources.remove(&key).unwrap_or_default();let mut ids=HashSet::new();
  for (i,a) in old.iter().enumerate(){crate::run_control::work("investment_labor_index")?;self.remove((key.0,key.1,i),a);if let Some(id)=&a.employment_id{*self.typed.get_mut(id).expect("indexed employment")-=1;ids.insert(id.clone());}}
  for (i,a) in rows.iter().enumerate(){crate::run_control::work("investment_labor_index")?;self.put((key.0,key.1,i),a.clone());if let Some(id)=&a.employment_id{*self.typed.entry(id.clone()).or_default()+=1;ids.insert(id.clone());}}
  self.sources.insert(key,rows);
  for id in ids{self.refresh_fallback(&id)?;}
  Ok(old)
 }
 pub fn set_employment(&mut self,i:usize,e:Employment)->OpResult<Option<Employment>> {
  let id=e.employment_id.clone();let old=self.fallback.insert(id.clone(),(i,e));
  if let Some((n,e))=&old{self.dirty.insert(e.pop_id.clone());if let Some(rows)=self.people.get_mut(&e.pop_id){rows.remove(&(4,*n,0));}}
  self.refresh_fallback(&id)?;Ok(old.map(|(_,e)|e))
 }
 pub fn check_fallback(&self)->OpResult<()> {
  if let Some(message)=self.fallback_errors.values().next(){return Err(error(message));} Ok(())
 }
 pub fn validate(&mut self,w:&dyn WorldRead)->OpResult<()> {
  for id in std::mem::take(&mut self.dirty) {
   crate::run_control::work("investment_labor_validate")?;
   self.invalid.remove(&id);
   let Some(rows)=self.people.get(&id).filter(|rows|!rows.is_empty()) else{continue;};
   let result=match w.pops().get(&id) {Some(p)=>crate::production_activities::validate_production_assignments(std::slice::from_ref(p),&Rows(rows)),None=>Err(error("Unknown assigned POP"))};
   if let Err(e)=result{self.invalid.insert(id,e.0);}
  }
  if let Some(message)=self.invalid.values().next(){return Err(error(message));}
  Ok(())
 }
}
