// Run from reference/: bun --no-install intelligent-ui-functions.ts
//
// Records upstream's Intelligent UI functions (intelligent-ui-functions/, the
// library's src/lib/functions vendored verbatim at the library commit in
// UPSTREAM) on the inputs in intelligent-ui-function-cases.ts, through the
// vendored parser and evaluator, so the runtime's argument rules (null for an
// optional argument, a missing required one, pruned array items, checked
// returns, reported throws) are in every result. Writes
// ../conformance/golden/intelligent-ui-functions.json, which
// swift/OpenUINative/Tests/OpenUIIntelligentUITests/FunctionReplayTests.swift
// replays through the Rust runtime with the library's Swift functions.
//
// Each call is the prop of a probe component. Most hold arguments in state
// variables so the runtime validates them. The optional-literal cases put
// the arguments directly in the call so the parser validates them. The
// recorded `source` is that program. Each runs in four time zones, each in
// its own Bun process with the clock fixed at CLOCK. Math.random uses the
// call's specified draw or a sequence seeded afresh for each call. A call whose
// result is the same in every zone is written once; otherwise once per zone,
// with its `zone`. A call that drew random numbers lists them in `draws`, and
// one the parser reported on (a missing required argument, for one) lists
// those reports in `parseErrors`; `errors` are the evaluation's. A call that
// shows an argument rule names it in `rule`.
//
// Values JSON can't carry are tagged, in arguments and results alike:
//   {"$num": "-0" | "Infinity" | "-Infinity" | "NaN"}
//   {"$utf16": [code units]}   a string holding a lone surrogate
//   {"$undefined": true}       undefined inside a result
// No recorded object is a lone "$num", "$utf16" or "$undefined" key, so the
// tags can't be mistaken for values.
import { deepStrictEqual } from "node:assert";
import { spawnSync } from "node:child_process";
import { cpSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { z } from "zod/v4";
import { prepareLibraryPackage } from "./library-package";
import { unwrapFieldValue } from "./renderer-actions";
import { argumentRules, cases, prunedOptionalLiterals, fixedDrawCalls, type Call } from "./intelligent-ui-function-cases";

const ZONES = ["UTC", "America/New_York", "Asia/Kolkata", "Australia/Lord_Howe"];
/** 2026-10-09T21:30:00.000Z */
const CLOCK = Date.UTC(2026, 9, 9, 21, 30, 0, 0);
const SEED = 0x5eed;
const OUT = "../conformance/golden/intelligent-ui-functions.json";
const TAGS = new Set(["$num", "$utf16", "$undefined"]);

type Recorded = { value: unknown; errors: unknown[]; parseErrors?: unknown[]; draws?: number[] };
type Case = { function: string; rule?: string; args: unknown; source: string } & Recorded & { zone?: string };

/** The tagged encoding above. */
function encode(value: unknown): unknown {
  if (value === undefined) return { $undefined: true };
  if (value === null || typeof value === "boolean") return value;
  if (typeof value === "number") {
    if (Object.is(value, -0)) return { $num: "-0" };
    return Number.isFinite(value) ? value : { $num: String(value) };
  }
  if (typeof value === "string") {
    return value.isWellFormed() ? value : { $utf16: Array.from({ length: value.length }, (_, i) => value.charCodeAt(i)) };
  }
  if (Array.isArray(value)) return Array.from(value, encode);
  if (typeof value === "object") {
    const keys = Object.keys(value);
    if (keys.length === 1 && TAGS.has(keys[0])) throw new Error(`a recorded object looks like a tag: ${keys[0]}`);
    const out: Record<string, unknown> = {};
    for (const key of keys) {
      if (!key.isWellFormed()) throw new Error("a recorded key holds a lone surrogate");
      out[key] = encode((value as Record<string, unknown>)[key]);
    }
    return out;
  }
  throw new Error(`cannot record a ${typeof value}`);
}

/** The value as OpenUI Lang source, which the reference's lexer reads back exactly. */
function source(value: unknown): string {
  if (value === null) return "null";
  if (typeof value === "boolean") return String(value);
  if (typeof value === "number") {
    if (Number.isNaN(value)) return "1e"; // a dangling exponent reads as NaN
    if (value === Infinity) return "1e999";
    if (value === -Infinity) return "-1e999";
    if (Object.is(value, -0)) return "-0";
    return value < 0 ? `-${String(-value)}` : String(value);
  }
  if (typeof value === "string") return JSON.stringify(value);
  if (Array.isArray(value)) return `[${value.map(source).join(", ")}]`;
  if (value && typeof value === "object") {
    return `{${Object.entries(value)
      .map(([key, item]) => {
        if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(key)) throw new Error(`no source for the key ${JSON.stringify(key)}`);
        return `${key}: ${source(item)}`;
      })
      .join(", ")}}`;
  }
  throw new Error(`no source for a ${typeof value}`);
}

function calls(): { name: string; args: Call; stateArgs: Call; source: string; rule?: string; draw?: number }[] {
  const call = (name: string, args: Call, rule?: string) => {
    const names = args.map((_, i) => `$a${i}`);
    const state = args.map((arg, i) => `${names[i]} = ${source(arg)}\n`).join("");
    return { name, args, stateArgs: args, source: `${state}root = Probe(@${name}(${names.join(", ")}))`, ...(rule ? { rule } : {}) };
  };
  return [
    ...Object.entries(argumentRules).flatMap(([rule, list]) => list.map(([name, args]) => call(name, args, rule))),
    ...Object.entries(cases).flatMap(([name, list]) => list.map((args) => call(name, args))),
    ...prunedOptionalLiterals.map(([name, args]) => ({
      name, args, stateArgs: [], rule: "pruned optional literal",
      source: `root = Probe(@${name}(${args.map(source).join(", ")}))`,
    })),
    ...fixedDrawCalls.map(({ args, draw }) => ({ ...call("Random", args), draw })),
  ];
}

/** One zone's results, in calls() order. Runs in a process started with TZ set. */
async function record(zone: string): Promise<Recorded[]> {
  // ICU may name the zone by an alias (Asia/Calcutta), so compare offsets.
  const offset = (timeZone?: string) => (ms: number) =>
    new Intl.DateTimeFormat("en-US", { timeZone, timeZoneName: "longOffset" }).format(ms);
  for (const ms of [Date.UTC(2026, 0, 15), Date.UTC(2026, 6, 15), CLOCK]) {
    if (offset()(ms) !== offset(zone)(ms)) throw new Error(`expected the zone ${zone}, got ${offset()(ms)}`);
  }
  const target = resolve(import.meta.dir, "../target");
  mkdirSync(target, { recursive: true });
  const scratch = mkdtempSync(join(target, "intelligent-ui-functions-"));
  try {
    cpSync(join(import.meta.dir, "intelligent-ui-functions"), join(scratch, "src/lib/functions"), { recursive: true });
    prepareLibraryPackage(scratch);
    const { functions } = await import(join(scratch, "src/lib/functions/index.ts"));
    const { createLibrary, defineComponent } = await import("./lang-core/src/library");
    const { compileSchema, parse } = await import("./lang-core/src/parser/parser");
    const { evaluateRoot } = await import("./lang-core/src/runtime/evaluate-tree");

    const probe = defineComponent({ name: "Probe", description: "", props: z.object({ value: z.any() }), component: null });
    const library = createLibrary({ components: [probe], functions, root: "Probe" });
    const schema = library.toJSONSchema();
    const params = compileSchema(schema);
    const draws: number[] = [];
    const runtimeFunctions = Object.fromEntries(
      functions.map((f: { name: string; fn: unknown }) => [f.name, { ...schema.functions![f.name], fn: f.fn }]),
    );
    function evaluate(program: string) {
      const errors: unknown[] = [];
      const parsed = parse(program, params);
      const state = parsed.stateDeclarations;
      const getState = (name: string) => unwrapFieldValue(state[name]);
      const root = evaluateRoot(
        { root: parsed.root },
        { ctx: { functions: runtimeFunctions, getState, resolveRef: () => null }, library, store: null, errors },
      );
      return { value: (root?.props as { value: unknown }).value, errors, parseErrors: parsed.meta.errors, state };
    }

    // The renderer unwraps a field held in state. An inline object remains an object.
    for (const [program, expected] of [
      ["$row = {value: 0}\nroot = Probe(@Append([], $row))", [0]],
      ["root = Probe(@Append([], {value: 0}))", [{ value: 0 }]],
    ] as const) {
      const { value, errors, parseErrors } = evaluate(program);
      deepStrictEqual({ value, errors, parseErrors }, { value: expected, errors: [], parseErrors: [] }, program);
    }

    // The fixed clock and the seeded draws.
    const RealDate = Date;
    class FixedDate extends RealDate {
      constructor(...args: unknown[]) {
        if (args.length === 0) super(CLOCK);
        else super(...(args as [number]));
      }
      static now() {
        return CLOCK;
      }
    }
    globalThis.Date = FixedDate as DateConstructor;
    let fixedDraw: number | undefined;
    let state = SEED;
    Math.random = () => {
      // mulberry32
      state = (state + 0x6d2b79f5) | 0;
      let t = Math.imul(state ^ (state >>> 15), 1 | state);
      t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
      const draw = fixedDraw ?? ((t ^ (t >>> 14)) >>> 0) / 4294967296;
      draws.push(draw);
      return draw;
    };

    return calls().map((call) => {
      fixedDraw = call.draw;
      state = SEED;
      draws.length = 0;
      const { value, errors, parseErrors, state: declarations } = evaluate(call.source);
      // Check the raw declarations independently of the renderer's state lookup.
      const held = call.stateArgs.map((_, i) => declarations[`$a${i}`]);
      if (JSON.stringify(encode(held)) !== JSON.stringify(encode(call.stateArgs)) || Object.keys(declarations).length !== held.length) {
        throw new Error(`${call.source} reads back as ${JSON.stringify(encode(declarations))}`);
      }
      return {
        value: encode(value),
        errors: errors.map(encode),
        ...(parseErrors.length ? { parseErrors: parseErrors.map(encode) } : {}),
        ...(draws.length ? { draws: [...draws] } : {}),
      };
    });
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
}

const zoneFlag = process.argv.indexOf("--zone");
if (zoneFlag >= 0) {
  process.stdout.write(JSON.stringify(await record(process.argv[zoneFlag + 1])));
} else {
  const byZone = ZONES.map((zone) => {
    const run = spawnSync(process.execPath, ["--no-install", import.meta.path, "--zone", zone], {
      env: { ...process.env, TZ: zone },
      encoding: "utf8",
      maxBuffer: 1 << 30,
    });
    if (run.status !== 0) throw new Error(`recording in ${zone} failed:\n${run.stderr}`);
    return JSON.parse(run.stdout) as Recorded[];
  });
  const out: Case[] = [];
  calls().forEach((call, i) => {
    const head = { function: call.name, ...(call.rule ? { rule: call.rule } : {}), args: encode(call.args), source: call.source };
    const texts = byZone.map((results) => JSON.stringify(results[i]));
    if (texts.every((text) => text === texts[0])) out.push({ ...head, ...byZone[0][i] });
    else ZONES.forEach((zone, z) => out.push({ ...head, zone, ...byZone[z][i] }));
  });
  const header = {
    library: "@openuidev/intelligent-ui-library 0.0.2 (thesysdev/openui 7b7a519), src/lib/functions",
    engine: { bun: Bun.version, icu: process.versions.icu, unicode: process.versions.unicode },
    zones: ZONES,
    clock: CLOCK,
    seed: SEED,
  };
  writeFileSync(
    OUT,
    `{"header":${JSON.stringify(header)},\n"cases":[\n${out.map((c) => JSON.stringify(c)).join(",\n")}\n]}\n`,
  );
}
