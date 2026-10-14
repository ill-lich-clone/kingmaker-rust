'use strict';
// Independent oracle: the existing game decoder supplies semantics.
const fs=require('node:fs'),path=require('node:path');
const codec=require(path.resolve(__dirname,'../../../Проект карты Тейванкаля/scripts/kingmaker/state-codec.cjs'));
const input=JSON.parse(fs.readFileSync(0,'utf8'));
const dictionary=new Map(input.dictionary),shared={};
for(const [kind,n,payload] of input.shared){(shared[kind] ||= [])[n]=payload;}
const decoded=codec.decodeRecord(input.record.table,input.record,{resolve(n){
  if(!dictionary.has(n))throw Error('Unknown dictionary ID '+n);return dictionary.get(n);
}},shared);
const tag=codec.CODEC_TAG;
function normalize(value){
  if(value===undefined)return {[tag]:'undefined'};
  if(typeof value==='bigint')return {[tag]:'bigint',value:String(value)};
  if(typeof value==='number'&&(!Number.isFinite(value)||Object.is(value,-0)))return {[tag]:'number',value:Object.is(value,-0)?'-0':String(value)};
  if(value instanceof Date)return {[tag]:'date',value:value.toISOString()};
  if(value instanceof Set)return {[tag]:'set',value:[...value].map(normalize)};
  if(value instanceof Map)return {[tag]:'map',value:[...value].map(([k,v])=>[normalize(k),normalize(v)])};
  if(Array.isArray(value))return value.map(normalize);
  if(value&&typeof value==='object')return Object.fromEntries(Object.entries(value).map(([k,v])=>[k,normalize(v)]));
  return value;
}
process.stdout.write(JSON.stringify(normalize(decoded)));
