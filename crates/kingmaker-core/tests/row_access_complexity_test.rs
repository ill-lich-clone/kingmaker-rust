use kingmaker_core::row_store::{RowRead,RowView,ReadSequence,RowTable};
use std::cell::Cell;
struct CountRows { rows:RowTable<usize,usize>, visits:Cell<usize> }
impl RowRead<usize,usize> for CountRows {
 fn len(&self)->usize{self.rows.len()}
 fn at(&self,index:usize)->Option<&usize>{self.rows.at(index)}
 fn get(&self,key:&usize)->Option<&usize>{self.rows.get(key)}
 fn iter(&self)->Box<dyn Iterator<Item=(usize,&usize)>+'_>{Box::new(self.rows.iter().inspect(|_|self.visits.set(self.visits.get()+1)))}
 fn values(&self)->Box<dyn Iterator<Item=&usize>+'_>{Box::new(self.rows.values().inspect(|_|self.visits.set(self.visits.get()+1)))}
}
#[test]
fn remediation_indexed_access_linear() {
 for n in [1000,2000,4000] {
  let rows=CountRows{rows:RowTable::from_rows((0..n).map(|i|(i,i))).unwrap(),visits:Cell::new(0)};
  let view=RowView::Indexed(&rows);
  for i in 0..n { assert_eq!(view.at(i),Some(&i)); assert_eq!(view.len(),n); assert_eq!(ReadSequence::get(&view,i),Some(&i)); }
  assert_eq!(view.at(n),None);
  assert!(rows.visits.get()<=3*n+1,"n={n} visits={}",rows.visits.get());
 }
}
