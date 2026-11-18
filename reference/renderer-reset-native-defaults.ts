// Pin the existing public-Value native-function boundary reached by Reset.
// The runner is the attributed transcription; evaluator/tree/store are upstream.
import {writeFileSync} from 'fs';
process.env.OPENUI_TELEMETRY_DISABLED='1';process.env.DO_NOT_TRACK='1';
const {createStore}=await import('./lang-core/src/runtime/store');
const {evaluateRoot}=await import('./lang-core/src/runtime/evaluate-tree');
const {rendererState,unwrapFieldValue}=await import('./renderer-actions');
const store=createStore(),events=[];
const root={type:'element',typeName:'Box',props:Object.fromEntries(['constructor','toString','__proto__'].map(name=>[name,{k:'UnaryOp',op:'!',operand:{k:'StateRef',n:name}}])),partial:false,hasDynamicProps:true};
const result={root,stateDeclarations:{}},ctx={getState:n=>unwrapFieldValue(store.get(n))};
const api=rendererState({store,evaluationContext:ctx,schema:{},resultRef:{current:result},queryManager:{},propsRef:{current:{onAction:e=>events.push(e)}}});
await api.triggerAction('',undefined,{steps:[{type:'reset',targets:['constructor','toString','__proto__']},{type:'continue_conversation',message:'after'}]});
const errors=[];
const reference=JSON.parse(JSON.stringify({events,snapshot:store.getSnapshot(),tree:{root:evaluateRoot(result,{ctx,store,library:{components:{Box:{props:{shape:{}}}}},errors}),errors}}));
writeFileSync(new URL('../conformance/golden/runtime-reset-native-defaults.json',import.meta.url),JSON.stringify(reference)+'\n');
store.dispose();
