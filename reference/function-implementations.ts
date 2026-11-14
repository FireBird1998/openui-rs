// Pure functions over JSON; recipes are shared with tests/support/functions.rs.
import recipes from '../conformance/function-implementations.json';
export function implementation(name: string) {
 const r=recipes[name];
 if(!r)throw Error('No corpus implementation for '+name);
 return args=>{
  switch(r.op) {
   case 'args':return args;
   case 'get':return args[r.key];
   case 'constant':return structuredClone(r.value);
   case 'undefined':return undefined;
   case 'throw':throw Error(r.value);
   case 'repeat':return 'x'.repeat(r.value);
   case 'deep':{let v=null;for(let i=0;i<r.value;i++)v=[v];return v;}
   case 'twice':return Number(args[r.key])*2;
   case 'percent':return ((args.part/args.total)*100).toFixed(1)+'%';
   case 'sum':{const xs=args.numbers??[];if(!Array.isArray(xs))throw Error('numbers must be an array');return String(args.label??'')+xs.reduce((a,b)=>a+Number(b),0);}
   case 'upper':return String(args.s??args.text??'').toUpperCase();
   case 'format':return (args.label??'')+String(args.value??'');
   default:throw Error('Unknown implementation '+r.op);
  }
 };
}
export function runtimeFunctions(definitions, counts) {
 return Object.fromEntries(Object.entries(definitions??{}).map(([name,def])=>[name,{...def,fn:args=>{if(counts)counts[name]++;return implementation(name)(args);}}]));
}
