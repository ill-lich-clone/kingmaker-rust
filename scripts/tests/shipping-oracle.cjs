'use strict';
const fs=require('node:fs'),path=require('node:path');
const root=path.resolve(__dirname,'../../../Проект карты Тейванкаля/scripts/kingmaker');
const input=JSON.parse(fs.readFileSync(0,'utf8'));
input.snapshot.appliedEventIds=new Set(input.snapshot.appliedEventIds||[]);
let result;
if(input.action==='arrivals')result=require(path.join(root,'supply.cjs')).planShipmentArrivals(input.snapshot,{period:input.period,date:input.snapshot.campaign.date});
else if(input.action==='dispatch')result=require(path.join(root,'supply.cjs')).planSupplyOrders(input.snapshot,{period:input.period,orders:input.orders},{actor:{kind:'entity',id:input.orders[0].buyer_id}});
else if(input.action==='physical'){
 const active=input.active?{ledgerSession:require(path.join(root,'ledger.cjs')).createLedgerSession(input.snapshot,{cow:true})}:undefined;
 result=require(path.join(root,'foundation-supply.cjs')).planPhysicalSupply(input.snapshot,{period:input.period,orders:input.orders,marketOperations:input.marketOperations||[],foodOrders:input.foodOrders||[],active});
}
else if(input.action==='interrupt')result=require(path.join(root,'supply.cjs')).planShipmentInterruption(input.snapshot,input.request);
else if(input.action==='resume')result=require(path.join(root,'supply.cjs')).planShipmentRecovery(input.snapshot,input.request,{actor:{kind:'entity',id:input.actor}});
else if(input.action==='return')result=require(path.join(root,'supply.cjs')).planShipmentReturn(input.snapshot,input.request,{actor:{kind:'entity',id:input.actor}});
else if(input.action==='food-forecast'){
 const f=require(path.join(root,'food-supply-forecast.cjs')).buildFoodSupplyForecast(input.snapshot,{orders:input.orders});
 const forecasts=[];for(const step of input.steps){const o=input.orders[step.order];forecasts.push(f.forecastFor(o,{travel_days:step.days}));if(step.quantity)f.registerDispatch(o,step.quantity,step.date);}
 result={forecasts,diagnostics:f.diagnostics};
}
else if(input.action==='food-reserves')result=require(path.join(root,'food-supply-forecast.cjs')).planLocalFoodReserves(input.snapshot,{period:input.period,orders:input.orders});
else if(input.action==='state-procurement')result=require(path.join(root,'state-procurement.cjs')).planStateRawProcurement(input.snapshot,{period:input.period,policies:input.policies});
else if(input.action==='trade-offers')result=require(path.join(root,'foundation-network.cjs')).planTradeOffers(input.snapshot,{period:input.period}).registry.saleOffers;
else if(input.action==='warehouse-capacity')result=require(path.join(root,'storage-capacity.cjs')).deriveStorageCapacities(input.snapshot);
else throw Error('Unknown shipping oracle action');
process.stdout.write(JSON.stringify(result,(_,v)=>typeof v==='bigint'?String(v):v));
