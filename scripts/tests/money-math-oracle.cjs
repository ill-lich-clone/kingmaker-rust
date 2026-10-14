'use strict';
const path=require('node:path');
const {multiplyMoney}=require(path.resolve(__dirname,'../../../Проект карты Тейванкаля/scripts/kingmaker/ledger.cjs'));
const values=JSON.parse(process.argv[2]);
console.log(JSON.stringify(values.map(value=>({text:String(value),minor:multiplyMoney(101n,value).toString()}))));
