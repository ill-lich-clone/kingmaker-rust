'use strict';
const fs=require('node:fs'),path=require('node:path');
const root=path.resolve(__dirname,'../../../Проект карты Тейванкаля/scripts/kingmaker');
const input=JSON.parse(fs.readFileSync(0,'utf8'));
input.snapshot.appliedEventIds=new Set(input.snapshot.appliedEventIds||[]);
let result;
if(input.action==='artisan_quotes') {
 result=require(path.join(root,'foundation-artisan-policy.cjs')).buildArtisanQuotes(input.snapshot,{orders:input.orders,marketOperations:input.operations});
} else if(input.action==='artisan_choices') {
 const value=require(path.join(root,'foundation-artisan-policy.cjs')).planArtisanChoices(input.snapshot,{period:input.period,catalog:input.snapshot.registry.productionCatalog,quotes:input.quotes||[],opening:input.opening_phase||false});
 result={operations:value.operations,enterprises:value.registry.artisanEnterprises};
} else if(input.action==='artisan_choose') {
 result=require(path.join(root,'artisans.cjs')).chooseArtisanMethod(input.snapshot,{enterpriseId:input.enterprise_id,catalog:input.snapshot.registry.productionCatalog,marketQuotes:input.quotes||[],...(input.capital_budget_minor!=null?{capitalBudget:BigInt(input.capital_budget_minor)}:{})});
} else if(input.action==='capital_opening') {
 const orders=require(path.join(root,'market.cjs')).buildNeedOrders(input.snapshot,input.profile||{});
 const quotes=require(path.join(root,'foundation-artisan-policy.cjs')).buildArtisanQuotes(input.snapshot,{orders,marketOperations:[]});
 const value=require(path.join(root,'artisan-working-capital.cjs')).planArtisanWorkingCapital(input.snapshot,{period:input.period,catalog:input.snapshot.registry.productionCatalog,quotes,policies:input.policies||{}});
 result={operations:value.operations,enterprises:value.registry.artisanEnterprises};
} else if(input.action==='capital') {
 const value=require(path.join(root,'artisan-working-capital.cjs')).workingCapitalRequirements(input.snapshot,{quotes:input.quotes||[]});
 result={entities:[...value.entities.values()],groups:[...value.groups.values()]};
} else if(input.action==='artisan_plan') {
 const value=require(path.join(root,'foundation-artisan-policy.cjs')).planArtisanIncome(input.snapshot,{period:input.period,opening:input.opening,operations:input.operations,policies:input.policies||{}});
 result={operations:value.operations,artisans:value.registry.artisanEnterprises};
} else if(input.action==='producer_plan') {
 const value=require(path.join(root,'foundation-income.cjs')).planProducerIncome(input.snapshot,{period:input.period,opening:input.opening,operations:input.operations,policies:input.policies||{}});
 result={operations:value.operations,history:value.registry.producerIncome,diagnostics:value.result.blocked};
} else if(input.action==='producer_recognition') {
 const value=require(path.join(root,'foundation-income.cjs')).planProducerIncome(input.snapshot,{period:input.period,opening:input.opening,operations:input.operations,policies:input.policies||{}});
 result=value.registry.producerIncome.map(row=>({owner_id:row.owner_id,currency_id:row.currency_id,revenue_minor:row.revenue_minor,opening_stock_revenue_minor:row.opening_stock_revenue_minor,recognized_cost_minor:row.recognized_cost_minor,diagnostics:[]}));
} else if(input.action==='route') {
 result=require(path.join(root,'supply.cjs')).routeReason(input.snapshot.registry,input.route,input.offer);
} else throw Error('Unknown income oracle action');
process.stdout.write(JSON.stringify(result,(_,v)=>typeof v==='bigint'?String(v):v));
