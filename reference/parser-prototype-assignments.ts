// Run from reference/: bun --no-install parser-prototype-assignments.ts
import {readFileSync, writeFileSync} from 'fs';
import {join} from 'path';
process.env.OPENUI_TELEMETRY_DISABLED = '1';
process.env.DO_NOT_TRACK = '1';
const {compileSchema, createStreamParser, parse} = await import('./lang-core/src/parser/parser');
const base=join(import.meta.dir,'..','conformance');
const corpus=JSON.parse(readFileSync(join(base,'parser-prototype-assignments.json'),'utf8'));
function fingerprint(value: unknown) {
 let h=0x811c9dc5;
 for(const b of new TextEncoder().encode(JSON.stringify(value))) h=Math.imul(h^b,0x01000193)>>>0;
 return h.toString(16).padStart(8,'0');
}
const results={};
for(const c of corpus.cases) {
 const library=compileSchema(c.schema);
 const streams={};
 for(const n of corpus.chunkings) {
  const parser=createStreamParser(library);
  const chars=Array.from(c.program) as string[];
  const steps=[];
  for(let i=0;i<chars.length;i+=n) steps.push(fingerprint(parser.push(chars.slice(i,i+n).join(''))));
  steps.push(fingerprint(parser.getResult(true)));
  streams[n]=steps;
 }
 results[c.name]={oneshot:parse(c.program,library),streams};
}
writeFileSync(join(base,'golden','parser-prototype-assignments.json'),JSON.stringify(results)+'\n');
