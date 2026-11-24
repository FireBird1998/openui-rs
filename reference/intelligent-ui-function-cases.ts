// The inputs intelligent-ui-functions.ts runs through upstream's Intelligent
// UI functions: one list of argument lists per function, in upstream's group
// order. A function's ticket adds its section; each argument list is one call,
// positional as the model writes it. Values may be anything JavaScript holds:
// -0, Infinity, NaN and lone surrogates are written by the recording in its
// tagged encoding (see intelligent-ui-functions.ts).

/** One call: its positional arguments. Fewer than the params leaves the rest missing. */
export type Call = unknown[];

const NaN_ = Number.NaN;

// Write these arguments inline so the parser validates the optional literal.
export const prunedOptionalLiterals: [string, Call][] = [];
for (const n of [Infinity, -Infinity, NaN_]) {
  prunedOptionalLiterals.push(
    ["Pad", ["x", 5, n]], ["Split", ["a,b", n]],
    ["Join", [[1, 2], n]], ["Join", [[1, 2], "|", n]], ["Duration", [1, n]],
    ["Random", [1, 2, n]],
  );
}

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
  "pruned array items": [["Stats", [[1, "a", null, 2]]], ["Allocate", [9, [1, "a", 2]]]],
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

// lists.ts and helpers.ts. Shared inputs exercise the different callers of
// same(), including mixed number/string ids inside arrays and objects.
const equals: [unknown, unknown][] = [
  [1.0000000000000002, "1.0000000000000002"], [1e100, "1e+100"],
  [1, "1"], [1.5, "1.50"], [1e21, "1e+21"], [1e-7, "1e-7"],
  [1e-6, "0.000001"], [1e20, "100000000000000000000"], [-0, "-0"],
  [true, "true"], [null, "null"], ["é", "e\u0301"],
  [[1, { a: 2 }], ["1", { a: "2" }]], [[1], { a: 1 }],
  [{ b: 2, a: 1 }, { a: "1", b: 2 }], [{ a: 1 }, { b: 1 }],
];
const rows = [{ id: 1, data: [{ name: "Ada" }] }, { id: "1" }, null, [2]];
const positions = [-1e300, -5, -3, -1.9, -0.9, -0, 1.9, 3, 5, 1e300];
const simpleLists = [[], [1], [null, 1, true, "a", [1, 2], { id: 3 }]];
const nearLimit = Array(9999).fill(0);
const full = [...nearLimit, 1];

const lists: Record<string, Call[]> = {
  Append: [...simpleLists.map(xs => [xs, [1]]), [nearLimit, 1], [full, 1]],
  Prepend: [...simpleLists.map(xs => [xs, { id: 1 }]), [nearLimit, 1], [full, 1]],
  Concat: [[[], []], [[1], []], [[], [2]], [[1, null], [[2], 3]], [nearLimit, [1]], [nearLimit, [1, 2]]],
  Remove: [...equals.map(([a, b]) => [[a, 8, b], "", b]), [rows, "id", 1], [rows, "data.0.name", "Ada"]],
  RemoveAt: positions.map(i => [[1, 2, 3], i]),
  Update: [...equals.map(([a, b]) => [[{ id: a, kept: true }], "id", b, "id", false]),
    [rows, "id", 1, "a.b", [0]], [[{ z: 1 }], "z", 1, "0", 2],
    [[{ z: 1 }], "z", 1, "4294967295", 2], [[{ z: 1 }], "z", 1, "01", 2], [rows, "data.0.name", "Ada", "id", 2], [rows, "no", 1, "yes", 2]],
  Toggle: [...equals.map(([a, b]) => [[a, 8, a], b]), [[], 1], [full, 2]],
  Includes: [...equals.map(([a, b]) => [[a], b]), ["a12b", 12], ["x,y", ["x", "y"]],
    ["[object Object]", {}], ["1,,2", [1, null, [2]]], ["é", "e\u0301"], ["e\u0301", "e"],
    ["", ""], ["true", true], ["\uFFFD", "\ud800"], ["\uFFFD", "\uFFFD"], ["42", { toString: 42 }], [17, 1],
    ...[true, 2, null, "a", [], {}].map(some => [{ some }, 2])],
  Find: [...equals.map(([a, b]) => [[{ id: a }, { id: b }], "id", b]),
    [rows, "data.0.name", "Ada"], [rows, "data.length", 1],
    [[{}, []], "__proto__", {}], [[[], {}], "__proto__", []], [[[], {}], "__proto__.__proto__", {}],
    [[{ x: { y: 3 } }], "x.y", 3], [[{ x: ["x", "y"] }], "x.01", "y"], [rows, "absent", 0], [[], "", 1]],
  IndexOf: [...equals.map(([a, b]) => [[0, a, b], b]), [[], 1]],
  Take: positions.map(n => [[1, 2, 3], n]),
  Slice: [...positions.map(n => [[1, 2, 3], n]), ...positions.map(n => ["e\u0301😀z", 0, n]),
    ["😀", 0, 1], ["😀", 1, 2], ["ab😀cd", -4, -2], ["abc", 2, 1], [[1, 2], 1, null], [17, 0]],
  Unique: [...equals.map(([a, b]) => [[a, b, a]]), [[true, "true", null, "null", 0, -0, "0"]],
    [[[1], { a: 1 }, { a: "1" }, ["1"]]]],
  Reverse: [...simpleLists.map(xs => [xs]), [[1, "a", null, 2]], [[Infinity]], [[null]]],
  Range: [[0, 0], [0, 4], [3, -3, -2], [3, 0], [0, 3, -1], [0, 3, 0], [0, 3, -0],
    [0, 1, 0.1], [1.23456789012345, 1.234567890124, 1e-13],
    [123456789012.5, 123456789013, 0.5], [-123456789012.5, -123456789013, -0.5],
    [0, 4e-323, 5e-324], [0, 0.9, 0.3], [0, 10000], [0, 10001], [-1e308, 1e308, 1e308], [0, 1, 1e-300],
    [Infinity, 1], [-Infinity, 1], [NaN_, 1]],
  Pick: [...simpleLists, [1, 2], [1, 2, 3, 4, 5]].map(xs => [xs]),
  Shuffle: [...simpleLists, [1, 2], [1, 2, 3, 4, 5], [1, 1, 1]].map(xs => [xs]),
  Flatten: [...simpleLists.map(xs => [xs]), [[[1, [2]], [], 3]], [[nearLimit, [1]]], [[full, [1]]]],
  Chunk: [...positions.map(n => [[1, 2, 3, 4, 5], n]), [[], 2], [[1, 2, 3], 2], [[1], 0.99]],
};
// Missing, null and wrong-type parameters reach the same validated calls as
// the model's programs do.
for (const calls of Object.values(lists)) calls.push([], [null], [false]);

// text.ts. Match and measure code units; keep decomposed accents and case context.
const words = ["", "a", "Straße", "ΟΣ", "ΣΑΣ", "İıi", "ﬃ", "𐐨😀", "e\u0301",
  'one-two/three(four"five', "one.two'four_five", "\uFEFFa\u0085b\u200Bc", "\ud800"];
const spaces = ["\t", "\n", "\r", "\u000b", "\u00a0", "\u1680", "\u2000", "\u200a",
  "\u2028", "\u2029", "\u202f", "\u205f", "\u3000", "\ufeff", "\u0085", "\u180e", "\u200b"];
const text: Record<string, Call[]> = {
  Join: [[[]], [[1, null, "", true, false, -0]], [[1e-7, 1e-6, 1e20, 1e21], "|"],
    [[{ b: 1e-5, a: [true, null, '"\\\n'] }, [1, 2]], ""], [rows, "/", "data.0.name"],
    [[{ x: [0, false] }, {}, null], "|", "x.1"], [[[], [2], "abc"], ";", "length"],
    [[{ a: 1 }], "", "__proto__"], [[{ a: 1 }], "", "constructor"], [[[1]], "", "join"],
    [[{ n: Infinity, xs: [NaN_] }]], [["\ud800", "😀"]], [[Infinity, 1], ","], [[null, 1], ","],
    [[{ n: null, xs: [null] }]], [["�", "😀"]], [[{ s: "\ud800" }]], [[{ s: "�" }]],
    [[[0, "\b\f\n\r\t\u0000\u001f\\\""]]]],
  Pad: [["x", 0], ["😀", 2], ["e\u0301", 3, "ab"], [1e21, 8, ""], [-0, 3],
    [true, 6], [[1, null, [2]], 8, "x"], [{}, 0], [{ toString: 42 }, 3],
    ["a", 4.9, "xy"], ["z", 4, "😀"], ["x", 999], ["x", 1000], ["x", 1001],
    ["x", -1e300], ["x", 1e300], ["\udc00", 2], [1, Infinity], [1, -Infinity], [1, NaN_], [Infinity, 3], [-Infinity, 3], [NaN_, 3]],
  Upper: words.map(s => [s]),
  Lower: [...words, "AΣ\u0301", "AΣ\u0301B", "AΣ\0B", "\u0301Σ", "A\u0345Σ", "AΣ\u0345B"].map(s => [s]),
  Trim: [...words, ...spaces.map(s => s + "x" + s)].map(s => [s]),
  Capitalize: [...words, ...spaces.map(s => "a" + s + "b"), "ß ß", "😀apple", "e\u0301clair",
    "1a /-apple", "(𐐨"].map(s => [s]),
  Split: [[""], [", a,,b , "], ["\ufeffa\u0085,b"], ["😀a", ""], ["e\u0301", ""],
    ["e\u0301e", "e"], ["é", "e\u0301"], ["𐐨😀𐐨", "𐐨"], ["aaabaaa", "aa"],
    ["\ud800", ""], ["�", ""], ["�x", "\ud800"], ["�x", "�"], ["a\0b", "\0"], ["\na\t", ""], ["a".repeat(10000), ""], ["a".repeat(10001), ""],
    [" ".repeat(10001) + "a", ""]],
  Replace: [["", "", "a"], ["aba", "", "x"], ["aaa", "aa", "$&"], ["a.a", ".", "$1"],
    ["one two", " ", ""], ["e\u0301", "e", ""], ["é", "e\u0301", "x"], ["😀z😀", "😀", "𐐨"],
    ["abc", "b", "\ud800"], ["\ud800", "x", "z"], ["�", "x", "z"], ["abc", "b", "�"],
    ["x�x", "\ud800", "y"], ["x�x", "�", "y"], ["a\0b", "\0", "z"]],
  Duration: [...[-0, -0.4, -0.5, 0.49999999999999994, 0.5, 59.5, 60, 61, 119,
    3599.5, 3601, 3661, -4800, 1e20, 1e100].flatMap(n => [[n], [n, "clock"]]),
    [Infinity], [-Infinity], [NaN_]],
  Default: [[null, "fallback"], ["", 1], [" ", 1], [false, 1], [0, 1], [-0, 1],
    [[], 1], [{}, 1], [[1, null], 1], [["\ud800"], 1], [{ n: Infinity }, 1], [1, null],
    [Infinity, 1], [-Infinity, 1], [NaN_, 1], [1, Infinity]],
};
for (const calls of Object.values(text)) calls.push([], [null], [false]);
for (const n of [Infinity, -Infinity, NaN_]) {
  // State reads reach the call at runtime. Upstream then prunes an invalid
  // optional argument or short-circuits validation for a required null.
  text.Join.push([[1, 2], n], [[1, 2], "|", n], [null, n]);
  text.Pad.push(["x", 5, n], [null, 5, n]);
  text.Split.push(["a,b", n], [null, n]);
  text.Replace.push([null, n, "x"]);
  text.Duration.push([1, n], [null, n]);
  text.Default.push([n, null]);
}
text.Split.push(["😀\ud800", ""], ["😀�", ""]);

// numbers.ts. Cover both directions of every unit and every temperature pair,
// including same-unit conversions (which still do the arithmetic).
const units = [
  ["m", "mm cm m km in ft yd mi"], ["kg", "mg g kg oz lb st"],
  ["l", "ml l tsp tbsp floz cup pt qt gal"], ["m/s", "m/s km/h mph kn"],
  ["m2", "m2 km2 ft2 ha acre"],
];
export const fixedDrawCalls = [
  // Subtracting these finite bounds overflows in both Random modes.
  ...[[-1e308, 1e308], [-1e308, 1e308, true]].flatMap(args => [0, 0.5].map(draw => ({ args, draw }))),
  // Bun drops the wrong-type integer flag and draws in floating-point mode.
  ...[Infinity, -Infinity, NaN_, 1].map(flag => ({ args: [1, 2, flag], draw: 0.25 })),
];

const numbers: Record<string, Call[]> = {
  Pow: [[2, 10], [-2, 3], [-2, 0.5], [0, -1], [0, 0], [-0, 3], [2, -1074],
    [10, 400], [2, 0.5], [1.001, 37], [1.23456789, 1000], [1.23456789, 1001], [1.3454142683185637, -0.5], [1.1, -3.75], [3, 4.25], [Infinity, 0]],
  Sqrt: [[-1], [-0], [0], [2], [4], [1e-320], [Number.MAX_VALUE], [-Infinity]],
  Random: [[1, 5], [5, 1], [2, 2], [-3.2, 4.8, true], [0.1, 0.9, true], [-2, -2, true],
    [0, 0.25, false], [3, 9, null], [1, Infinity]],
  Clamp: [[-2, 0, 4], [2, 4, 0], [5, 4, 0], [1, 3, 3], [-0, 0, 1], [Infinity, -2, 2]],
  Percent: [[3, 4], [-2, 4], [1, 0], [0, -0], [0.1, 0.3], [1e308, 1e-308], [Infinity, 0]],
  Lerp: [[0, 10, 0.5], [10, 0, 2], [1, 3, -1], [0.1, 0.3, 0.7],
    [-1e308, 1e308, 0], [1e308, 1e308, 0.5], [0, 1, NaN_]],
  Allocate: [[100, [1, 1, 1]], [-100, [1, 1, 1]], [4, []], [2, [0, 0]],
    [2, [1, 4, 1, 1, 1], 0], [4100, Array.from({ length: 10001 }, (_, i) => (i * 7919) % 10001), 0],
    [4, [0, 2, 1]], [3, [1, -1]], [3, [null, 2]], [3, [Infinity, 2]],
    [5, Array(20).fill(1), 0], [1, [0.25, 0.5, 0.25], 0], [1, [3, 2, 2], null],
    [0.49999999999999994, [1], 0], [0.5, [1], 0], [-0.5, [1], 0],
    [1e308, [1, 0]], [4, [1e308, 1e308], 0], [Infinity, [1]], [3, [1], Infinity],
    ...[-3, 0, 0.49, 0.5, 1, 1.5, 2, 5.5, 6, 7, 9].map(d => [1.23456789, [1, 1, 1], d])],
  Payment: [[300000, 0.5, 360], [12, 0, 4], [10, -0, 2.5], [1, 1, 0], [1, 1, -1],
    [-1, 0.01, 9], [1, -100, 2], [1, -120, 0.5], [12, -200, 3], [1, 1e-30, 2],
    [1, 100, 10000], [1, 3.5, 1.5], [1, 0, Infinity]],
  Convert: [...units.flatMap(([base, names]) => names.split(" ").flatMap(u => [[4.125, u, base], [-7.75, base, u]])),
    ...["c", "f", "k"].flatMap(a => ["c", "f", "k"].map(b => [-40.1, a, b])),
    [1, "  FT\uFEFF", " In\n"], [1, "K", "c"], [1, "\u0085m", "cm"], [1, "m", "kg"],
    [1, "c", "m"], [1, "", "m"], [1, "Constructor", "constructor"], [1, "__proto__", "constructor"],
    [1, "toString", "toString"], [1, "\ud800", "m"], [1e308, "km", "km"], [NaN_, "m", "m"]],
};
for (const calls of Object.values(numbers)) calls.push([], [null], [false]);
// Bun short-circuits on a required null. Native checks the other argument
// before validation. The finite control takes the silent-null path in both.
for (const n of [Infinity, -Infinity, NaN_, 1]) {
  numbers.Pow.push([n, null]);
  numbers.Random.push([n, null]);
  numbers.Clamp.push([n, null, 2]);
  numbers.Percent.push([n, null]);
  numbers.Lerp.push([n, null, 0.5]);
  numbers.Allocate.push([n, null]);
  numbers.Payment.push([n, null, 2]);
  numbers.Convert.push([n, null, "m"]);
}

/** Every function's calls, by name. */
export const cases: Record<string, Call[]> = {
  ...lists,
  ...text,
  ...numbers,
  MixColor: mixColor,
};
