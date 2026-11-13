// Runs the pinned parser, store and tree evaluator; no runtime transcription.
// From reference/: bun --no-install runtime.ts golden
// Diagnose: bun --no-install runtime.ts show <case> <scenario> <chunking> <step>
import { readFileSync, writeFileSync, mkdirSync, existsSync } from 'fs';
import { join, dirname } from 'path';
import { plugin } from 'bun';
// Same read-only export hook as run.ts; vendored parser stays untouched.
plugin({name:'runtime-parser-stages',setup(build){build.onLoad({filter:/lang-core\/src\/parser\/parser\.ts$/},({path})=>({contents:readFileSync(path,'utf8')+'\nexport { preprocess };\n',loader:'ts'}));}});
process.env.OPENUI_TELEMETRY_DISABLED = '1';
process.env.DO_NOT_TRACK = '1';
const { compileSchema, parse, createStreamParser, preprocess } = await import('./lang-core/src/parser/parser');
const { evaluateRoot } = await import('./lang-core/src/runtime/evaluate-tree');
const { evaluate } = await import('./lang-core/src/runtime/evaluator');
const { createStore } = await import('./lang-core/src/runtime/store');
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
const runtimeLibrary=c=>{
  if(!runtimeLibraries.has(c.schema))runtimeLibraries.set(c.schema,{components:Object.fromEntries([...library(c)].map(([n,d])=>[n,{props:{shape:Object.fromEntries(d.params.map(p=>[p.name,p.schema]))}}]))});
  return runtimeLibraries.get(c.schema);
};
const input = c => readFileSync(join(C,c.file),'utf8');
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
function observe(c,result,store,saved,coverage) {
  store.initialize(result.stateDeclarations??{},saved);
  const errors=[];
  if(!rawSchemas.has(c.schema)) rawSchemas.set(c.schema,JSON.parse(readFileSync(join(C,schemas[c.schema]),'utf8')));
  const ctx={actions:rawSchemas.get(c.schema).actions,getState:n=>{if(coverage)coverage.state=true;return store.get(n);},resolveRef:n=>{
    const q=result.queryStatements?.find(s=>s.statementId===n);
    if(q) return q.defaultsAST?evaluate(q.defaultsAST,ctx):null;
    return result.mutationStatements?.some(s=>s.statementId===n)?{status:'idle',data:null,error:null}:null;
  }};
  const root=evaluateRoot(result,{ctx,library:runtimeLibrary(c),store,errors});
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
  return JSON.stringify({root,errors,snapshot:store.getSnapshot()});
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
function write(name,value){const p=join(G,name);mkdirSync(dirname(p),{recursive:true});writeFileSync(p,JSON.stringify(value)+'\n');}
const [cmd,...args]=process.argv.slice(2);
if(cmd==='golden') {
  const stats={total:cases.length,dynamic:0,state:0,references:0,errors:0,recorded:0,recordedState:0,steps:0,scenarioDifferences:0,treeDifferences:0,features:{},original:{dynamic:0,state:0,references:0,errors:0}};
  const evidence={};
  for(const c of cases) {
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
  write('coverage.json',stats);write('evidence.json',evidence);console.log(JSON.stringify(stats));
} else if(cmd==='show') {
  const [name,scenario,k,step]=args,c=cases.find(c=>c.name===name);if(!c)throw Error(name);
  process.stdout.write(run(c,k)[scenario][Number(step)-1]);
} else throw Error('golden | show <case> <defaults|persisted> <chunking> <step>');
