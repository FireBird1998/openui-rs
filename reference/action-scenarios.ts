// Interaction generation only. All language operations use lang-core; the
// renderer-specific handlers live in the separately attributed transcription.
import { ownSteps } from './lang-core/src/runtime/evaluator';

function discover(value, source, out=[], path=[]) {
 if(!value || typeof value!=='object')return out;
 if(Array.isArray(value.steps))out.push({source,path,value});
 for(const [key,child] of Object.entries(value))discover(child,source,out,[...path,key]);
 return out;
}
export async function actionScenarios(c,saved,h) {
 const scenarios={};
 for(const scenario of ['defaults','persisted']) {
  const real=h.createStore(),pristine=new Set<string>(),names=new Set<string>();
  const store={...real,
   initialize(defaults,persisted){
    for(const key of Object.keys(persisted)){names.add(key);pristine.delete(key);}
    for(const key of Object.keys(defaults)){names.add(key);if(!Object.hasOwn(real.getSnapshot(),key)&&!Object.hasOwn(persisted,key))pristine.add(key);}
    real.initialize(defaults,persisted);
   },
   set(name,value,options){names.add(name);if(options?.pristine)pristine.add(name);else pristine.delete(name);real.set(name,value,options);},
   dispose(){pristine.clear();real.dispose();}
  };
  let result=h.parse(h.input(c),h.library(c),c.root),capture:any={},referenceValues=c.referenceValues;
  const currentCase=()=>({...c,referenceValues});
  if(c.queryComplete===false)result={...result,queryStatements:result.queryStatements.map(q=>({...q,complete:false}))};
  let observation=JSON.parse(h.observe(currentCase(),result,store,scenario==='persisted'?saved:{},null,{capture}));
  const plans=discover(capture.root,'tree');discover(store.getSnapshot(),'state',plans);
  if(!plans.length&&!c.actionInteractions)continue;
  const markers=[];
  const markerWalk=v=>{if(!v||typeof v!=='object')return;if(v.__reactive==='assign'){markers.push(v);return;}for(const child of Object.values(v))markerWalk(child);};
  markerWalk(capture.root);
  const handles=plans.map(p=>({...p,value:p.value,generation:0}));
  const next=h.generator(parseInt(h.fp(c.name+':actions'),16));
  const order=handles.map((_,i)=>i);
  for(let i=order.length-1;i>0;i--){const j=Math.floor(next()*(i+1));[order[i],order[j]]=[order[j],order[i]];}
  const ops:any[]=[];
  for(const handle of c.actionSequenceOnly?[]:order){
   ops.push({kind:'run',handle,message:'click-'+Math.floor(next()*100),...(next()<.5?{formName:'contact'}:{})});
   if(markers.length)ops.push({kind:'binding',marker:markers[handle%markers.length],value:Math.floor(next()*20)});
   ops.push({kind:'field',name:next()<.5?'toString':'name',componentType:'Input',value:'field-'+Math.floor(next()*100),formName:next()<.5?'contact':'',save:next()<.5});
  }
  if(handles.length&&!c.actionSequenceOnly){
   ops.push({kind:'run',handle:0,message:'again'},{kind:'redraw'},{kind:'run',handle:0,message:'third'});
   ops.push({kind:'retain',handle:0},{kind:'runState',message:'stored'});
   ops.push({kind:'program',program:h.input(c)+'\n$streamed = 99'},{kind:'run',handle:0,message:'after stream'});
   ops.push({kind:'dispose'},{kind:'run',handle:0,message:'after dispose'});
  }
  for(const op of c.actionInteractions??[])ops.push(op);
  const steps=[];let generation=0,active:any;
  const records=new Set<any>(),tokens=new Map<any,any>();
  const resultRef={get current(){return result;}};
  const makeContext=(managers)=>{
   const manager=h.createQueryManager(null);managers.push(manager);let registered=false;
   manager.registerMutations((result.mutationStatements??[]).map(m=>({statementId:m.statementId,toolName:m.statementId})));
   const ctx:any={functions:h.runtimeFunctions(h.rawSchemas.get(c.schema).functions,active.callCounts),actions:h.rawSchemas.get(c.schema).actions,getState:n=>h.unwrapFieldValue(store.get(n)),resolveRef:n=>{
    const q=result.queryStatements?.find(q=>q.statementId===n),m=result.mutationStatements?.some(m=>m.statementId===n);
    if((q||m)&&Object.hasOwn(referenceValues??{},n))return referenceValues[n];
    if(q)return registered?manager.getResult(n):q.complete?(q.defaultsAST?h.evaluate(q.defaultsAST,ctx):null):null;
    return m?manager.getMutationResult(n):null;
   }};
   manager.evaluateQueries((result.queryStatements??[]).map(q=>({statementId:q.statementId,toolName:q.statementId,args:null,defaults:q.complete&&q.defaultsAST?h.evaluate(q.defaultsAST,ctx):null,complete:q.complete})));
   registered=true;return ctx;
  };
  const drain=async record=>{
   await Promise.resolve();await Promise.resolve();
   if(!record.token)await record.promise;
  };
  const apiFor=(record,formName)=>h.rendererState({store,schema:h.rawSchemas.get(c.schema),resultRef,evaluationContext:{},
   evaluate:expr=>h.evaluate(expr,record.context??(record.context=makeContext(record.managers))),
   propsRef:{current:{onAction:e=>active.events.push(e),onStateUpdate:()=>{active.saveRequested=true;}}},
   queryManager:{invalidate:ids=>active.events.push({type:'run',params:{statementId:ids[0],refType:'query'},humanFriendlyMessage:'',formState:record.api.getFormPayload(formName),formName}),
    fireMutation:(id,args)=>{
     active.events.push({type:'run',params:{statementId:id,refType:'mutation',args},humanFriendlyMessage:'',formState:record.api.getFormPayload(formName),formName});
     return new Promise(resolve=>{record.token={record,resolve,generation,consumed:false};});
    }}
  });
  for(let index=0;index<ops.length;index++) {
   const op=ops[index],schema=h.rawSchemas.get(c.schema),definitions=schema.functions;
   active={events:[],callCounts:definitions?Object.fromEntries(Object.keys(definitions).map(n=>[n,0])):undefined};
   let fieldValue,failed=false,suspended;
   const local={managers:[],context:undefined,api:undefined};local.api=apiFor(local,op.formName);
   for(const record of records)if(record.context)record.context.functions=h.runtimeFunctions(definitions,active.callCounts);
   try {
    if(['run','runState','legacy','none'].includes(op.kind)){
     let action=op.kind==='run'?handles[op.handle].value:op.kind==='runState'?store.get('$retained'):op.action;
     if(Array.isArray(action?.steps)){
      action=ownSteps(action);
      if(op.kind==='run'&&handles[op.handle].generation!==generation)action={...action,steps:action.steps.filter(s=>s?.type!=='custom_action')};
     }
     const record:any={managers:[],context:undefined,api:undefined,token:undefined,failed:false};record.api=apiFor(record,op.formName);records.add(record);
     record.promise=record.api.triggerAction(op.message,op.formName,action).catch(()=>{record.failed=true;});
     await drain(record);failed=record.failed;suspended=!!record.token;
     if(record.token)tokens.set(op.slot??'current',record.token);
    } else if(op.kind==='resume') {
     const token=tokens.get(op.slot??'current');
     if(!token||token.consumed||token.generation!==generation){failed=true;suspended=false;}
     else {
      token.consumed=true;const record=token.record;record.token=undefined;token.resolve(op.success);
      await drain(record);failed=record.failed;suspended=!!record.token;
      if(record.token)tokens.set(op.into??op.slot??'current',record.token);
     }
    } else if(op.kind==='field') {
     local.api.setFieldValue(op.formName,op.componentType,op.name,op.value,op.save);
     fieldValue=local.api.getFieldValue(op.formName,op.name);
    } else if(op.kind==='binding') {
     h.resolveStateField(op.name??'field',op.marker,store,makeContext(local.managers),n=>local.api.getFieldValue(op.formName,n),(n,v)=>local.api.setFieldValue(op.formName,undefined,n,v)).setValue(op.value);
    } else if(op.kind==='default') {
     h.persistDefaultValue({isStreaming:op.streaming,existingValue:local.api.getFieldValue(op.formName,op.name),defaultValue:op.value,formName:op.formName,componentType:op.componentType,name:op.name,shouldTriggerSaveCallback:op.save??false},local.api.setFieldValue);
     fieldValue=local.api.getFieldValue(op.formName,op.name);
    } else if(op.kind==='retain') {
     h.resolveStateField('plan',{__reactive:'assign',target:'$retained',expr:{k:'StateRef',n:'$value'}},store,makeContext(local.managers),()=>undefined,()=>{}).setValue(handles[op.handle].value);
    } else if(op.kind==='program') {
     result=h.parse(op.program,h.library(c),c.root);store.initialize(result.stateDeclarations??{},{});
    } else if(op.kind==='references') {
     referenceValues=op.values;
    } else if(op.kind==='dispose') {
     store.dispose();generation++;
     // ADR 0004 revokes suspended runs too. Resolve their real await as false
     // to release promises; stale opaque tokens remain available for refusal.
     for(const record of records)if(record.token){const token=record.token;record.token=undefined;token.resolve(false);await drain(record);}
    }
   } catch(e) { failed=true; }
   finally {for(const manager of local.managers)manager.dispose();}
   if(suspended&&!c.manualMutationOutcomes)ops.splice(index+1,0,{kind:'resume',success:true,slot:op.into??op.slot??'current'});
   observation=JSON.parse(h.observe(currentCase(),result,store,{},null,{skipInitialize:true,capture}));
   const state={events:active.events,failed,...(suspended!==undefined?{suspended}:{}),...(active.saveRequested!==undefined?{saveRequested:active.saveRequested}:{}),...(fieldValue!==undefined?{fieldValue}:{}),pristine:[...pristine].filter(n=>typeof n==='string').sort(),...observation,...(active.callCounts?{actionCallCounts:active.callCounts}:{})};
   steps.push({op,observation:state});
  }
  for(const record of records){if(record.token){record.token.resolve(false);record.token=undefined;}await drain(record);for(const manager of record.managers)manager.dispose();}
  scenarios[scenario]={handles:handles.map(({source,path})=>({source,path})),steps};
 }
 if(!Object.keys(scenarios).length)return null;
 // Intern entire observed fields. Their JSON is still compared byte for byte;
 // repeated unchanged trees do not bloat each interaction or its parsed heap.
 const values=[],ids=new Map();
 for(const s of Object.values(scenarios) as any[])for(const step of s.steps)for(const [key,value] of Object.entries(step.observation)){
  if(value&&typeof value==='object'){
   const json=JSON.stringify(value);let id=ids.get(json);
   if(id===undefined){id=values.length;ids.set(json,id);values.push(value);}
   step.observation[key]={$value:id};
  }
 }
 return {values,...scenarios};
}
