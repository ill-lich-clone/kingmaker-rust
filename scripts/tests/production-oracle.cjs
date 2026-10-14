'use strict';
// Small test fixtures only. Never reads the active SQLite campaign or advances a week.
const path=require('node:path');
const root=path.resolve(__dirname,'../../../Проект карты Тейванкаля/scripts/kingmaker');
const {applyOperations,planPriorObligations}=require(path.join(root,'ledger.cjs'));
const {planFeasibleActivities,planFoundationPayroll}=require(path.join(root,'activity-plans.cjs'));
const {planFoundationProduction}=require(path.join(root,'foundation-production.cjs'));
const {planStorageReservations}=require(path.join(root,'storage.cjs'));
const input=JSON.parse(process.argv[2]);
if(input.mode==='storage') {
 const fixture=require(path.resolve(__dirname,'../../../Проект карты Тейванкаля/scripts/tests/fixtures/kingmaker-storage-rounding.json'));
 const result=planStorageReservations(fixture.snapshot,fixture);
 console.log(JSON.stringify({rows:result.registry.storageReservations,blocked:result.blocked,partial:result.partial}));
} else {
 let state=input.snapshot;state.appliedEventIds=new Set(input.appliedEventIds||[]);
 const args={period:input.request.period,catalog:state.registry.productionCatalog,policies:input.request.policies,profile:input.request.profile,maintenanceReceipts:input.request.maintenance_receipts};
 const all=[];const prior=planPriorObligations(state,args.period);all.push(...prior);state=applyOperations(state,prior).state;
 const plan=planFeasibleActivities(state,args);state.registry={...state.registry,...plan.registry};all.push(...plan.storageOperations);state=applyOperations(state,plan.storageOperations).state;
 const payroll=planFoundationPayroll(state,{...args,plan});all.push(...payroll);state=applyOperations(state,payroll).state;
 const actual=planFoundationProduction(state,{...args,plan,paidLabor:payroll});all.push(...actual.operations);state=applyOperations(state,actual.operations).state;state.registry={...state.registry,...actual.registry};
 console.log(JSON.stringify({accounts:state.accounts,stocks:state.stocks,obligations:state.obligations,storage:state.registry.storageReservations,factories:state.registry.factoryEnterprises,activities:actual.activities,payroll,operations:all},(_,v)=>typeof v==='bigint'?String(v):v));
}
