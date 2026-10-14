'use strict';
const fs=require('node:fs'),path=require('node:path');
const root=path.resolve(__dirname,'../../../Проект карты Тейванкаля/scripts/kingmaker');
const load=n=>require(path.join(root,n+'.cjs'));
const input=JSON.parse(fs.readFileSync(0,'utf8'));
const s=input.snapshot;s.appliedEventIds=new Set(s.appliedEventIds||[]);
const ledger=load('ledger');const operations=[],requirements=[];
let state=s;const merge=x=>{operations.push(...x.operations);state=ledger.applyOperations(state,x.operations).state;};
if(input.action==='opening') {
 if(state.registry.adventurerIncomeRates)merge(load('adventurer-income').planAdventurerIncome(state,input.options));
 const b=load('state-budget').planStateBudgetFunding(state,input.options);requirements.push(...b.result.requirements);merge(b);
 merge(load('foundation-budget').planBudgetFunding(state,input.options));
 merge({operations:ledger.planPriorObligations(state,input.options.period)});
 merge({operations:ledger.planPayroll(load('foundation-labor').serviceLaborSnapshot(state),input.options.period,input.options.policies)});
} else throw Error('Unknown action');
process.stdout.write(JSON.stringify({operations,requirements,accounts:state.accounts,obligations:state.obligations},(_,v)=>typeof v==='bigint'?String(v):v));
