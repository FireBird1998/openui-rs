// Regenerate the Swift vector resource from an already available lucide-react
// package: bun --no-install scripts/lucide-artwork.mjs /path/to/lucide-react
import { copyFileSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";

const pkg = resolve(process.argv[2]);
const { name, version } = JSON.parse(readFileSync(resolve(pkg, "package.json")));
if (name !== "lucide-react" || version !== "0.562.0") throw Error("Expected lucide-react 0.562.0");
const map = (await import(pathToFileURL(resolve(pkg, "dist/esm/dynamicIconImports.js")))).default;
const artwork = new Map();
const names = new Map();
const permutations = new Map();
for (const [key, load] of Object.entries(map)) {
  const icon = await load();
  // Several wire names can import the very same module.
  const canonical = icon.default.displayName;
  if (!artwork.has(canonical)) {
    const svg = icon.__iconNode.map(([tag, attributes]) => {
      const attrs = Object.entries(attributes).filter(([name]) => name !== "key")
        .map(([name, value]) => `${name}="${value}"`).join(" ");
      return `<${tag} ${attrs}/>`;
    }).join("");
    artwork.set(canonical, svg);
  }
  names.set(key, canonical);
  // Upstream assigns each permutation in catalog order, then overlays the
  // exact keys. Store one winner per word multiset instead of all permutations.
  permutations.set(key.split("-").sort().join("-"), canonical);
}
const dir = new URL("../swift/OpenUINative/Sources/OpenUIIntelligentUI/Lucide/", import.meta.url);
mkdirSync(dir, { recursive: true });
const catalog = Object.fromEntries(Object.entries({ artwork, names, permutations })
  .map(([key, map]) => [key, Object.fromEntries(map)]));
writeFileSync(new URL("catalog.json", dir), JSON.stringify(catalog, null, 2) + "\n");
copyFileSync(resolve(pkg, "LICENSE"), new URL("LICENSE.txt", dir));
