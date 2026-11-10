// Copies the core's WebAssembly build (scripts/wasm.sh at the repo root makes
// it) into wasm/, where the app and `npm run compare` load it. The files are
// build output and git-ignored.
import { copyFileSync, existsSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const from = join(here, "../../../target/wasm-pkg/web");
const to = join(here, "../wasm");
const files = ["openui_wasm.js", "openui_wasm.d.ts", "openui_wasm_bg.wasm", "openui_wasm_bg.wasm.d.ts"];

const missing = files.filter((f) => !existsSync(join(from, f)));
if (missing.length) {
  console.error(
    `next-compare: the WebAssembly build is missing (${missing.join(", ")} in target/wasm-pkg/web).\n` +
      "Build it first, from the repo root:\n\n    scripts/wasm.sh\n",
  );
  process.exit(1);
}
mkdirSync(to, { recursive: true });
for (const f of files) copyFileSync(join(from, f), join(to, f));
console.log(`next-compare: copied the WebAssembly build into wasm/`);
