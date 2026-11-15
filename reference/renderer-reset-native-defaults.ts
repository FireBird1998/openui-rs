// Pin the existing public-Value native-function boundary reached by Reset.
// The runner is the attributed transcription; evaluator/tree/store are upstream.
import {writeFileSync} from 'fs';
import {createStore} from './lang-core/src/runtime/store';
import {evaluateRoot} from './lang-core/src/runtime/evaluate-tree';
import {rendererState,unwrapFieldValue} from './renderer-actions';
const store=createStore(),events=[];
const root={type:'element',typeName:'Box',props:Object.fromEntries(['constructor','toString','__proto__'].map(name=>[name,{k:'UnaryOp',op:'!',operand:{k:'StateRef',n:name}}])),partial:false,hasDynamicProps:true};
const result={root,stateDeclarations:{}},ctx={getState:n=>unwrapFieldValue(store.get(n))};
const api=rendererState({store,evaluationContext:ctx,schema:{},resultRef:{current:result},queryManager:{},propsRef:{current:{onAction:e=>events.push(e)}}});
await api.triggerAction('',undefined,{steps:[{type:'reset',targets:['constructor','toString','__proto__']},{type:'continue_conversation',message:'after'}]});
const errors=[];
const reference=JSON.parse(JSON.stringify({events,snapshot:store.getSnapshot(),tree:{root:evaluateRoot(result,{ctx,store,library:{components:{Box:{props:{shape:{}}}}},errors}),errors}}));
// This is a deliberate, visible representation divergence, not an oracle:
// native functions project to undefined in Value, so !function becomes true.
const rust=structuredClone(reference);rust.tree.root.props.constructor=true;rust.tree.root.props.toString=true;
writeFileSync(new URL('../conformance/divergences/runtime-reset-native-defaults.json',import.meta.url),JSON.stringify({reason:'function-state-value',reference,rust})+'\n');
store.dispose();
