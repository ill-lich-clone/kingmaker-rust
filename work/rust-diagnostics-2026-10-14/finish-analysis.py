import json,hashlib
from pathlib import Path
root=Path(__file__).resolve().parent
identity=json.loads((root/'identity.json').read_text(encoding='utf-8'))
def digest(path):
 h=hashlib.sha256()
 with Path(path).open('rb') as f:
  for block in iter(lambda:f.read(8*1024*1024),b''):h.update(block)
 return h.hexdigest()
original=Path(identity['source'])
verification=dict(snapshot_sha256_matches=digest(identity['snapshot'])==identity['snapshot_sha256'],executable_sha256_matches=digest(identity['executable'])==identity['executable_sha256'],runtime_sha256_matches=digest(identity['runtime'])==identity['runtime_sha256'],original_source_size_unchanged=original.stat().st_size==identity['source_size'],original_source_mtime_unchanged=original.stat().st_mtime_ns==identity['source_mtime_ns'],original_source_content_hash_checked=False)
(root/'input-verification.json').write_text(json.dumps(verification,indent=2),encoding='utf-8')
summary=json.loads((root/'profile-summary.json').read_text(encoding='utf-8'))
attribution=json.loads((root/'phase-attribution.json').read_text(encoding='utf-8'))
samples=[json.loads(line) for line in (root/'process-samples.jsonl').read_text(encoding='utf-8').splitlines()]
assert sum(g['self_ns'] for g in attribution['groups'])==summary['total_self_ns']
assert sum(g['hash_calls'] for g in attribution['groups'])==summary['counters']['hash_calls']
assert sum(g['hash_bytes'] for g in attribution['groups'])==summary['counters']['hash_bytes']
guards=[g for g in attribution['groups'] if g['name']=='guard.serialize']
phase_samples=[]
for boundary in attribution['completed_boundaries']:
 if not boundary['name'].startswith('phase.'):continue
 begin=boundary['start_seconds'];end=begin+boundary['seconds']
 within=[s for s in samples if begin<=s['elapsed_seconds']<=end]
 phase_samples.append(dict(name=boundary['name'],seconds=boundary['seconds'],sampled_peak_private_bytes=max((s['private_bytes'] for s in within),default=None),sampled_cpu_seconds=within[-1]['cpu_seconds']-within[0]['cpu_seconds'] if len(within)>1 else None))
active=[b for b in attribution['active'] if b['name'].startswith('phase.')]
for b in active:
 within=[s for s in samples if s['elapsed_seconds']>=b['start_seconds']]
 phase_samples.append(dict(name=b['name'],complete=False,observed_seconds=samples[-1]['elapsed_seconds']-b['start_seconds'],first_sample_private_bytes=within[0]['private_bytes'] if within else None,last_sample_private_bytes=within[-1]['private_bytes'] if within else None,sampled_peak_private_bytes=max((s['private_bytes'] for s in within),default=None)))
result=dict(source_and_input_verification=verification,aggregation_matches_existing_summary=True,completed_scope_self_seconds=summary['total_self_ns']/1e9,guard_self_seconds=sum(g['self_ns'] for g in guards)/1e9,guard_by_phase=[dict(phase=g['phase'],self_seconds=g['self_ns']/1e9,calls=g['calls'],hash_bytes=g['hash_bytes']) for g in guards],last_process_sample=samples[-1],phase_samples=phase_samples,cpu_memory_phase_alignment='Approximate: process start vs instrumentation start and 1s sample cadence. CPU interval excludes edge samples.')
(root/'diagnostic-findings.json').write_text(json.dumps(result,indent=2),encoding='utf-8')
print(json.dumps(result))