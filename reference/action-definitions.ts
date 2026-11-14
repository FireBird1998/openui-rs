// Direct pinned evaluator probes for action definition fields, independent of
// compileSchema's acceptance. From reference/: bun --no-install action-definitions.ts
import {evaluateRoot} from './lang-core/src/runtime/evaluate-tree';
import {writeFileSync} from 'fs';
const variants=[];
const base=()=>({type:'object',properties:{x:{type:'string'}}});
for(const key of ['required','properties','type'])for(const [i,v] of [undefined,null,false,1,'xy',[],{},['x']].entries()){
 const params=base();delete params[key];if(v!==undefined)params[key]=v;
 variants.push([key+'-'+i,{Probe:{params}}]);
}
for(const [i,v] of [undefined,null,false,1,'x',[],{}, {some:1},{some:null},{includes:1}].entries()){
 if(i<7)variants.push(['params-'+i,{Probe:i===0?{}:{params:v}}]);
 else variants.push(['required-method-'+i,{Probe:{params:{...base(),required:v}}}]);
}
for(const [i,v] of [null,false,1,'x',[],{}].entries())variants.push(['definition-'+i,{Probe:v}]);
for(const [i,v] of [undefined,null,false,1,'fallback',[],{}, {nested:1}].entries()){
 const x={type:'string'};if(v!==undefined)x.default=v;
 variants.push(['default-'+i,{Probe:{params:{type:'object',properties:{x},required:['x']}}}]);
}
variants.push(['numeric-order',{Probe:{params:{type:'object',properties:{z:{},'2':{},'0':{}}}}}]);
for(const key of ['constructor','toString','__proto__','01','0','length']) {
 variants.push(['required-key-'+key,{Probe:{params:{type:'object',properties:{},required:[key]}}}]);
 variants.push(['array-properties-'+key,{Probe:{params:{type:'object',properties:[{default:'zero'},{default:'one'}],required:[key]}}}]);
}
for(const key of [null,1,false,[],{},['x']])variants.push(['required-data-key-'+JSON.stringify(key),{Probe:{params:{type:'object',properties:{x:{}},required:[key]}}}]);
const cases=[];
for(const [name,actions] of variants)for(const [scenario,args] of [['value',[{k:'Str',v:'x'},{k:'Num',v:2},{k:'Bool',v:false}]],['null',[{k:'Null'}]],['missing',[]],['mapped',[{k:'Str',v:'x'}]]]){
 const node={k:'Comp',name:'Probe',args,...(scenario==='mapped'?{mappedProps:{x:{k:'Str',v:'x'}}}:{})};
 const root={type:'element',typeName:'Box',props:{a:node,b:{k:'Arr',els:[node]}},partial:false,hasDynamicProps:true};
 const errors=[];const result=evaluateRoot({root},{ctx:{actions,getState:()=>undefined,resolveRef:()=>null},library:{components:{Box:{props:{shape:{a:{},b:{}}}}}},errors});
 cases.push({name:name+'/'+scenario,actions,root,expected:{root:result,errors}});
}
writeFileSync('../conformance/golden/action-definitions.json',JSON.stringify(cases)+'\n');console.log(cases.length+' action-definition observations');
