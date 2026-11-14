// Runtime probes bypass parser compilation so malformed definitions reach calls.
import {evaluateRoot} from './lang-core/src/runtime/evaluate-tree';
import {writeFileSync} from 'fs';
const variants=[];
const base=()=>({type:'object',properties:{x:{type:'string'}}});
const kinds=[undefined,null,false,1,'xy',[],{},['x']];
for(const key of ['required','properties','type'])for(const [i,v] of kinds.entries()){
 const params=base();delete params[key];if(v!==undefined)params[key]=v;
 variants.push([key+'-'+i,{params}]);
}
for(const [i,v] of kinds.entries())variants.push(['params-'+i,i===0?{}:{params:v}]);
for(const [i,v] of [null,false,1,'x',[],{}].entries())variants.push(['definition-'+i,v]);
for(const [i,v] of kinds.entries()){
 const x={type:'string'};if(v!==undefined)x.default=v;
 variants.push(['default-'+i,{params:{type:'object',properties:{x},required:['x']}}]);
 variants.push(['returns-'+i,{params:base(),...(v!==undefined?{returns:v}:{})}]);
}
for(const returns of [{type:'string'},{type:'array',items:{type:'number'}},{type:'object',properties:{x:{type:'number',default:7}},required:['x']},{enum:['a']},{type:'object',properties:{x:{type:'string'}}}])variants.push(['return-schema-'+JSON.stringify(returns),{params:base(),returns}]);
for(const key of ['constructor','toString','__proto__','hasOwnProperty','valueOf','length','0','01']) {
 variants.push(['required-'+key,{params:{type:'object',properties:{},required:[key]}}]);
 variants.push(['param-'+key,{params:{type:'object',properties:JSON.parse('{"'+key+'":{"type":"string"}}')}}]);
}
for(const key of [null,1,false,[],{},['x']])variants.push(['required-data-'+JSON.stringify(key),{params:{type:'object',properties:{x:{}},required:[key]}}]);
variants.push(['undefined-default-no-type',{params:{properties:{toString:{default:null}},required:['toString']}}]);
for(const fallback of [null,7])variants.push(['coerced-required-default-'+fallback,{params:{type:'object',properties:{x:{default:fallback}},required:[['x']]}}]);
const cases=[];
for(const [name,definition]of variants)for(const [scenario,args,mappedProps]of [['value',[{k:'Str',v:'x'},{k:'Num',v:2}]],['null',[{k:'Null'}]],['missing',[]],['mapped',[],{x:{k:'Num',v:3},extra:{k:'Str',v:'extra'}}],['mapped-null',[],{x:{k:'Null'}}],['mapped-undefined',[],{toString:{k:'Str'}}],['raw-null',[null]],['mapped-raw-null',[],{x:null}],['mapped-keys',[],JSON.parse('{"__proto__":{"k":"Str","v":"own"},"constructor":{"k":"Num","v":2},"toString":{"k":"Str","v":"own"}}')]])for(const [outcome,result]of [['args'],['undefined'],['null',null],['wrong',5],['object',{x:3}],['array',[1,'bad',2]],['throw','boom']]){
 const node={k:'Comp',name:'Probe',args,...(mappedProps?{mappedProps}:{})};
 const root={type:'element',typeName:'Box',props:{a:node,b:{k:'Arr',els:[node]}},partial:false,hasDynamicProps:true};
 const fn=a=>{if(outcome==='throw')throw Error(result);return outcome==='args'?a:result;};
 const functions={Probe:definition&&typeof definition==='object'?{...definition,fn}:definition};
 const errors=[];const value=evaluateRoot({root},{ctx:{functions,getState:()=>undefined,resolveRef:()=>null},library:{components:{Box:{props:{shape:{a:{},b:{}}}}}},errors});
 cases.push({name:name+'/'+scenario+'/'+outcome,definition,outcome,...(result!==undefined?{result}:{}),root,expected:{root:value,errors}});
}
writeFileSync('../conformance/golden/function-definitions.json',JSON.stringify(cases)+'\n');console.log(cases.length+' function-definition observations');
