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
import { join } from "path";
import { prepareLibraryPackage } from "./library-package";

const upstream = process.argv[2];
if (!upstream) throw new Error("usage: react-ui-spec.ts <upstream dir>");
prepareLibraryPackage(upstream);
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
