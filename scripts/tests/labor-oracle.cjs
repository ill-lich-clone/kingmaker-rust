'use strict';
const fs=require('node:fs'),path=require('node:path');
const root=path.resolve(__dirname,'../../../Проект карты Тейванкаля/scripts/kingmaker');
const input=JSON.parse(fs.readFileSync(0,'utf8'));let result;
if(input.action==='institution_capacity') result=require(path.join(root,'institution-capacity.cjs')).deriveInstitutionJobs(input.registry).jobs;
else if(input.action==='roster') {const r=require(path.join(root,'employment-roster.cjs')).buildLaborRoster(input.snapshot);result={by_pop:[...r.byPop],by_job:[...r.byJob],by_pool:[...r.byPool]};}
else if(input.action==='release') result=require(path.join(root,'pop-mobility.cjs')).planEmploymentReleases(input.snapshot,{period:input.period});
else if(input.action==='tenure') result=require(path.join(root,'factories.cjs')).updateFactoryTenure(input.factory,input.options);
else if(input.action==='basic_basket') result=String(require(path.join(root,'state-services.cjs')).createBasicBasketPricer(input.snapshot.registry)(input.pop));
else if(input.action==='rgo_contract') result=require(path.join(root,'current-world-labor.cjs')).ownRgoContract({...input.options,weeklyBasketMinor:BigInt(input.options.weeklyBasketMinor)});
else if(input.action==='hiring') {input.snapshot.appliedEventIds=new Set();result=require(path.join(root,'labor-market.cjs')).planLocalHiring(input.snapshot,{period:'w',policies:input.snapshot.registry.policies||{}});}
else if(input.action==='labor_cash'){const r=require(path.join(root,'employment-roster.cjs')).buildLaborRoster(input.snapshot);result=[...require(path.join(root,'labor-market.cjs')).availableLaborCash(input.snapshot,input.snapshot.registry.policies||{},r)];}
else if(input.action==='forecast'){const f=require(path.join(root,'production-hiring.cjs')).createProductionHiringBudget(input.snapshot),entities=new Map([...input.snapshot.registry.resourceOperations||[],...input.snapshot.registry.factoryEnterprises||[],...input.snapshot.registry.artisanEnterprises||[]].map(e=>[e.id,e])),pops=new Map(input.snapshot.registry.pops.map(p=>[p.pop_id,p]));result=[];for(const step of input.steps){const e=entities.get(step.entity_id),p=pops.get(step.pop_id),n=f.limit(e,p,step.requested);result.push(n);if(step.accept&&n)f.accept(e,p,n);}}
else if(input.action==='artisan_hiring')result=require(path.join(root,'labor-market.cjs')).planArtisanHiring(input.snapshot,{period:'w'});
else if(input.action==='cohort_split'){input.snapshot.appliedEventIds=new Set(input.applied_event_ids||[]);result=require(path.join(root,'pop-cohorts.cjs')).planPopSplit(input.snapshot,input.options);}
else if(input.action==='history')result=require(path.join(root,'pop-cohorts.cjs')).updateUnemploymentHistory(input.snapshot,{period:input.period,payroll:input.payroll||[]});
else if(input.action==='profession'){input.snapshot.appliedEventIds=new Set();result=require(path.join(root,'pop-mobility.cjs')).planProfessionChanges(input.snapshot,{period:input.period,policies:input.snapshot.registry.policies||{}});}
else if(input.action==='migration_departure'){input.snapshot.appliedEventIds=new Set();result=require(path.join(root,'pop-migration.cjs')).planMigrationDepartures(input.snapshot,{period:input.period,policies:input.snapshot.registry.policies||{}});}
else if(input.action==='migration_arrival')result=require(path.join(root,'pop-migration.cjs')).planMigrationArrivals(input.snapshot,{period:input.period,date:input.date});
else if(input.action==='walking_path')result=require(path.join(root,'pop-migration.cjs')).walkingPath(input.registry,input.from,input.to,input.policies||{});
else throw Error('Unknown labor oracle action');
process.stdout.write(JSON.stringify(result,(_,v)=>typeof v==='undefined'?{'__kingmaker_codec_v2__':'undefined'}:typeof v==='bigint'?String(v):v));
