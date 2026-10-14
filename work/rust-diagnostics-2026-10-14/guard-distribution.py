import json,heapq
from pathlib import Path
from collections import defaultdict
root=Path(__file__).resolve().parent
active={};top=[];buckets=defaultdict(lambda:dict(calls=0,bytes=0,self_seconds=0));count=0
with (root/'run/profile.jsonl').open(encoding='utf-8') as f:
 for line in f:
  try:r=json.loads(line)
  except json.JSONDecodeError:break
  if r['event']=='begin':
   p=active.get(r['parent_id'],{})
   r['phase']=r['name'] if r['name'].startswith('phase.') else p.get('phase','outside_week')
   r['parent_name']=p.get('name');active[r['id']]=r
  elif r['event']=='end':
   b=active.pop(r['id'])
   if b['name']!='guard.serialize':continue
   count+=1;n=r['counters'].get('hash_bytes',0)
   bucket='<1KiB' if n<1024 else '<1MiB' if n<1048576 else '<100MiB' if n<104857600 else '>=100MiB'
   k=(b['phase'],bucket);g=buckets[k];g['calls']+=1;g['bytes']+=n;g['self_seconds']+=r['self_ns']/1e9
   item=(r['self_ns'],r['id'],dict(phase=b['phase'],parent=b['parent_name'],bytes=n,self_seconds=r['self_ns']/1e9,inclusive_seconds=r['elapsed_ns']/1e9,start_seconds=b['relative_ns']/1e9))
   heapq.heappush(top,item)
   if len(top)>15:heapq.heappop(top)
result=dict(completed_guard_calls=count,buckets=[dict(phase=k[0],bucket=k[1],**v) for k,v in buckets.items()],largest_self_calls=[v[2] for v in sorted(top,reverse=True)])
(root/'guard-distribution.json').write_text(json.dumps(result,indent=2),encoding='utf-8')
print(json.dumps(result))