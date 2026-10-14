'use strict';
// Tiny source-contract fixtures only. No active DB, full world or game-week execution.
const fs=require('node:fs'),path=require('node:path');
const source=path.resolve(__dirname,'../../../Проект карты Тейванкаля/scripts');
function revive(value) {
 if(Array.isArray(value))return value.map(revive);
 if(value&&typeof value==='object') {
  if(value.__kingmaker_codec_v2__==='undefined'&&Object.keys(value).length===1)return undefined;
  if(value.__kingmaker_codec_v2__==='bigint')return BigInt(value.value);
  return Object.fromEntries(Object.entries(value).map(([k,v])=>[k,revive(v)]));
 }
 return value;
}
const input=revive(JSON.parse(fs.readFileSync(0,'utf8')));
if(input.action==='needs') {
 const rows=require(path.join(source,'population/needs-resolver.js')).resolveNeedQuantities(input.needs,input.reference);
 process.stdout.write(JSON.stringify(rows));
} else if(input.action==='orders') {
 process.stdout.write(JSON.stringify(require(path.join(source,'kingmaker/market.cjs')).buildNeedOrders(input.snapshot,input.profile||{})));
} else if(input.action==='clear') {
 input.snapshot.appliedEventIds=new Set();
 input.snapshot.reservations=[];
 const purposes=new Map(input.snapshot.accounts.map(a=>[a.owner_id,a.purpose]));
 const ops=require(path.join(source,'kingmaker/market.cjs')).clearLocalOrders(input.snapshot,input.period,input.orders,input.priorities,{sellerPurpose:s=>purposes.get(s.owner_id)||'employer_operating_capital'});
 process.stdout.write(JSON.stringify(ops,(_,v)=>typeof v==='bigint'?String(v):v));
} else if(input.action==='foundation') {
 input.snapshot.appliedEventIds=new Set();
 input.snapshot.reservations=[];
 const result=require(path.join(source,'kingmaker/foundation-market.cjs')).planFoundationMarket(input.snapshot,{period:input.period,orders:input.orders,priorities:input.priorities});
 process.stdout.write(JSON.stringify(result,(_,v)=>typeof v==='bigint'?String(v):v));
} else if(input.action==='inputs') {
 process.stdout.write(JSON.stringify(require(path.join(source,'kingmaker/foundation-market.cjs')).buildFoundationInputOrders(input.snapshot,{catalog:input.snapshot.registry.productionCatalog,plan:{activities:input.activities}})));
} else if(input.action==='food') {
 input.snapshot.appliedEventIds=new Set();input.snapshot.reservations=[];
 process.stdout.write(JSON.stringify(require(path.join(source,'kingmaker/food-conversion.cjs')).planFoodConversion(input.snapshot,{period:input.period,orders:input.orders,stage:input.stage}),(_,v)=>typeof v==='bigint'?String(v):v));
} else if(input.action==='cycle') {
 const {buildNeedOrders}=require(path.join(source,'kingmaker/market.cjs'));
 const {planFoundationMarket,buildFoundationInputOrders}=require(path.join(source,'kingmaker/foundation-market.cjs'));
 const {planFoodConversion,foodOrders,remainingFoodOrders}=require(path.join(source,'kingmaker/food-conversion.cjs'));
 const {applyOperations}=require(path.join(source,'kingmaker/ledger.cjs'));
 const {planFeasibleActivities,planFoundationPayroll}=require(path.join(source,'kingmaker/activity-plans.cjs'));
 const {planFoundationProduction}=require(path.join(source,'kingmaker/foundation-production.cjs'));
 let state=input.snapshot;state.appliedEventIds=new Set();state.reservations=[];const operations=[];
 const merge=change=>{operations.push(...change.operations);state=applyOperations(state,change.operations).state;state.registry={...state.registry,...change.registry};};
 const maintenance=buildNeedOrders(state).filter(o=>o.maintenance);
 merge(input.instant?planFoodConversion(state,{period:input.period+':maintenance',orders:maintenance,stage:'early'}):planFoundationMarket(state,{period:input.period+':maintenance',orders:maintenance,priorities:['basic']}));
 let early={operations:[]};if(input.instant){early=planFoodConversion(state,{period:input.period,orders:buildNeedOrders(state).filter(o=>!o.maintenance),stage:'early'});merge(early);}
 const args={period:input.period,catalog:state.registry.productionCatalog,policies:{},profile:{},maintenanceReceipts:[]};const plan=planFeasibleActivities(state,args);merge({operations:plan.storageOperations,registry:plan.registry});const paid=planFoundationPayroll(state,{...args,plan});merge({operations:paid,registry:{}});merge(planFoundationProduction(state,{...args,plan,paidLabor:paid}));
 let civilian=buildNeedOrders(state).filter(o=>!o.maintenance);const orders=[...(input.instant?foodOrders(civilian):civilian),...buildFoundationInputOrders(state,{catalog:state.registry.productionCatalog,plan})];let pending=input.instant?remainingFoodOrders(orders,early.operations):orders;
 if(input.instant){const late=planFoodConversion(state,{period:input.period,orders:pending,stage:'late'});merge(late);pending=remainingFoodOrders(pending,late.operations);}
 const market=planFoundationMarket(state,{period:input.period,orders:pending,priorities:plan.purchase_priorities});merge(market);
 if(input.instant){const actual=new Map();for(const o of market.operations)if(o.type==='purchase')actual.set(o.order_id,(actual.get(o.order_id)||0)+o.quantity);const residual=pending.map(o=>({...o,quantity:Math.max(0,o.quantity-(actual.get(o.id)||0))})).filter(o=>o.quantity>1e-10);merge(planFoodConversion(state,{period:input.period,orders:residual,stage:'surplus'}));}
 process.stdout.write(JSON.stringify({operations,accounts:state.accounts,stocks:state.stocks},(_,v)=>typeof v==='bigint'?String(v):v));
} else {throw Error('Unsupported isolated market oracle action');}
