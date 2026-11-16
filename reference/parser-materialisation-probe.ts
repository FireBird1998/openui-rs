// Run from reference/: bun --no-install parser-materialisation-probe.ts
process.env.OPENUI_TELEMETRY_DISABLED = '1';
process.env.DO_NOT_TRACK = '1';
const {compileSchema, parse} = await import('./lang-core/src/parser/parser');
const {evaluateRoot} = await import('./lang-core/src/runtime/evaluate-tree');
const box = {$defs:{Box:{properties:{a:{}}}}};
const p = parse('root=Box({k:"Str",v:"y",z:2})', compileSchema(box));
const errors=[];
const root=evaluateRoot(p,{ctx:{getState:()=>undefined,resolveRef:()=>null},library:{components:{}},errors});
console.log(JSON.stringify({name:'descriptor-extra-keys',parse:p,evaluated:root,errors}));
for (const kind of ['functions','actions','components']) {
 const schema=structuredClone(box);
 if(kind==='components')schema.$defs.Each={properties:{x:{}}};
 else schema[kind]={Each:{params:{properties:{x:{}}}}};
 for(const source of ['root=Box(@Each(0,x,0))','root=Box(true?@Each(0,x,x):null)','root=Box(Each(0,x,x))'])
  console.log(JSON.stringify({kind,source,result:parse(source,compileSchema(schema))}));
}
for (const name of Object.getOwnPropertyNames(Object.prototype)) {
 for (const mode of ['property','required-property','required-only']) {
  const sub={type:'object',...(mode==='required-only'?{}:{properties:{[name]:{type:'string'}}}),...(mode==='property'?{}:{required:[name]})};
  const schema={$defs:{Box:{properties:{a:sub}}}};
  console.log(JSON.stringify({name,mode,result:parse('root=Box({})',compileSchema(schema))}));
 }
}
{
 const schema=JSON.parse('{"$defs":{"Box":{"properties":{"a":{"type":"object","properties":{"__proto__":{"type":"object","required":["__proto__"],"properties":{"__proto__":{"type":"string"}}}}}}}}}');
 console.log(JSON.stringify({name:'prototype-null-parent',result:parse('root=Box({})',compileSchema(schema))}));
}
for (const name of Object.getOwnPropertyNames(Object.prototype)) {
 const sub={type:'object',required:[name],properties:{[name]:{type:'string'}}};
 const schema={$defs:{Box:{properties:{a:sub}}}};
 console.log(JSON.stringify({name,mode:'null-prototype-required',result:parse('root=Box({__proto__:null})',compileSchema(schema))}));
}
console.log(JSON.stringify({name:'null-prototype-then-own',result:parse('root=Box({__proto__:null,__proto__:"own"})',compileSchema(box))}));
{
 const value={type:'object',required:['constructor'],properties:{constructor:{type:'string'}}};
 const schema={...box,functions:{Probe:{params:{properties:{value}}}}};
 console.log(JSON.stringify({name:'literal-call-null-prototype',result:parse('root=Box(@Probe({__proto__:null}))',compileSchema(schema))}));
}
{
 const schema={$defs:{Box:{properties:{a:{type:'object',properties:{x:{type:'number',default:1}}}}}}};
 const result=parse('root=Box({__proto__:null,x:"bad"})',compileSchema(schema));
 console.log(JSON.stringify({name:'validation-spread-inheritance',result,constructorPresent:'constructor' in result.root.props.a,constructorOwn:Object.hasOwn(result.root.props.a,'constructor')}));
}
