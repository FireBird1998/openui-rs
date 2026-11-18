// Run from reference/: bun --no-install query-default-cycles.ts
// Same observation shape and eager registration order as runtime.ts.
import {writeFileSync} from 'fs';
process.env.OPENUI_TELEMETRY_DISABLED='1';process.env.DO_NOT_TRACK='1';
const {compileSchema,parse}=await import('./lang-core/src/parser/parser');
const {evaluate}=await import('./lang-core/src/runtime/evaluator');
const {evaluateRoot}=await import('./lang-core/src/runtime/evaluate-tree');
const {createQueryManager}=await import('./lang-core/src/runtime/queryManager');
const {createStore}=await import('./lang-core/src/runtime/store');
const library=compileSchema({$defs:{Box:{properties:{a:{}}}}});
const results={};
for (const [reason,decl,read] of [
 ['self','q=Query("t",{},q)',true],
 ['pair','q=Query("t",{},r)\nr=Query("t",{},q)',true],
 ['long','q=Query("t",{},r)\nr=Query("t",{},s)\ns=Query("t",{},q)',true],
 ['unread','q=Query("t",{},r)\nr=Query("t",{},q)',false],
] as const) {
 const program=decl+'\nroot=Box('+(read?'q':'1')+')';
 const p=parse(program,library);
 const observe=()=>{
  const store=createStore(),manager=createQueryManager(null);let registered=false;
  const errors=[];let root=null,thrown;
  let resolutions=0;
  const ctx={getState:n=>store.get(n),resolveRef:n=>{
   const q=p.queryStatements?.find(q=>q.statementId===n);
   // Keep this resolver frame: JavaScriptCore's proper tail calls would
   // otherwise turn a cycle into a non-terminating tail-call loop.
   try { return q?(registered?manager.getResult(n):q.defaultsAST?evaluate(q.defaultsAST,ctx):null):null; }
   finally { resolutions++; }
  }};
  try {
   manager.evaluateQueries((p.queryStatements??[]).map(q=>({statementId:q.statementId,toolName:q.statementId,args:null,defaults:q.defaultsAST?evaluate(q.defaultsAST,ctx):null,complete:q.complete})));
   registered=true;
   root=evaluateRoot(p,{ctx,library:{components:{}},store,errors});
  } catch(e) {thrown={name:e.name,message:e.message};} finally {manager.dispose();}
  return {root,errors,snapshot:store.getSnapshot(),...(thrown?{throw:thrown}:{})};
 };
 results[reason]={program,reference:{defaults:observe(),persisted:observe()}};
}
writeFileSync('../conformance/golden/query-default-cycles.json',JSON.stringify(results)+'\n');
