// Set up the temporary upstream package tree used by the two library importers.
// The caller owns this scratch directory. Only the reference's zod loads for real
// alongside the vendored lang-core. Capture createLibrary's original declarations
// before its component and function arrays become records.
import { mkdirSync, symlinkSync, writeFileSync } from "fs";
import { join } from "path";

export function prepareLibraryPackage(upstream: string) {
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
  const langCore = join(import.meta.dir, "lang-core", "src", "index");
  file(
    "@openuidev/react-lang/package.json",
    JSON.stringify({ name: "@openuidev/react-lang", type: "module", main: "index.ts" }),
  );
  file(
    "@openuidev/react-lang/index.ts",
    `import { createLibrary as upstreamCreateLibrary, markReactive } from ${JSON.stringify(langCore)};\n` +
      'import { stub } from "react/stub.js";\n' +
      `export * from ${JSON.stringify(langCore)};\n` +
      "export const declarations = new WeakMap();\n" +
      "export function createLibrary(input) {\n" +
      "  const library = upstreamCreateLibrary(input);\n" +
      "  declarations.set(library, input);\n  return library;\n}\n" +
      "export function reactive(schema: any) {\n  markReactive(schema);\n  return schema;\n}\n" +
      `export const ${reactLangOnly.map((n) => `${n} = stub`).join(", ")};\n`,
  );
  try {
    symlinkSync(join(import.meta.dir, "node_modules", "zod"), join(modules, "zod"));
  } catch (e) {
    if ((e as NodeJS.ErrnoException).code !== "EEXIST") throw e;
  }
  process.env.OPENUI_TELEMETRY_DISABLED = "1";
  process.env.DO_NOT_TRACK = "1";
  process.env.NODE_ENV = "production";
  return join(modules, "@openuidev/react-lang/index.ts");
}
