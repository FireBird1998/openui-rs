// Record the Intelligent UI library's theme and its prop value rules from
// upstream's own source, for the Swift library's tests (#159). Upstream is
// the library commit in reference/UPSTREAM; the checkout must hold both
// packages/libraries/intelligent-ui and packages/react-components there.
//
// - `tokens`: every token of shared/tokens.ts as written (a var() chain).
// - `light`, `dark`: each token resolved in that mode, as the page sees it:
//   the library's ThemeProvider writes its defaults (defaultThemes.ts) as
//   --openui-intelligent-* and react-components' ThemeProvider writes its
//   default theme (defaultTheme.ts) as --openui-*; every var() is then
//   replaced by its value or its fallback. color-mix() is left for the test.
// - `cases`: upstream's fg, bg, size, radius and space from shared/props.ts,
//   run on a table of inputs. A non-string result is recorded as null.
//
// Usage, from reference/:
//   BUN_RUNTIME_TRANSPILER_CACHE_PATH=0 bun --no-install intelligent-ui-theme.ts "$checkout"
import { readFileSync } from "node:fs";
import { join } from "node:path";

const [checkout] = process.argv.slice(2);
if (!checkout) throw new Error("usage: intelligent-ui-theme.ts <upstream checkout>");
const pin = readFileSync(join(import.meta.dir, "UPSTREAM"), "utf8").split("\n");
const library = "packages/libraries/intelligent-ui";
const manifest = JSON.parse(readFileSync(join(checkout, library, "package.json"), "utf8"));
for (const field of [
  `library package: ${manifest.name} ${manifest.version}`,
  `library path: ${library}`,
]) {
  if (!pin.includes(field)) throw new Error(`UPSTREAM does not match the checkout: ${field}`);
}

const src = join(checkout, library, "src");
const components = join(checkout, "packages/react-components/src/components/ThemeProvider");
const { tokens } = await import(join(src, "shared/tokens.ts"));
const { fg, bg, size, radius, space } = await import(join(src, "shared/props.ts"));
const intelligent = await import(join(src, "components/ThemeProvider/defaultThemes.ts"));
const reactComponents = await import(join(components, "defaultTheme.ts"));
const { camelToKebab } = await import(join(components, "utils.ts"));

/** The variables both providers write for a mode. */
function variables(mode: "light" | "dark"): Map<string, string> {
  const vars = new Map<string, string>();
  const components = mode === "light" ? reactComponents.defaultLightTheme : reactComponents.defaultDarkTheme;
  const library = mode === "light" ? intelligent.defaultLightTheme : intelligent.defaultDarkTheme;
  for (const theme of [components, library]) {
    for (const [key, value] of Object.entries(theme)) {
      if (typeof value === "string") vars.set(`--openui-${camelToKebab(key)}`, value);
    }
  }
  return vars;
}

/** The end of the parenthesised group opening at `open`. */
function closing(text: string, open: number): number {
  let depth = 0;
  for (let i = open; i < text.length; i++) {
    if (text[i] === "(") depth++;
    else if (text[i] === ")" && --depth === 0) return i;
  }
  throw new Error(`unbalanced: ${text}`);
}

/** `text` with every var() replaced by its variable's value, else its fallback. */
function resolve(text: string, vars: Map<string, string>, depth = 0): string {
  if (depth > 32) throw new Error(`var() too deep: ${text}`);
  const at = text.indexOf("var(");
  if (at < 0) return text;
  const open = at + 3;
  const end = closing(text, open);
  const inner = text.slice(open + 1, end);
  let comma = -1;
  for (let i = 0, d = 0; i < inner.length; i++) {
    if (inner[i] === "(") d++;
    else if (inner[i] === ")") d--;
    else if (inner[i] === "," && d === 0) { comma = i; break; }
  }
  const name = (comma < 0 ? inner : inner.slice(0, comma)).trim();
  const fallback = comma < 0 ? undefined : inner.slice(comma + 1).trim();
  const value = vars.get(name) ?? fallback;
  if (value === undefined) throw new Error(`unset variable with no fallback: ${name}`);
  const replaced = text.slice(0, at) + resolve(value, vars, depth + 1) + text.slice(end + 1);
  return resolve(replaced, vars, depth + 1);
}

function resolved(mode: "light" | "dark") {
  const vars = variables(mode);
  return Object.fromEntries(Object.entries(tokens as Record<string, string>).map(([k, v]) => [k, resolve(v, vars)]));
}

// Inputs taken from upstream's source: every token, each word and character
// UNSAFE_CSS refuses, the whitespace JavaScript's trim() and \s know, each
// unit size() accepts, and the near misses of each.
const colorTokens = [
  "ink", "muted", "faint", "canvas", "surface", "surface-strong", "border",
  "info", "success", "warning", "danger", "purple", "pink",
  "chart1", "chart2", "chart3", "chart4", "chart5", "chart6",
];
const unsafeWords = ["url", "image", "image-set", "cross-fade", "element", "expression", "paint", "var", "attr", "env"];
const colorInputs: unknown[] = [
  ...colorTokens,
  "Ink", " ink ", "chart7", "chart0", "infoSoft", "surfaceStrong", "surface_strong",
  "", "   ", "\t\n",
  "red", "Red", "transparent", "currentColor", "#fff", "#EFE9DF", "#0285ff80",
  "rgb(0, 0, 0)", "rgba(255 0 0 / 50%)", "hsl(120 50% 50%)", "oklch(0.6 0.2 30)",
  "color-mix(in srgb, red 40%, blue)", "linear-gradient(red, blue)", "not a colour",
  ...unsafeWords.map((w) => `${w}(x)`),
  ...unsafeWords.map((w) => `${w.toUpperCase()} (x)`),
  "url\t(x)", "url\n(x)", "url (x)", "url﻿(x)", "url (x)", "url　(x)", "url\u0085(x)", "url​(x)",
  "curl(x)", "urls(x)", "url", "myvar(--a)", "image-sets(x)",
  "@import x", "@IMPORT x", "@importx", "@ import",
  "red;", "a{b", "a}b", "<b>", "a>b", "a\\b", "red !important",
  "﻿red﻿", " red ", " red ", "\u0085red", "​red",
  "constructor", "toString", "__proto__", "hasOwnProperty", "valueOf", "isPrototypeOf",
  12, 0, true, false, null, [], ["ink"], { ink: 1 },
];
const sizeInputs: unknown[] = [
  0, 1, 12, 12.5, -5, -0.5, 1e21, 0.000001,
  "0", "12", "12.5", "007", " 12 ", "12.", ".5", "-12", "+12", "1e3", "١٢", "１２",
  "12px", "12.5px", "50%", "1.5em", "2rem", "10vh", "10vw", "1fr", "auto",
  "12PX", "50 %", "12 px", "-12px", ".5px", "1e3px", "12pt", "12ch", "AUTO", " auto ", "autos",
  "12px\n", "\t50%\t", "﻿12", " 12px", "\u008512", "12​",
  "calc(100% - 4px)", "", "  ", "none",
  true, false, null, [], [12], { px: 12 },
];
const radiusInputs: unknown[] = [
  "sm", "md", "lg", "xl", "full", "SM", " sm", "xs", "2xl", "none",
  "12", "12px", "50%", "auto", "constructor", "toString", "__proto__",
  0, 4, 4.5, 999, -3, 1e21,
  true, null, [], ["sm"],
];
const spaceInputs: unknown[] = [0, 1, 2, 2.5, 6, -1, -0.25, 1e21, "2", "2px", "", true, null, [], [2]];

const run = (f: (v: unknown) => unknown, inputs: unknown[]) =>
  inputs.map((input) => {
    const output = f(input);
    return { input, output: typeof output === "string" ? output : null };
  });

const out = {
  upstream: { commit: pin.find((l) => l.startsWith("library commit: "))?.slice(16), package: `${manifest.name} ${manifest.version}` },
  tokens,
  light: resolved("light"),
  dark: resolved("dark"),
  cases: {
    fg: run(fg, colorInputs),
    bg: run(bg, colorInputs),
    size: run(size, sizeInputs),
    radius: run(radius, radiusInputs),
    space: run(space, spaceInputs),
  },
};
process.stdout.write(JSON.stringify(out, null, 2) + "\n");
