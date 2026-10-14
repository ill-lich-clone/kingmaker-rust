import json,sys
from pathlib import Path
from collections import defaultdict
root=Path(__file__).resolve().parent
active={};groups=defaultdict(lambda:dict(calls=0,self_ns=0,inclusive_ns=0,hash_calls=0,hash_bytes=0));completed=[];latest=0
with (root/'run/profile.jsonl').open(encoding='utf-8') as f:
 for line in f:
  try:r=json.loads(line)
  except json.JSONDecodeError:break
  if r['event']=='begin':
   parent=active.get(r['parent_id'],{})
   r['phase']=r['name'] if r['name'].startswith('phase.') else parent.get('phase')
   active[r['id']]=r;latest=max(latest,r['relative_ns'])
  elif r['event']=='end':
   b=active.pop(r['id']);latest=max(latest,b['relative_ns']+r['elapsed_ns'])
   g=groups[(b.get('phase') or 'outside_week',b['name'])]
   g['calls']+=1;g['self_ns']+=r['self_ns'];g['inclusive_ns']+=r['elapsed_ns']
   g['hash_calls']+=r['counters'].get('hash_calls',0);g['hash_bytes']+=r['counters'].get('hash_bytes',0)
   if b['name'].startswith(('phase.','run.','storage.capacity.')):
    completed.append(dict(name=b['name'],seconds=r['elapsed_ns']/1e9,self_seconds=r['self_ns']/1e9,outcome=r['outcome'],start_seconds=b['relative_ns']/1e9))
result=dict(latest_trace_seconds=latest/1e9,active=[dict(name=b['name'],start_seconds=b['relative_ns']/1e9) for b in active.values()],completed_boundaries=completed,groups=[dict(phase=p,name=n,**v) for (p,n),v in sorted(groups.items(),key=lambda x:x[1]['self_ns'],reverse=True)])
if '--save' in sys.argv:(root/'phase-attribution.json').write_text(json.dumps(result,ensure_ascii=False,indent=2),encoding='utf-8')
print(json.dumps(dict(latest_trace_seconds=result['latest_trace_seconds'],active=result['active'],completed_boundaries=completed,top_groups=result['groups'][:8]),ensure_ascii=False))