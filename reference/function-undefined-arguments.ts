// Run from reference/: bun --no-install function-undefined-arguments.ts
// Reference output only; Rust expectations remain in conformance/divergences/.
process.env.OPENUI_TELEMETRY_DISABLED='1';process.env.DO_NOT_TRACK='1';
const {compileSchema,parse}=await import('./lang-core/src/parser/parser');
const {evaluateRoot}=await import('./lang-core/src/runtime/evaluate-tree');
import {writeFileSync} from 'fs';
const cases=[];
for(const [name,properties,source]of [['optional',{a:{},b:{},c:{default:'d'}},'@K2(1,$o.zz)'],['prototype',JSON.parse('{"__proto__":{},"a":{}}'),'@K2($o)']]){
 const definition={params:{type:'object',properties}};const schema={$defs:{Box:{properties:{value:{}}}},functions:{K2:definition}};
 let received;const root=parse('root=Box('+source+')',compileSchema(schema)).root;
 evaluateRoot({root},{ctx:{getState:()=>({}),resolveRef:()=>null,functions:{K2:{...definition,fn:args=>{received=Object.entries(args).map(([key,value])=>({key,...(value===undefined?{undefined:true}:{value})}));return null;}}}},library:{components:{}},errors:[]});
 cases.push({name,definition,source:'root=Box('+source+')',reference:received});
}
writeFileSync('../conformance/golden/function-argument-undefined.json',JSON.stringify(cases.map(({name,reference})=>({name,reference})))+'\n');
