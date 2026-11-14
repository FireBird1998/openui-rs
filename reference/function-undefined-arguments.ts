import {compileSchema,parse} from './lang-core/src/parser/parser';
import {evaluateRoot} from './lang-core/src/runtime/evaluate-tree';
import {writeFileSync} from 'fs';
const cases=[];
for(const [name,properties,source]of [['optional',{a:{},b:{},c:{default:'d'}},'@K2(1,$o.zz)'],['prototype',JSON.parse('{"__proto__":{},"a":{}}'),'@K2($o)']]){
 const definition={params:{type:'object',properties}};const schema={$defs:{Box:{properties:{value:{}}}},functions:{K2:definition}};
 let received;const root=parse('root=Box('+source+')',compileSchema(schema)).root;
 evaluateRoot({root},{ctx:{getState:()=>({}),resolveRef:()=>null,functions:{K2:{...definition,fn:args=>{received=Object.entries(args).map(([key,value])=>({key,...(value===undefined?{undefined:true}:{value})}));return null;}}}},library:{components:{}},errors:[]});
 cases.push({name,definition,source:'root=Box('+source+')',reference:received,rust:name==='optional'?[{key:'a',value:1}]:[]});
}
writeFileSync('../conformance/divergences/function-argument-undefined.json',JSON.stringify(cases)+'\n');
