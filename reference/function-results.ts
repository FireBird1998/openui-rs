// Returned values take the same evaluator/prop path as other computed values.
import {evaluateRoot} from './lang-core/src/runtime/evaluate-tree';
import {runtimeFunctions} from './function-implementations';
import {readFileSync,writeFileSync} from 'fs';
const schema=JSON.parse(readFileSync('../conformance/schemas/runtime-functions.json','utf8'));
const functions=runtimeFunctions(schema.functions);
const cases=[];
for(const name of ['Huge','Deep','AstData','ElementData','Throw','Wrong','Undefined','Null']) {
 const root={type:'element',typeName:'Box',props:{value:{k:'Comp',name,args:[]}},partial:false,hasDynamicProps:true};
 const errors=[];const value=evaluateRoot({root},{ctx:{functions,getState:()=>undefined,resolveRef:()=>null},library:{components:{Box:{props:{shape:{value:{}}}}}},errors});
 cases.push({name,root,expected:{root:value,errors}});
}
writeFileSync('../conformance/golden/function-results.json',JSON.stringify(cases)+'\n');
