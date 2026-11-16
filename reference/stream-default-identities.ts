// Run from reference/: bun --no-install stream-default-identities.ts
import { createStreamingParser } from './lang-core/src/parser';
import { createStore } from './lang-core/src/runtime/store';
import { readFileSync } from 'fs';
const schema = JSON.parse(readFileSync('../conformance/schemas/runtime.json','utf8'));
const program = 'root=Box("state")\n$a=[1,2]\n$o={x:1}\n$e=@Each([1,2],i,i+1)\n$n=4\n$read=$n*2\n';
const outputs = [];
for (const edited of ['none','a-before','a-after','o-before','o-after','e-before','e-after']) {
  const name = edited==='none' ? null : '$'+edited[0];
  const setAt = name===null ? null : edited.endsWith('-after') ? program.indexOf('$n=4') : program.indexOf(name+'=')+(name==='$e'?12:5);
  const setValue = name==='$o' ? {user:true} : ['user'];
  const parser = createStreamingParser(schema), store = createStore();
  let notifications = 0;
  store.subscribe(()=>notifications++);
  let previous: Record<string,unknown> = {};
  const steps=[];
  for(let i=0;i<program.length;i++) {
    const result=parser.push(program[i]!);
    const defaults=result.stateDeclarations;
    const same=Object.keys(defaults).filter(k=>Object.hasOwn(previous,k)&&Object.is(defaults[k],previous[k]));
    const before=notifications;
    store.initialize(defaults,{});
    steps.push({same,notified:notifications-before,snapshot:JSON.parse(JSON.stringify(store.getSnapshot()))});
    if (i===setAt && name!==null) store.set(name,setValue);
    previous=defaults;
  }
  outputs.push({edited,setKey:name,setAt,setValue,steps});
}
console.log(JSON.stringify({program,outputs}));
