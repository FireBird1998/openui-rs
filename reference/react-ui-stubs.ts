// Preloaded by react-ui-spec.ts: loads upstream's react-ui package for its
// component schemas only. Imports of third-party packages (React, icons,
// charts) and of style sheets are replaced with one inert stub, so nothing has
// to be installed; zod, react-lang and react-ui's own modules load as written.
// Bun applies a runtime plugin's onResolve only to the entry file, so this
// rewrites import statements as each file loads instead.
import { plugin } from "bun";
import { readFileSync } from "fs";

const KEEP = (spec: string) =>
  spec.startsWith("zod") ||
  spec === "@openuidev/react-lang" ||
  (spec.startsWith(".") && !/\.s?css$/.test(spec));
const IMPORT =
  /^import\s+(type\s+)?([\s\S]*?)\s*from\s*["']([^"']+)["'];?|^import\s+["']([^"']+)["'];?/gm;

/** The local names an import clause binds, less type-only ones. */
function bindings(clause: string): string[] {
  const names: string[] = [];
  const named = clause.match(/\{([\s\S]*)\}/);
  for (const part of named ? named[1].split(",") : []) {
    const p = part.trim();
    if (p && !p.startsWith("type ")) names.push(p.split(/\s+as\s+/).pop()!.trim());
  }
  const rest = clause.replace(/\{[\s\S]*\}/, "").replace(/,/g, " ").trim();
  const namespace = rest.match(/\*\s+as\s+(\w+)/);
  if (namespace) names.push(namespace[1]);
  else if (rest) names.push(...rest.split(/\s+/).filter(Boolean));
  return names;
}

plugin({
  name: "react-ui-schemas-only",
  setup(build) {
    build.onLoad({ filter: /packages\/react-ui\/src\/.*\.m?[jt]sx?$/ }, ({ path }) => {
      const stubbed: string[] = [];
      const contents = readFileSync(path, "utf8").replace(
        IMPORT,
        (all, typeOnly, clause, spec, bare) => {
          if (bare) return KEEP(bare) ? all : "";
          if (KEEP(spec)) return all;
          if (!typeOnly) stubbed.push(...bindings(clause));
          return "";
        },
      );
      const header = stubbed.length
        ? `import { stub as __stub } from "react/stub.js";\nconst ${stubbed.map((n) => `${n} = __stub`).join(", ")};\n`
        : "";
      const loader = path.endsWith("x") ? "tsx" : path.endsWith("ts") ? "ts" : "js";
      return { contents: header + contents, loader };
    });
  },
});
