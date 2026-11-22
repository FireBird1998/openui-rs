// The inputs intelligent-ui-functions.ts runs through upstream's Intelligent
// UI functions: one list of argument lists per function, in upstream's group
// order. A function's ticket adds its section; each argument list is one call,
// positional as the model writes it. Values may be anything JavaScript holds:
// -0, Infinity, NaN and lone surrogates are written by the recording in its
// tagged encoding (see intelligent-ui-functions.ts).

/** One call: its positional arguments. Fewer than the params leaves the rest missing. */
export type Call = unknown[];

const NaN_ = Number.NaN;

// ─── Argument rules ──────────────────────────────────────────────────────────
// What lang-core's runtime does before a function body runs (spec #128,
// research part 3, section 0), recorded on the functions that show each rule
// and written with the rule's name in `rule`. A rule's pairs replay once their
// functions are implemented; until then the replay test counts them as
// recorded, not yet replayed.
export const argumentRules: Record<string, [string, Call][]> = {
  // The param's default applies (Range's step is 1; Default's value is optional).
  "optional null": [["Range", [0, 3, null]], ["Range", [0, 3]], ["Default", [null, 1]]],
  // The error is reported; null items aren't checked and reach the body.
  "pruned array items": [["Stats", [[1, "a", null, 2]]]],
  // The error is reported and the default applies.
  "invalid optional word": [["Format", [5, "bogus"]], ["Duration", [4800, "bogus"]]],
  // z.any() params are required, so null or nothing misses one.
  "required any": [["Append", [[1], null]], ["Default", [5, null]], ["Default", [5]]],
};

// ─── Colours (colors.ts) ─────────────────────────────────────────────────────
const mixColor: Call[] = [
  // The description's example, and the ends.
  ["#ffffff", "#0285ff", 0.5],
  ["#000000", "#ffffff", 0],
  ["#000000", "#ffffff", 1],
  ["#000000", "#ffffff", 0.25],
  ["#000000", "#ffffff", 0.75],
  ["#123456", "#fedcba", 0.3],
  // Three digits, no "#", either case.
  ["#fff", "#000", 0.5],
  ["fff", "000", 0.5],
  ["#FFF", "#abc", 0.3],
  ["#AbCdEf", "#123456", 0.1],
  ["abcdef", "#ABC", 0.9],
  // t is clamped to [0, 1].
  ["#102030", "#405060", -1],
  ["#102030", "#405060", 2],
  ["#102030", "#405060", -0],
  ["#102030", "#405060", 1e-300],
  ["#102030", "#405060", 1e308],
  ["#000000", "#ffffff", 0.9999999999999999],
  // Math.round: ties go up, and the largest double below 0.5 goes down.
  ["#000000", "#030303", 0.5],
  ["#030303", "#000000", 0.5],
  ["#000000", "#010101", 0.5],
  ["#010101", "#000000", 0.5],
  ["#000000", "#010101", 0.49999999999999994],
  ["#000000", "#ff7f01", 0.5],
  // Trimmed with JavaScript's whitespace, and only that.
  ["  #fff  ", "#000", 0.5],
  ["\t#fff\n", "\r#000\v", 0.5],
  ["\f #fff　", "﻿#000 ", 0.5],
  [" #fff ", "  #000  ", 0.5],
  ["\u0085#fff", "#000", 0.5],
  ["᠎#fff", "#000", 0.5],
  ["​#fff", "#000", 0.5],
  // Not a hex colour: the error names the argument as written.
  ["", "#000", 0.5],
  ["#", "#000", 0.5],
  ["#ff", "#000", 0.5],
  ["#ffff", "#000", 0.5],
  ["#fffff", "#000", 0.5],
  ["#fffffff", "#000", 0.5],
  ["#ggg", "#000", 0.5],
  ["##fff", "#000", 0.5],
  ["# fff", "#000", 0.5],
  ["red", "#000", 0.5],
  ["rgb(0, 0, 0)", "#000", 0.5],
  ["  nope ", "#000", 0.5],
  ["#fff\n#000", "#000", 0.5],
  ["#１２３", "#000", 0.5],
  ["#ＡＢＣ", "#000", 0.5],
  ["#٠١٢", "#000", 0.5],
  ["\u{1F600}", "#000", 0.5],
  ["é", "#000", 0.5],
  ["#fff", "nope", 0.5],
  ["bad", "worse", 0.5],
  // Lone surrogates: the error carries one back.
  ["\ud800", "#000", 0.5],
  // t that JSON can't carry.
  ["#102030", "#405060", Infinity],
  ["#102030", "#405060", -Infinity],
  ["#102030", "#405060", NaN_],
  // Argument rules: missing or null required arguments, wrong types, extras.
  [],
  ["#fff"],
  ["#fff", "#000"],
  [null, "#000", 0.5],
  ["#fff", null, 0.5],
  ["#fff", "#000", null],
  [null, null, null],
  [1, "#000", 0.5],
  ["#fff", 2, 0.5],
  ["#fff", "#000", "0.5"],
  ["#fff", "#000", true],
  [["#fff"], "#000", 0.5],
  [{ hex: "#fff" }, "#000", 0.5],
  ["#fff", "#000", 0.5, 7],
];

/** Every function's calls, by name. */
export const cases: Record<string, Call[]> = {
  MixColor: mixColor,
};
