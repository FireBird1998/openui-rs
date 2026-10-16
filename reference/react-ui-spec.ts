// Upstream react-ui's component library, as the reference sees it: prints
// {schema, spec, promptOptions} as JSON, where schema is the library's
// toJSONSchema(), spec its toSpec() (what generateSystemPrompt takes) and
// promptOptions the package's own examples and rules. Run by import-corpus.ts:
//
//   bun --no-install --preload ./react-ui-stubs.ts react-ui-spec.ts <upstream dir>
//
// The upstream dir holds packages/react-ui/src. React and the package's other
// dependencies are stubs (react-ui-stubs.ts), react-lang is lang-core plus its
// `reactive` marker, and zod is the reference's own, so nothing is installed.
import { mkdirSync, symlinkSync, writeFileSync } from "fs";
import { join } from "path";

const upstream = process.argv[2];
if (!upstream) throw new Error("usage: react-ui-spec.ts <upstream dir>");
const HERE = import.meta.dir;
const modules = join(upstream, "node_modules");

function file(rel: string, content: string) {
  mkdirSync(join(modules, rel, ".."), { recursive: true });
  writeFileSync(join(modules, rel), content);
}
const reactExports = [
  "jsx", "jsxs", "jsxDEV", "Fragment", "createElement", "cloneElement", "isValidElement",
  "Children", "forwardRef", "memo", "lazy", "Suspense", "Component", "PureComponent",
  "createContext", "useContext", "useState", "useEffect", "useLayoutEffect", "useMemo",
  "useCallback", "useRef", "useReducer", "useId", "useImperativeHandle",
  "useSyncExternalStore", "useTransition", "useDeferredValue", "startTransition",
];
file("react/package.json", JSON.stringify({ name: "react", type: "module", main: "index.js" }));
file(
  "react/stub.js",
  "export const stub = new Proxy(function () {}, {\n" +
    '  get: (t, k) => (k === Symbol.toPrimitive ? () => "" : stub),\n' +
    "  apply: () => stub,\n  construct: () => stub,\n});\n",
);
for (const entry of ["index", "jsx-runtime", "jsx-dev-runtime"]) {
  file(
    `react/${entry}.js`,
    'import { stub } from "./stub.js";\nexport default stub;\n' +
      `export const ${reactExports.map((n) => `${n} = stub`).join(", ")};\n`,
  );
}
// react-lang re-exports lang-core; its own exports are React hooks and
// components a schema never calls, and `reactive`, which marks a schema.
const reactLangOnly = [
  "Renderer", "useRendererQuery", "FormNameContext", "useFormName", "useGetFieldValue",
  "useIsQueryLoading", "useIsStreaming", "useRenderNode", "useSetDefaultValue",
  "useSetFieldValue", "useTriggerAction", "useStateField", "FormValidationContext",
  "useCreateFormValidation", "useFormValidation", "WithPreviewRenderer", "useRendererPreview",
];
const langCore = join(HERE, "lang-core", "src", "index");
file(
  "@openuidev/react-lang/package.json",
  JSON.stringify({ name: "@openuidev/react-lang", type: "module", main: "index.ts" }),
);
file(
  "@openuidev/react-lang/index.ts",
  `import { markReactive } from ${JSON.stringify(langCore)};\n` +
    'import { stub } from "react/stub.js";\n' +
    `export * from ${JSON.stringify(langCore)};\n` +
    "export function reactive(schema: any) {\n  markReactive(schema);\n  return schema;\n}\n" +
    `export const ${reactLangOnly.map((n) => `${n} = stub`).join(", ")};\n`,
);
try {
  symlinkSync(join(HERE, "node_modules", "zod"), join(modules, "zod"));
} catch (e) {
  if ((e as NodeJS.ErrnoException).code !== "EEXIST") throw e;
}

process.env.OPENUI_TELEMETRY_DISABLED = "1";
process.env.DO_NOT_TRACK = "1";
process.env.NODE_ENV = "production";
const { openuiLibrary, openuiPromptOptions } = await import(
  join(upstream, "packages/react-ui/src/genui-lib/openuiLibrary.tsx")
);
console.log(
  JSON.stringify({
    schema: openuiLibrary.toJSONSchema(),
    spec: openuiLibrary.toSpec(),
    promptOptions: openuiPromptOptions,
  }),
);
