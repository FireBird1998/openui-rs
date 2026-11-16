// Runs the pinned parser, store and tree evaluator. The renderer-only runner
// is explicitly transcribed in renderer-actions.ts.
// From reference/: bun --no-install runtime.ts golden
// Diagnose: bun --no-install runtime.ts show <case> <scenario> <chunking> <step>
import { readFileSync, writeFileSync, mkdirSync, existsSync, rmSync } from 'fs';
import { join, dirname } from 'path';
import {runtimeFunctions} from './function-implementations';
import {rendererState,unwrapFieldValue,persistDefaultValue} from './renderer-actions';
import {actionScenarios} from './action-scenarios';
import { plugin } from 'bun';
// Same read-only export hook as run.ts; vendored parser stays untouched.
plugin({name:'runtime-parser-stages',setup(build){build.onLoad({filter:/lang-core\/src\/parser\/parser\.ts$/},({path})=>({contents:readFileSync(path,'utf8')+'\nexport { preprocess };\n',loader:'ts'}));}});
process.env.OPENUI_TELEMETRY_DISABLED = '1';
process.env.DO_NOT_TRACK = '1';
const { compileSchema, parse, createStreamParser, preprocess } = await import('./lang-core/src/parser/parser');
const { evaluateRoot } = await import('./lang-core/src/runtime/evaluate-tree');
const { evaluate } = await import('./lang-core/src/runtime/evaluator');
const { markReactive, isReactiveSchema } = await import('./lang-core/src/reactive');
const { resolveStateField } = await import('./lang-core/src/runtime/state-field');
const { createStore } = await import('./lang-core/src/runtime/store');
const { createQueryManager } = await import('./lang-core/src/runtime/queryManager');
const { tokenize } = await import('./lang-core/src/parser/lexer');
const { autoClose, split } = await import('./lang-core/src/parser/statements');
const { parseExpression } = await import('./lang-core/src/parser/expressions');
const C = join(import.meta.dir, '../conformance'), G = join(C, 'golden/runtime');
const manifests = ['manifest','imported','fuzz','synthetic','recorded','runtime'].map(n => JSON.parse(readFileSync(join(C,n+'.json'),'utf8')));
const cases = manifests.flatMap(m => m.cases), schemas = Object.assign({}, ...manifests.map(m => m.schemas));
const chunkings = manifests[0].chunkings, libraries = new Map(), rawSchemas = new Map(), runtimeLibraries=new Map();
const library = c => {
  if (!libraries.has(c.schema)) libraries.set(c.schema, compileSchema(JSON.parse(readFileSync(join(C, schemas[c.schema]),'utf8'))));
  return libraries.get(c.schema);
};
const bindingSpecs=JSON.parse(readFileSync(join(C,'bindings.json'),'utf8'));
const specs=Object.fromEntries(Object.entries(bindingSpecs).map(([name,path])=>[name,JSON.parse(readFileSync(join(C,path),'utf8')).library]));
// This synthetic component uses real upstream schemas, not signature parsing,
// so the oracle distinguishes an inner reactive type from an outer prop mark.
const { z }=await import('zod');
const { createLibrary, defineComponent }=await import('./lang-core/src/library');
const item=z.string();markReactive(item);
const boundList=z.array(z.string()),boundUnion=z.union([z.string(),z.number()]);
markReactive(boundList);markReactive(boundUnion);
const bindingTypesProps=z.object({list:z.array(item),union:z.union([item,z.number()]),boundList,boundUnion});
const bindingTypesLibrary=createLibrary({components:[defineComponent({name:'BindingTypes',description:'Binding annotations',props:bindingTypesProps,component:null})],root:'BindingTypes'});
const oddBound=()=>{const schema=z.string();markReactive(schema);return schema;};
const oddProps=z.object({quoted:z.enum(['a"b','c']),afterQuote:oddBound(),slash:z.literal('end\\'),afterSlash:oddBound(),'a:b':oddBound(),'x?':oddBound(),'lt<':oddBound(),'p(':oddBound(),'c,':oddBound(),"it's":oddBound(),last:oddBound()});
const oddLibrary=createLibrary({components:[defineComponent({name:'SignatureOdd',description:'Odd signature text',props:oddProps,component:null})],root:'SignatureOdd'});
const liveProps={BindingTypes:[bindingTypesProps,bindingTypesLibrary],SignatureOdd:[oddProps,oddLibrary]};
// These literal signatures cannot all be disambiguated from their text.
// Keep the actual weak-set marks alongside the spec for helper diagnostics.
const diagnosticBinding=schema=>{markReactive(schema);return schema;};
const diagnosticProps={
  E10:z.object({e10:diagnosticBinding(z.literal('a"b')),after:oddBound()}),
  E11:z.object({e11:diagnosticBinding(z.literal('\\')),after:oddBound()}),
  E12:z.object({'a, b':oddBound(),b:z.string(),after:oddBound()}),
  E13:z.object({'a, b':oddBound(),after:oddBound()}),
};
const diagnosticLibrary=createLibrary({components:Object.entries(diagnosticProps).map(([name,props])=>defineComponent({name,description:'Binding extraction diagnostics',props,component:null})),root:'E10'});
const runtimeLibrary=c=>{
  if(!runtimeLibraries.has(c.schema)) {
    const spec=specs[c.schema];
    const components=Object.fromEntries([...library(c)].map(([n,d])=>{
      const signature=spec?.components?.[n]?.signature??'';
      if(c.schema==='runtime-bindings'&&Object.hasOwn(liveProps,n)) {
        const [props,library]=liveProps[n];
        if(signature!==library.toSpec().components[n].signature)throw Error(n+' signature drift');
        return [n,{props}];
      }
      const bound=new Set([...signature.matchAll(/(\w+)\??: \$binding</g)].map(m=>m[1]));
      return [n,{props:{shape:Object.fromEntries(d.params.map(p=>{
        // Fresh identities: markReactive never mutates the compiled schema.
        const schema={...p.schema};if(bound.has(p.name))markReactive(schema);
        return [p.name,schema];
      }))}}];
    }));
    runtimeLibraries.set(c.schema,{components});
  }
  return runtimeLibraries.get(c.schema);
};
const input = c => c.fuzzInput ?? readFileSync(join(C,c.file),'utf8');
const fp = s => { let h = 0x811c9dc5; for (const b of new TextEncoder().encode(s)) h = Math.imul(h ^ b, 0x01000193) >>> 0; return h.toString(16).padStart(8,'0'); };
// Canonical run-length encoding: a singleton hash, or [hash, count >= 2].
function runs(hashes) {
  const out=[];
  for(let i=0;i<hashes.length;) {
    let end=i+1;while(end<hashes.length&&hashes[end]===hashes[i])end++;
    out.push(end-i===1?hashes[i]:[hashes[i],end-i]);i=end;
  }
  return out;
}
function generator(seed) { let a=seed>>>0; return () => { a=(a+0x6d2b79f5)>>>0; let t=a; t=Math.imul(t^(t>>>15),t|1); t^=t+Math.imul(t^(t>>>7),t|61); return ((t^(t>>>14))>>>0)/4294967296; }; }
function plan(c,k) {
  // Same chunkings as the core, including seeded random and resync plans.
  const path=join(C,'golden',c.name+'.steps.json');
  if (existsSync(path)) { const p=JSON.parse(readFileSync(path,'utf8'))[k]; if(p?.ops) return {ops:p.ops}; if(p?.sizes) return {sizes:p.sizes}; }
  if(k==='recorded') return {sizes:c.recorded};
  const n=Array.from(input(c)).length, next=generator(parseInt(fp(`${c.name}:${k}`),16));
  const sizes=[]; for(let pos=0;pos<n;) { const size=Math.min(k.startsWith('random-')?1+Math.floor(next()*16):Number(k),n-pos); sizes.push(size);pos+=size; }
  return {sizes};
}
function persisted(c,result) {
  const next=generator(parseInt(fp(c.name+':persisted'),16));
  // Keep shape, vary each declared name deterministically. Applied each chunk.
  return JSON.parse(JSON.stringify(Object.fromEntries(Object.entries(result.stateDeclarations??{}).map(([k,v])=>[k,
    typeof v==='string'?'saved-'+Math.floor(next()*1000):typeof v==='boolean'?!v:typeof v==='number'?v+1+Math.floor(next()*9):Array.isArray(v)?[...v, Math.floor(next()*10)]:v&&typeof v==='object'?{...v,saved:Math.floor(next()*1000)}:Math.floor(next()*1000)]))));
}
function observe(c,result,store,saved,coverage,interaction) {
  if(c.queryComplete===false)result={...result,queryStatements:result.queryStatements.map(q=>({...q,complete:false}))};
  if(!interaction?.skipInitialize)store.initialize(result.stateDeclarations??{},{...saved,...(c.stateValues??{})});
  const errors=[];
  if(!rawSchemas.has(c.schema)) rawSchemas.set(c.schema,JSON.parse(readFileSync(join(C,schemas[c.schema]),'utf8')));
  // No provider or refresh interval: register/read using the pinned manager,
  // without fetching or starting timers. Defaults use the caller's declaration
  // resolver while registering, just as toQueryNode accepts a caller context.
  const manager=createQueryManager(null);let registered=false;
  manager.registerMutations((result.mutationStatements??[]).map(m=>({statementId:m.statementId,toolName:m.statementId})));
  const definitions=rawSchemas.get(c.schema).functions;
  const callCounts=definitions?Object.fromEntries(Object.keys(definitions).map(n=>[n,0])):undefined;
  const ctx={functions:runtimeFunctions(definitions,callCounts),actions:rawSchemas.get(c.schema).actions,getState:n=>{if(coverage)coverage.state=true;return unwrapFieldValue(store.get(n));},resolveRef:n=>{
    const q=result.queryStatements?.find(s=>s.statementId===n);
    const m=result.mutationStatements?.some(s=>s.statementId===n);
    if((q||m)&&Object.prototype.hasOwnProperty.call(c.referenceValues??{},n))return c.referenceValues[n];
    if(q) return registered?manager.getResult(n):q.complete?(q.defaultsAST?evaluate(q.defaultsAST,ctx):null):null;
    return m?manager.getMutationResult(n):null;
  }};
  let root=null,thrown;
  try {
    manager.evaluateQueries((result.queryStatements??[]).map(q=>({statementId:q.statementId,toolName:q.statementId,args:null,defaults:q.complete&&q.defaultsAST?evaluate(q.defaultsAST,ctx):null,complete:q.complete})));
    registered=true;
    if(interaction?.write) {
      const {marker,value}=interaction.write;
      resolveStateField('field',marker,store,ctx,()=>undefined,()=>{throw Error('not reactive')}).setValue(value);
      if(interaction.writeOnly)return callCounts?JSON.stringify({callCounts}):undefined;
    }
    root=evaluateRoot(result,{ctx,library:runtimeLibrary(c),store,errors});
    if(interaction?.capture)interaction.capture.root=root;
  } catch(e) { thrown={name:e instanceof Error?e.name:'Error',message:e instanceof Error?e.message:String(e)}; }
  finally {manager.dispose();}
  if(coverage) {
    const walk=v=>{if(!v||typeof v!=='object')return;
      if(v.hasDynamicProps)coverage.dynamic=true;
      if(v.k==='RuntimeRef')coverage.features.add('34:query');
      if(Array.isArray(v.steps))coverage.features.add('36:actions');
      if(v.k==='Comp') {
        if(v.name==='Each')coverage.features.add('34:each');
        else if(['Action','Set','Reset','Run','ToAssistant','OpenUrl'].includes(v.name))coverage.features.add('36:actions');
        else if(v.mappedProps && !['Count','First','Last','Sum','Avg','Min','Max','Sort','Filter','Round','Abs','Floor','Ceil'].includes(v.name))coverage.features.add(library(c).callDefs?.get(v.name)?.kind==='action'?'36:actions':library(c).callDefs?.get(v.name)?.kind==='function'&&!library(c).callDefs.get(v.name).builtin?'38:functions':v.fn?'38:functions':'34:components');
      }
      if(v.k==='Assign'||v.__reactive)coverage.features.add('35:bindings');
      if(v.k==='Assign')coverage.features.add('35:assign-descriptor');
      Object.values(v).forEach(walk);
    };
    walk(result.root);walk(result.rootExpression);
    if(result.rootExpression)coverage.features.add('34:root');
    if(errors.length)coverage.errors=true;
  }
  return JSON.stringify({root,errors,snapshot:store.getSnapshot(),...(callCounts?{callCounts}:{}),...(thrown?{throw:thrown}:{})});
}
function markers(root) {
  const out=[];
  function walk(v,path=[]) {
    if(!v||typeof v!=='object')return;
    if(v.__reactive==='assign'){out.push({path,marker:v});return;}
    for(const [k,child]of Object.entries(v))walk(child,[...path,k]);
  }
  walk(root);return out;
}
function writes(c,saved) {
  const store=createStore(),result=parse(input(c),library(c),c.root);
  const initial=JSON.parse(observe(c,result,store,saved));
  const next=generator(parseInt(fp(c.name+':writes'),16)),steps=[];
  // Each discovered marker receives every JSON kind, in a seeded order and
  // in one continuing store. Keep the original marker across redraws.
  for(const {path,marker}of markers(initial.root)) {
    const values=[null,false,true,Math.floor(next()*21)-10,'write-'+Math.floor(next()*1000),[],[1,'x',null],{}, {n:2,active:false}];
    for(let i=values.length-1;i>0;i--){const j=Math.floor(next()*(i+1));[values[i],values[j]]=[values[j],values[i]];}
    for(const value of values) {
      const failure=observe(c,result,store,saved,null,{skipInitialize:true,writeOnly:true,write:{marker,value}});
      // A post-write draw is a new observation: register defaults from the
      // updated store, matching this adapter's existing fresh-manager policy.
      const observation=JSON.parse(observe(c,result,store,saved,null,{skipInitialize:true}));
      if(failure)observation.writeError=JSON.parse(failure).throw;
      steps.push({path,marker,value,observation,...(failure&&JSON.parse(failure).callCounts?{writeCallCounts:JSON.parse(failure).callCounts}:{})});
    }
  }
  return steps;
}
function run(c,k,coverage) {
  const full=parse(input(c),library(c),c.root), saved=persisted(c,full), stores=[createStore(),createStore()], results=[[],[]];
  const add=r=>{for(let i=0;i<2;i++)results[i].push(observe(c,r,stores[i],i?saved:{},coverage));};
  let p={};
  if(k==='oneshot')add(full);
  else {
    p=plan(c,k);const parser=createStreamParser(library(c),c.root), cps=Array.from(input(c));
    if(p.ops) {
      const other=[...cps],m=Math.floor(cps.length/2);if(other.length)other[m]=other[m]==='Z'?'Y':'Z';
      for(const [kind,n]of p.ops)add(kind==='e'?parser.getResult(true):kind==='r'?parser.getResult():parser.set((kind==='p'?cps:other).slice(0,n).join('')));
    } else {
      let pos=0;for(const size of p.sizes){add(parser.push(cps.slice(pos,pos+size).join('')));pos+=size;}add(parser.getResult(true));
    }
  }
  return {...p,defaults:results[0],persisted:results[1],saved};
}
function actionCounts(){return {cases:0,scenarios:0,steps:0,plans:0,originalCases:0,kinds:Object.create(null),events:Object.create(null)};}
function countActions(c,actions,counts){
 if(!actions)return;
 counts.cases++;if(!c.name.startsWith('runtime/'))counts.originalCases++;
 for(const name of ['defaults','persisted'])if(actions[name]){
  counts.scenarios++;counts.plans+=actions[name].handles.length;counts.steps+=actions[name].steps.length;
  for(const {op,observation} of actions[name].steps){counts.kinds[op.kind]=(counts.kinds[op.kind]??0)+1;
   const events=actions.values[observation.events.$value];for(const event of events)counts.events[event.type]=(counts.events[event.type]??0)+1;
  }
 }
}
function write(name,value){const p=join(G,name);mkdirSync(dirname(p),{recursive:true});writeFileSync(p,JSON.stringify(value)+'\n');}
const [cmd,...args]=process.argv.slice(2);
async function fuzzCase(c) {
  if(Buffer.byteLength(JSON.stringify(c))>65536||Buffer.byteLength(c.fuzzInput)>4096||(c.actionInteractions?.length??0)>64||(c.actionInteractions??[]).some(op=>typeof op.program==='string'&&Buffer.byteLength(op.program)>4096))throw Error('runtime fuzz case bounds');
  const full=parse(input(c),library(c),c.root),saved=persisted(c,full);
  const initial={defaults:JSON.parse(observe(c,full,createStore(),{})),persisted:JSON.parse(observe(c,full,createStore(),saved))};
  const actions=await actionScenarios(c,saved,{observe,parse,library,input,generator,fp,rawSchemas,runtimeLibrary,createStore,createQueryManager,evaluate,evaluateRoot,resolveStateField,runtimeFunctions,rendererState,unwrapFieldValue,persistDefaultValue});
  // Keep the oracle's exact serialized bytes inside strings. The outer
  // protocol may normalize controller numbers, but cannot rewrite these.
  const serialized=o=>({tree:JSON.stringify({root:o.root,errors:o.errors}),snapshot:JSON.stringify(o.snapshot),events:JSON.stringify(o.events??[])});
  for(const observation of Object.values(initial))observation.serialized=serialized(observation);
  for(const scenario of ['defaults','persisted'])for(const step of actions?.[scenario]?.steps??[]) {
    const expanded=Object.fromEntries(Object.entries(step.observation).map(([k,v])=>[k,v&&typeof v==='object'&&Object.hasOwn(v,'$value')?actions.values[v.$value]:v]));
    step.observation.serialized=serialized(expanded);
  }
  return {initial,saved,actions};
}
if(cmd==='fuzz') {
  // The existing golden adapter remains the oracle, including real retained
  // step identities and async mutation suspension. No packages are installed.
  const {createInterface}=await import('node:readline');
  for await(const line of createInterface({input:process.stdin})) {
    try {
      if(Buffer.byteLength(line)>65536)throw Error('runtime fuzz input exceeds 64 KiB');
      process.stdout.write(JSON.stringify(await fuzzCase(JSON.parse(line)))+'\n');
    } catch(e) {process.stdout.write(JSON.stringify({workerError:String(e)})+'\n');}
  }
} else if(cmd==='fuzz-golden') {
  const pinned=JSON.parse(readFileSync(join(C,'runtime-fuzz.json'),'utf8')).cases;
  const output=[];
  for(const c of pinned)output.push({name:c.name,...await fuzzCase(c)});
  writeFileSync(join(C,'golden/runtime-fuzz.json'),JSON.stringify(output)+'\n');
} else if(cmd==='golden') {
  const actionStats=actionCounts();
  const stats={total:cases.length,dynamic:0,state:0,references:0,errors:0,recorded:0,recordedState:0,steps:0,scenarioDifferences:0,treeDifferences:0,features:{},original:{dynamic:0,state:0,references:0,errors:0}};
  const evidence={};
  for(const c of cases.filter(c=>!process.env.RUNTIME_CASE||c.name.includes(process.env.RUNTIME_CASE))) {
    const coverage={dynamic:false,state:false,errors:false,features:new Set()};
    const statements=split(tokenize(autoClose(preprocess(input(c))).text));
    const names=new Set(statements.filter(s=>!s.id.startsWith('$')).map(s=>s.id));
    let refs=false;
    const visit=(v,bound=new Set())=>{
      if(!v||typeof v!=='object')return;
      if(v.k==='Ref'&&names.has(v.n)&&!bound.has(v.n))refs=true;
      if(v.k==='Comp'&&v.name==='Each'){
        visit(v.args[0],bound);
        const name=v.args[1]?.n??v.args[1]?.v;
        visit(v.args[2],new Set([...bound,name]));return;
      }
      Object.values(v).forEach(child=>visit(child,bound));
    };
    for(const s of statements)visit(parseExpression(s.tokens));
    const one=run(c,'oneshot',coverage);
    write(c.name+'.oneshot.json',{saved:one.saved,defaults:JSON.parse(one.defaults[0]),persisted:JSON.parse(one.persisted[0])});
    const fieldWrites={defaults:writes(c,{}),persisted:writes(c,one.saved)};
    if(fieldWrites.defaults.length||fieldWrites.persisted.length)write(c.name+'.writes.json',fieldWrites);
    else rmSync(join(G,c.name+'.writes.json'),{force:true});
    const actions=await actionScenarios(c,one.saved,{observe,parse,library,input,generator,fp,rawSchemas,runtimeLibrary,createStore,createQueryManager,evaluate,evaluateRoot,resolveStateField,runtimeFunctions,rendererState,unwrapFieldValue,persistDefaultValue});
    countActions(c,actions,actionStats);
    if(actions)write(c.name+'.actions.json',actions);
    else rmSync(join(G,c.name+'.actions.json'),{force:true});
    const streams={};
    for(const k of [...(c.chunkings??chunkings),...(c.recorded?['recorded']:[])]) {
      const r=run(c,k,coverage); stats.steps+=r.defaults.length;
      stats.scenarioDifferences+=r.defaults.filter((v,i)=>v!==r.persisted[i]).length;
      stats.treeDifferences+=r.defaults.filter((v,i)=>JSON.stringify(JSON.parse(v).root)!==JSON.stringify(JSON.parse(r.persisted[i]).root)).length;
      const core=existsSync(join(C,'golden',c.name+'.steps.json'));
      streams[k]={...(!core&&r.sizes&&(k.startsWith('random-')||k==='recorded')?{sizes:r.sizes}:{}),...(!core&&r.ops?{ops:r.ops}:{}),defaults:runs(r.defaults.map(fp)),persisted:r.defaults.every((v,i)=>v===r.persisted[i])?'defaults':runs(r.persisted.map(fp))};
    }
    write(c.name+'.steps.json',streams);
    for(const k of ['dynamic','state','errors'])if(coverage[k])stats[k]++;
    if(refs)stats.references++;
    if(!c.name.startsWith('runtime/')){for(const k of ['dynamic','state','errors'])if(coverage[k])stats.original[k]++;if(refs)stats.original.references++;}
    if(c.source==='recorded'){stats.recorded++;if(Object.keys(one.saved).length)stats.recordedState++;}
    const features=[...coverage.features].sort();for(const f of features)stats.features[f]=(stats.features[f]??0)+1;
    evidence[c.name]={features,dynamic:coverage.dynamic,state:coverage.state,errors:coverage.errors,references:refs};
  }
  const bindingPairs=Object.fromEntries(Object.keys(schemas).map(schema=>[schema,Object.entries(runtimeLibrary({schema}).components).flatMap(([name,d])=>Object.entries(d.props.shape).filter(([k,v])=>isReactiveSchema(v)).map(([key])=>[name,key]))]));
  write('bindings.json',bindingPairs);
  write('binding-undecided-spec.json',{spec:diagnosticLibrary.toSpec(),bindings:Object.entries(diagnosticProps).flatMap(([name,props])=>Object.entries(props.shape).filter(([,schema])=>isReactiveSchema(schema)).map(([prop])=>[name,prop]))});
  if(!process.env.RUNTIME_CASE){write('coverage.json',stats);write('evidence.json',evidence);write('action-coverage.json',actionStats);}console.log(JSON.stringify(stats));
} else if(cmd==='actions') {
  const counts=actionCounts();
  for(const c of cases.filter(c=>!process.env.RUNTIME_CASE||c.name.includes(process.env.RUNTIME_CASE))) {
    const one=JSON.parse(readFileSync(join(G,c.name+'.oneshot.json'),'utf8'));
    // observe caches the raw schema, just as full regeneration does.
    observe(c,parse(input(c),library(c),c.root),createStore(),{});
    const actions=await actionScenarios(c,one.saved,{observe,parse,library,input,generator,fp,rawSchemas,runtimeLibrary,createStore,createQueryManager,evaluate,evaluateRoot,resolveStateField,runtimeFunctions,rendererState,unwrapFieldValue,persistDefaultValue});
    countActions(c,actions,counts);
    if(actions)write(c.name+'.actions.json',actions); else rmSync(join(G,c.name+'.actions.json'),{force:true});
  }
  if(!process.env.RUNTIME_CASE)write('action-coverage.json',counts);
  console.log(JSON.stringify(counts));
} else if(cmd==='show') {
  const [name,scenario,k,step]=args,c=cases.find(c=>c.name===name);if(!c)throw Error(name);
  process.stdout.write(run(c,k)[scenario][Number(step)-1]);
} else throw Error('golden | actions | show <case> <defaults|persisted> <chunking> <step>');
