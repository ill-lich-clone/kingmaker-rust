'use strict';
// One supplied miniature only; no SQLite, runtime changes or measured-world weeks.
const fs=require('node:fs'),path=require('node:path');
const root=path.resolve(__dirname,'../../../Проект карты Тейванкаля/scripts/kingmaker');
const input=JSON.parse(fs.readFileSync(0,'utf8'));
const s=input.snapshot;s.appliedEventIds=new Set(s.appliedEventIds||[]);
s.registry.foundationMigration={version:'explicit miniature enables current automatic output offers'};
s.registry.foundationSignature=require(path.join(root,'engine-version.cjs')).foundationSignature(s);
const out=require(path.join(root,'foundation-turn.cjs')).planFoundationPhases(s,{period:input.period,decisions:input.policies||{},profile:input.profile||{}});
process.stdout.write(JSON.stringify({accounts:out.state.accounts,stocks:out.state.stocks,offers:out.state.registry.saleOffers,priorities:out.state.registry.policies?.priorities,operations:out.operations,orders:out.orders,activities:out.activities,producedByKind:out.producedByKind},(_,v)=>typeof v==='bigint'?String(v):v));
