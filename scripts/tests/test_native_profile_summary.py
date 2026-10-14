import importlib.util,json,tempfile,unittest
from pathlib import Path
spec=importlib.util.spec_from_file_location('summary',Path(__file__).resolve().parents[1]/'summarize-native-profile.py')
m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
class SummaryTests(unittest.TestCase):
 def run_trace(self,rows,process=None,tail=''):
  with tempfile.TemporaryDirectory() as d:
   p=Path(d);(p/'profile').write_text(''.join(json.dumps(r)+'\n' for r in rows)+tail);(p/'process').write_text(json.dumps(process or {'outcome':'completed','exit_code':0,'pid':111,'peak_private_bytes':100}))
   return m.summarize(p/'profile',p/'process')
 def base(self):return [{'event':'header','schema':1,'pid':111},{'event':'begin','id':1,'parent_id':None,'name':'root','relative_ns':0},{'event':'begin','id':2,'parent_id':1,'name':'child','relative_ns':10},{'event':'end','id':2,'outcome':'ok','elapsed_ns':30,'self_ns':30,'counters':{'hash_bytes':17}},{'event':'end','id':1,'outcome':'ok','elapsed_ns':100,'self_ns':70,'counters':{'hash_bytes':0}},{'event':'footer','complete':True}]
 def test_nested(self):
  s=self.run_trace(self.base());self.assertTrue(s['complete']);self.assertEqual(s['total_self_ns'],100);self.assertEqual(s['counters']['hash_bytes'],17)
 def test_thirteen_remediation_counters(self):
  rows=self.base();rows[3]['counters']={name:1 for name in ('audit_stock_changes','hash_bytes','hash_calls','history_cache_hits','history_cache_misses','history_payload_bytes','history_sql_queries','index_builds','index_rows','source_cache_hits','source_cache_misses','source_payload_bytes','source_sql_queries')}
  s=self.run_trace(rows);self.assertTrue(s['complete']);self.assertEqual(len(s['counters']),13)
 def test_counter_bound_remains_enforced(self):
  rows=self.base();rows[3]['counters']={str(i):1 for i in range(14)}
  s=self.run_trace(rows);self.assertTrue(s['corrupted']);self.assertIn('too many counters',s['errors'])
 def test_partial_tail_and_memory_limit(self):
  s=self.run_trace(self.base()[:3],{'outcome':'memory_limit','exit_code':1},'{"event":');self.assertFalse(s['complete']);self.assertEqual([x['name'] for x in s['last_open_scopes']],['root','child']);self.assertTrue(s['truncated_tail'])
 def test_failure_footer_and_nonzero_exit(self):
  for status in [{'outcome':'timeout','exit_code':1},{'outcome':'completed','exit_code':3}]:self.assertFalse(self.run_trace(self.base(),status)['complete'])
 def test_corrupt_middle(self):
  s=self.run_trace([],tail='{"event":"header","schema":1}\nBROKEN\n{"event":"footer","complete":true}\n');self.assertTrue(s['corrupted']);self.assertFalse(s['complete'])
 def test_process_pid_must_match(self):
  s=self.run_trace(self.base(),{'outcome':'completed','exit_code':0,'pid':222,'peak_private_bytes':999})
  self.assertFalse(s['complete']);self.assertEqual(s['process_association'],'mismatch');self.assertIsNone(s['process'])
 def test_missing_pid_is_unverified(self):
  s=self.run_trace(self.base(),{'outcome':'completed','exit_code':0})
  self.assertFalse(s['complete']);self.assertEqual(s['process_association'],'unverified');self.assertIsNone(s['process'])
if __name__=='__main__':unittest.main()
