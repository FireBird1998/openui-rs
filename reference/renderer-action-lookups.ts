// Direct probes of renderer-actions.ts's attributed renderer transcription.
// All store/evaluation operations use lang-core. No React or tool execution.
import {writeFileSync} from 'fs';
import {createStore} from './lang-core/src/runtime/store';
import {rendererState} from './renderer-actions';
const definitions=[undefined,null,false,'\ud800',[],['x'],{Go:{params:{}}}];
const types=[undefined,null,false,1,'','constructor','length','0','Go',['Go'],{},{toString:null,valueOf:null}];
const cases=[];
for(const actions of definitions)for(const type of types){
 const index=cases.length,formName=index%2?'contact':undefined;
 const action={...(type===undefined?{}:{type}),params:index%3===0?['💛','\ud800','\udfff','\u{f0000}'][Math.floor(index/3)%4]:index%3===1?[1,null]:{url:'old',constructor:2},url:'new',context:index%2?{n:1}:0};
 const store=createStore();store.set('$n',1);const events=[];
 const api=rendererState({store,evaluationContext:{},schema:{actions},resultRef:{current:{}},queryManager:{},propsRef:{current:{onAction:e=>events.push(e)}}});
 api.setFieldValue('contact','Input','x',2,false);
 let failed=false;try{await api.triggerAction('legacy',formName,action);}catch{failed=true;}
 cases.push({...(actions===undefined?{}:{actions}),action,...(formName===undefined?{}:{formName}),events,failed});
 store.dispose();
}
writeFileSync(new URL('../conformance/golden/runtime/renderer-action-lookups.json',import.meta.url),JSON.stringify(cases)+'\n');
console.log(`Renderer action lookups: ${cases.length} observations.`);
