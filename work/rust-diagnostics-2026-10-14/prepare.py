import hashlib,json,sqlite3,time,platform
from pathlib import Path
root=Path.cwd()
out=root/'work/rust-diagnostics-2026-10-14'
runtime=root.parent/'Проект карты Тейванкаля/data/kingmaker/runtime.json'
config=json.loads(runtime.read_text(encoding='utf-8-sig'))
source=Path(config['dbPath']); snapshot=out/'input.sqlite'
def digest(path):
    h=hashlib.sha256()
    with path.open('rb') as f:
        for chunk in iter(lambda:f.read(8*1024*1024),b''): h.update(chunk)
    return h.hexdigest()
start=time.perf_counter()
with sqlite3.connect(source.as_uri()+'?mode=ro',uri=True) as src:
    with sqlite3.connect(snapshot) as dst: src.backup(dst,pages=8192)
config['dbPath']=str(snapshot)
(out/'runtime.json').write_text(json.dumps(config,ensure_ascii=False,indent=2),encoding='utf-8')
exe=Path('C:/Users/ill_lich/.cargo-target/kingmaker-rust/release/kingmaker-cli.exe')
identity=dict(runtime=str(runtime),runtime_sha256=digest(runtime),source=str(source),source_size=source.stat().st_size,source_mtime_ns=source.stat().st_mtime_ns,snapshot=str(snapshot),snapshot_sha256=digest(snapshot),snapshot_size=snapshot.stat().st_size,executable=str(exe),executable_sha256=digest(exe),preparation_seconds=time.perf_counter()-start,platform=platform.platform(),compilation_included=False,filesystem_cache='not flushed; snapshot creation and hashing warm the input')
(out/'identity.json').write_text(json.dumps(identity,ensure_ascii=False,indent=2),encoding='utf-8')
args=['--runtime',str(out/'runtime.json'),'--run-weeks','1','--output',str(out/'run'),'--profile']
(out/'arguments.json').write_text(json.dumps(args,ensure_ascii=False),encoding='utf-8')
print(json.dumps({'snapshot_bytes':snapshot.stat().st_size,'preparation_seconds':identity['preparation_seconds']}))