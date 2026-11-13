// Runs the pinned store itself; no store implementation is transcribed.
// From reference/: bun --no-install store-interface.ts > ../crates/openui-runtime/tests/store.json
// The renderer is not vendored. Persisted state on first initialize versus every
// initialize is an assumption tested in both forms, not a renderer claim.
import { readFileSync } from "fs";
import { createStore } from "./lang-core/src/runtime/store";
import { createStreamingParser } from "./lang-core/src/parser";
import { parseExpression } from "./lang-core/src/parser/expressions";
import { tokenize } from "./lang-core/src/parser/lexer";
import { evaluate } from "./lang-core/src/runtime/evaluator";

type Entry = [string, number];
type Step = { op: string; name?: string; value?: number; pristine?: boolean;
  defaults?: Entry[]; persisted?: Entry[]; chunk?: string; source?: string; duplicateLiteral?: "defaults" | "persisted"; final?: boolean };

function sequence(name: string, make: (add: (v: unknown, expression?: string) => number, steps: Step[]) => void, about = {}) {
  const values: unknown[] = [], pool: any[] = [];
  const identities = new Map<unknown, number>(), scalarIds = new Map<string, number>();
  function add(v: any, expression?: string): number {
    const object = v !== null && typeof v === "object";
    const key = object ? "" : typeof v + ":" + (Object.is(v, -0) ? "-0" : String(v));
    const previous = object ? identities.get(v) : scalarIds.get(key);
    if (previous !== undefined) return previous;
    const id = values.length;
    values.push(v); pool.push(null);
    if (object) identities.set(v, id); else scalarIds.set(key, id);
    pool[id] = v === undefined ? ["undefined"] : v === null ? ["null"]
      : typeof v === "number" ? ["number", Object.is(v, -0) ? "-0" : String(v)]
      : typeof v === "string" ? ["string", v] : typeof v === "boolean" ? ["boolean", v]
      : Array.isArray(v) ? ["array", v.map(v => add(v))]
      : ["object", Object.entries(v).map(([k, v]) => [k, add(v)])];
    if (expression !== undefined) pool[id].push("ast", expression);
    return id;
  }
  const steps: Step[] = [];
  make(add, steps);
  const entries = (e: Entry[] = []) => Object.fromEntries(e.map(([k, i]) => [k, values[i]]));
  function apply(store: ReturnType<typeof createStore>, s: Step) {
    switch (s.op) {
      case "set": store.set(s.name!, values[s.value!], { pristine: s.pristine }); break;
      case "read": store.get(s.name!); break;
      case "read-set": store.set(s.name!, store.get(s.name!), { pristine: s.pristine }); break;
      case "initialize": {
        const literal = {$a:1, $a:2, $z:0, $z:undefined};
        store.initialize(s.duplicateLiteral === "defaults" ? literal : entries(s.defaults),
          s.duplicateLiteral === "persisted" ? literal : entries(s.persisted)); break;
      }
      case "dispose": store.dispose(); break;
      default: throw new Error(s.op);
    }
  }
  const snapshots: Entry[][] = [], snapshotIds = new Map<string,number>();
  function snapshotId(entries: Entry[]) {
    const key = JSON.stringify(entries), previous = snapshotIds.get(key);
    if (previous !== undefined) return previous;
    const id = snapshots.length; snapshots.push(entries); snapshotIds.set(key,id); return id;
  }
  const store = createStore();
  let notifications = 0, mappedVersion = 0;
  const subscribe = () => store.subscribe(() => notifications++);
  subscribe();
  const recorded = steps.map((s, i) => {
    const before = store.getSnapshot(), previousNotifications = notifications;
    apply(store, s);
    const after = store.getSnapshot();
    const notified = notifications - previousNotifications;
    const changed = s.op === "dispose" ? Object.keys(before).length !== 0 : notified !== 0;
    if (changed) mappedVersion++;
    if (s.op === "dispose") subscribe(); // dispose clears the reference listeners
    // Upstream has no pristine read: infer present names through its public API.
    const probe = createStore();
    for (const step of steps.slice(0, i + 1)) apply(probe, step);
    const sentinel = Symbol("default probe");
    probe.initialize(Object.fromEntries(Object.keys(after).map(k => [k, sentinel])), {});
    return { ...s, expected: { changed, notified, snapshotChanged: before !== after, mappedVersion,
      snapshot: snapshotId(Object.entries(after).map(([k, v]) => [k, add(v)])),
      pristine: Object.keys(after).filter(k => probe.get(k) === sentinel),
      read: add(store.get(s.name ?? "$missing")) } };
  });
  return { name, ...about, pool, snapshots, steps: recorded };
}

const cases = [sequence("directed", (add, s) => {
  const put = (name: string, v: unknown, pristine = false) => s.push({op:"set", name, value:add(v), pristine});
  const init = (defaults: Record<string, unknown>, persisted = {}) => s.push({op:"initialize",
    defaults:Object.entries(defaults).map(([k,v]) => [k,add(v)]),
    persisted:Object.entries(persisted).map(([k,v]) => [k,add(v)])});
  put("$absent", undefined, true);
  for (const v of [undefined, null, false, true, "", "hello\u0000😀", NaN, -0, 0, Infinity, -Infinity]) {
    put("$x", v); put("$x", v); s.push({op:"read",name:"$x"});
  }
  const child = [1];
  for (const v of [[], [], child, child, {a:1,b:null}, {b:null,a:1}, {a:child}, {a:child}, {a:[1]}, {b:1}, {}]) put("$x",v);
  s.push({op:"read-set",name:"$x"});
  init({$a:"partial",$gone:1}); init({$a:"complete",$b:2});
  put("$a","complete"); init({$a:"new"});
  put("$a","complete",true); init({$a:"new"});
  init({$a:"default"},{$a:"new",$extra:undefined}); init({$a:"later"});
  const object = {n:1}; init({$o:object}); init({$o:{n:1}}); init({},{$o:{n:1}});
  init({$undefined:undefined}); s.push({op:"dispose"},{op:"dispose"}); init({$a:3});
  for (const name of ["__proto__","constructor","10","2",""]) put(name,1);
})];

for (const seed of [1, 7, 32, 0xdeadbeef]) cases.push(sequence(`seed-${seed}`, (add, steps) => {
  let random = seed >>> 0;
  const next = (n: number) => { random ^= random << 13; random ^= random >>> 17; random ^= random << 5; return (random >>> 0) % n; };
  const names = ["$a","form","__proto__","constructor","2","10","", "$é", ...Array.from({length:13},(_,i)=>`$s${i}_${next(100)}`)];
  const shared: any[] = [{n:1}, [], [1]];
  const scalar = (nested: boolean): any => {
    switch(next(nested ? 9 : 10)) {
      case 0: return null; case 1: return next(2) === 0; case 2: return -0;
      case 3: return NaN; case 4: return Infinity; case 5: return -Infinity;
      case 6: return (next(20001)-10000)/10; case 7: return ["", "é\ud800", "😀\u0000", "quoted\"\\"][next(4)]!;
      case 8: return `value-${next(1000000)}-${"x".repeat(next(40))}`;
      default: return undefined;
    }
  };
  const value = (depth=0): any => {
    if (next(5) === 0) return shared[next(shared.length)];
    if (depth >= 2 || next(3) === 0) return scalar(depth !== 0);
    const v = next(2) === 0 ? Array.from({length:next(6)},()=>value(depth+1))
      : Object.fromEntries(Array.from({length:next(5)},()=>[names[next(names.length)]!,value(depth+1)]));
    shared.push(v); return v;
  };
  const map = (): Entry[] => names.filter(() => next(7) === 0).map(k => [k,add(value())]);
  for (let i=0;i<80;i++) {
    const name = names[next(names.length)]!;
    switch(next(12)) {
      case 0: steps.push({op:"dispose"}); break;
      case 1: case 2: case 3: steps.push({op:"initialize",defaults:map(),persisted:map()}); break;
      case 4: steps.push({op:"read",name}); break;
      case 5: steps.push({op:"read-set",name,pristine:next(2) === 0}); break;
      default: {
        const v = value(), step = {op:"set",name,value:add(v),pristine:next(3) === 0};
        steps.push(step);
        if (next(3) === 0) steps.push({...step});
        if (v && typeof v === "object" && !Array.isArray(v)) steps.push({...step,value:add({...v})});
      }
    }
  }
}));

cases.push(sequence("typed-asts", (add,s) => {
  for (const expression of ['1','1','2','null','null','"hi"','"hi"','true','true','[]','[]','{a:1}','{a:1}',
    '$x','$x','ref','ref','$x + 1','$x + 1','!$x','$x ? 1 : 2','obj.x','obj[0]','$x = 2','@Sum([1])']) {
    s.push({op:"set",name:"$ast",value:add(parseExpression(tokenize(expression)),expression)});
    s.push({op:"read-set",name:"$ast"});
  }
  s.push({op:"set",name:"$ast",value:add({k:"Num",v:1})});
  s.push({op:"set",name:"$ast",value:add(parseExpression(tokenize('1')),'1')});
}));

// Repeated keys are passed as actual object literals to the reference, rather
// than simulating intermediate writes. Rust replays the uncollapsed entries.
cases.push(sequence("duplicate-initialize", (add,s) => {
  const one=add(1), two=add(2), zero=add(0), undef=add(undefined);
  const defaults: Entry[] = [["$a",one],["$a",two],["$z",zero],["$z",undef]];
  s.push({op:"initialize",defaults,persisted:[],duplicateLiteral:"defaults"},{op:"initialize",defaults,persisted:[],duplicateLiteral:"defaults"});
  const persisted: Entry[] = [["$a",one],["$a",two],["$z",zero],["$z",undef]];
  s.push({op:"initialize",defaults:[],persisted,duplicateLiteral:"persisted"},{op:"initialize",defaults:[],persisted,duplicateLiteral:"persisted"});
  const first={}, last={x:[1]};
  const objects: Entry[] = [["$object",add(first)],["$object",add(last)]];
  s.push({op:"initialize",defaults:objects,persisted:[]},{op:"initialize",defaults:objects,persisted:[]});
}));

const syntheticSchemaFile = "schemas/text.json";
const syntheticSchema = JSON.parse(readFileSync(new URL(`../conformance/${syntheticSchemaFile}`,import.meta.url),"utf8"));
const programs = {
  array:'root = Text("state")\n$title = "Hello streaming"\n$edit = "default"\n$base = 7\n$twice = $base * 2\n$items = [1,2,3]\n',
  object:'root = Text("state")\n$edit = "default"\n$config = {a:1,b:"ready"}\n',
  rewind:'root = Text("rewind")\n$count = 12\n$rows = ["alpha",{n:2}]\n$gone = {x:"complete"}\n',
};
for (const [kind,program] of Object.entries(programs)) for (const mode of ["none","first","every"]) {
  cases.push(sequence(`synthetic-${kind}-${mode}`, (add,s) => {
    const parser = createStreamingParser(syntheticSchema);
    const initial: Entry[] = [["$title",add("saved")]];
    const persisted = (i:number) => mode === "every" || (mode === "first" && i===0) ? initial : [];
    let edited = false;
    // ASCII: each character here is exactly one byte. Initialize after every one.
    for (let i=0;i<program.length;i++) {
      const source = program.slice(0,i+1), final = i===program.length-1;
      const parsed = parser.set(source,final);
      s.push({op:"initialize",chunk:program[i],final,defaults:Object.entries(parsed.stateDeclarations).map(([k,v])=>[k,add(v)]),persisted:persisted(i)});
      if (kind === "array" && source.endsWith('$edit = "def') && !edited) {
        s.push({op:"set",name:"$edit",value:add("user")}); edited=true;
      }
      if (kind === "object" && source.endsWith('$edit = "def') && !edited) {
        s.push({op:"set",name:"$edit",value:add("user")}); edited=true;
      }
      if (kind === "rewind" && source.endsWith('$rows = ["alpha"') && !edited) {
        s.push({op:"set",name:"$rows",value:add(["user"])}); edited=true;
      }
    }
    if (kind === "rewind") {
      for (const source of [program.slice(0,program.indexOf('$rows')),program]) {
        const parsed = parser.set(source,true);
        s.push({op:"initialize",source,final:true,defaults:Object.entries(parsed.stateDeclarations).map(([k,v])=>[k,add(v)]),persisted:mode==="every"?initial:[]});
      }
    }
  },{synthetic:kind,schema:syntheticSchemaFile,persistedMode:mode}));
}

const recordings = [
  "batch-104-2026-11-11T105557-a-museum-collection-catalog-with-artifact.recording.json",
  "batch-22-2026-11-11T103431-a-sign-up-form-with-name.recording.json",
  "batch-19-2026-11-11T103421-local-events-this-week-as-a.recording.json",
];
const schemaFile = "imported/schemas/react-ui.json";
const schema = JSON.parse(readFileSync(new URL(`../conformance/${schemaFile}`, import.meta.url), "utf8"));
for (const recording of recordings) for (const mode of ["none", "first", "every"]) {
  const data = JSON.parse(readFileSync(new URL(`../conformance/recordings/${recording}`, import.meta.url), "utf8"));
  cases.push(sequence(`recorded-${recording.split("-").slice(0,2).join("-")}-${mode}`, (add,s) => {
    const parser = createStreamingParser(schema);
    let source = "";
    const initial = [["$restored",add("saved")],["$period",add("persisted period")]] as Entry[];
    for (const [i,chunk] of data.chunks.entries()) {
      source += chunk.text;
      const final = i === data.chunks.length-1;
      const parsed = parser.set(source,final);
      s.push({op:"initialize",chunk:chunk.text,final,
        defaults:Object.entries(parsed.stateDeclarations).map(([k,v]) => [k,add(v)]),
        persisted:mode === "every" || (mode === "first" && i === 0) ? initial : []});
      if (i % 13 === 0) s.push({op:"read",name:"$period"});
    }
  }, {recording, schema:schemaFile, persistedMode:mode}));
}

// One explicitly excluded divergence. Run upstream evaluation and both stores;
// JSON's normalization is exactly what the core evaluator's public Value returns.
const expression = "{a: $m.b}";
const evaluated = evaluate(parseExpression(tokenize(expression)), {getState:()=>({}),resolveRef:()=>undefined});
function divergence(value: unknown) {
  const store = createStore();
  store.initialize({$f:{}},{});
  let notified = false;
  store.subscribe(()=>notified=true);
  store.set("$f",value);
  return {changed:notified,json:JSON.stringify(store.getSnapshot())};
}
const excludedDivergence = {name:"nested-undefined-is-normalized-before-the-store", expression,
  upstream:divergence(evaluated),normalized:divergence(JSON.parse(JSON.stringify(evaluated)))};

// One call and its expected observations per line; pool values also stay compact.
const observationFields = ["changed","notified","snapshotChanged","mappedVersion","snapshot","pristine","read"];
console.log('{"observations":'+JSON.stringify(observationFields)+',"versionMapping":"upstream notifications plus nonempty dispose","divergence":'+JSON.stringify(excludedDivergence)+',"sequences":[');
for (const [i,c] of cases.entries()) {
  const {steps,pool,snapshots,...about} = c;
  console.log(JSON.stringify(about).slice(0,-1)+',"pool":'+JSON.stringify(pool)+',"snapshots":'+JSON.stringify(snapshots)+',"steps":[');
  for (const [j,step] of steps.entries()) console.log(JSON.stringify({...step,expected:observationFields.map(f=>(step.expected as any)[f])})+(j+1<steps.length?',':''));
  console.log(']}'+(i+1<cases.length?',':''));
}
console.log(']}');
