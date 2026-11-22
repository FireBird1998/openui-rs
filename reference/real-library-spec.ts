// Import a real library's declarations and independent upstream output.
// Run with --preload ./react-ui-stubs.ts (see TESTING.md).
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { join, resolve } from "node:path";
import { prepareLibraryPackage } from "./library-package";
import { toLibraryDefinition } from "./library-definition";
import { generateSystemPrompt } from "./lang-core/src/parser/prompt";
import { globalRegistry } from "zod/v4/core";

const [name, checkout] = process.argv.slice(2);
if (!checkout || (name !== "react-ui" && name !== "intelligent-ui")) {
  throw new Error("usage: real-library-spec.ts <react-ui|intelligent-ui> <upstream checkout>");
}
const react = name === "react-ui";
const pkg = react ? "packages/react-ui" : "packages/libraries/intelligent-ui";
if (!react) {
  const pin = readFileSync(join(import.meta.dir, "UPSTREAM"), "utf8").split("\n");
  const manifest = JSON.parse(readFileSync(join(checkout, pkg, "package.json"), "utf8"));
  for (const field of [
    `library package: ${manifest.name} ${manifest.version}`,
    `library path: ${pkg}`,
    `library license: ${manifest.license}`,
  ]) {
    if (!pin.includes(field)) throw new Error(`UPSTREAM does not match the checkout: ${field}`);
  }
}
const target = resolve(import.meta.dir, "../target");
mkdirSync(target, { recursive: true });
const imported = (file: string) =>
  JSON.parse(readFileSync(resolve(import.meta.dir, "../conformance", file), "utf8"));

async function importLibrary(withoutDescriptions = false) {
  const scratch = mkdtempSync(join(target, "library-"));
  try {
    const src = join(scratch, pkg, "src");
    cpSync(join(checkout, pkg, "src"), src, { recursive: true });
    const { declarations } = await import(prepareLibraryPackage(scratch));
    if (!react) {
      const { library } = await import(join(src, "lib/library.ts"));
      return { definition: toLibraryDefinition(declarations.get(library)), schema: library.toJSONSchema(), spec: library.toSpec() };
    }

    // For the control, use the exact receivers of both .describe() calls
    // before any component imports the shared schema or builds a library.
    const { iconPropsSchema } = await import(join(src, "components/_shared/icons/schema.ts"));
    for (const field of ["name", "category"]) {
      const clone = iconPropsSchema.shape[field];
      const parent = clone._zod.parent;
      if (!parent || typeof globalRegistry.get(clone)?.description !== "string" || globalRegistry.has(parent)) {
        throw new Error(`expected Icon.${field} to be a described clone`);
      }
      if (withoutDescriptions) iconPropsSchema.shape[field] = parent;
    }

    const { openuiLibrary: library, openuiPromptOptions: promptOptions } = await import(join(src, "genui-lib/openuiLibrary.tsx"));
    for (const field of ["name", "category"]) {
      const prop = library.components.Icon.props.shape[field];
      if (prop !== iconPropsSchema.shape[field] || globalRegistry.has(prop) === withoutDescriptions) {
        throw new Error(`Icon.${field}: library loaded the wrong description variant`);
      }
    }
    const spec = library.toSpec();
    const request = { library: spec, promptOptions };
    const prompt = imported("golden/prompts/react-ui-schema.json").prompt;
    const defaultPrompt = library.prompt();
    const checks = [
      ["default prompt", defaultPrompt, generateSystemPrompt({ library: spec })],
      ["JSON Schema", JSON.stringify(library.toJSONSchema()), JSON.stringify(imported("imported/schemas/react-ui.json"))],
      ["spec", JSON.stringify(spec), JSON.stringify(imported("prompts/react-ui-schema.json").library)],
      ["library.prompt", library.prompt(promptOptions), prompt],
      ["generateSystemPrompt", generateSystemPrompt(request), prompt],
      ["cloud config", generateSystemPrompt({ ...request, cloud: true }), imported("golden/prompts/react-ui-cloud.json").prompt],
    ];
    for (const [label, actual, expected] of checks) {
      if (actual !== expected) throw new Error(`react-ui ${withoutDescriptions ? "without" : "with"} descriptions: ${label} bytes differ`);
    }
    return {
      definition: toLibraryDefinition(declarations.get(library)),
      prompt: defaultPrompt,
      cloud: generateSystemPrompt({ cloud: true, library: spec }),
    };
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
}

const result = await importLibrary();
if (react && JSON.stringify(result) !== JSON.stringify(await importLibrary(true))) {
  throw new Error("react-ui: removing the two field descriptions changed the definition, default prompt or cloud config");
}
console.log(JSON.stringify(result, null, 1));
