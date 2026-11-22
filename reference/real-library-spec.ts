// Import a real library's declarations and independent upstream output.
// Run with --preload ./react-ui-stubs.ts (see TESTING.md).
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { join, resolve } from "node:path";
import { prepareLibraryPackage } from "./library-package";
import { toLibraryDefinition } from "./library-definition";

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
const scratch = mkdtempSync(join(target, "library-"));
try {
  const src = join(scratch, pkg, "src");
  cpSync(join(checkout, pkg, "src"), src, { recursive: true });
  const { declarations } = await import(prepareLibraryPackage(scratch));
  const library = react
    ? (await import(join(src, "genui-lib/openuiLibrary.tsx"))).openuiLibrary
    : (await import(join(src, "lib/library.ts"))).library;
  const schema = library.toJSONSchema();
  const spec = library.toSpec();
  if (react) {
    // These two oracles are already imported at the lang-core pin.
    const imported = (file: string) =>
      JSON.parse(readFileSync(resolve(import.meta.dir, "../conformance", file), "utf8"));
    if (JSON.stringify(schema) !== JSON.stringify(imported("imported/schemas/react-ui.json")) ||
        JSON.stringify(spec) !== JSON.stringify(imported("prompts/react-ui-schema.json").library)) {
      throw new Error("react-ui has drifted from the pinned schema or prompt spec");
    }
  }
  const definition = toLibraryDefinition(declarations.get(library));
  console.log(JSON.stringify(react ? definition : { definition, schema, spec }, null, 1));
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
