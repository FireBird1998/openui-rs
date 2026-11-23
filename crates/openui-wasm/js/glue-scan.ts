// Reads the wasm-bindgen output alone (before scripts/wasm.sh appends the
// guard) for the scans in pointer-redirection.test.ts and
// private-helpers.test.ts. A line of generated code the scan cannot read is
// reported in `unparsed`, never skipped (#217): a shape it does not understand
// could be a read through an instance it never saw.
import { readFileSync } from 'node:fs';

/** The wasm-bindgen output alone, before scripts/wasm.sh appends our files. */
export function generatedGlue(path: string) {
  const source = readFileSync(path, 'utf8');
  const appended = readFileSync(new URL('./tool-provider.js', import.meta.url), 'utf8');
  const at = source.indexOf(appended);
  if (at < 0) throw new Error(`${path}: appended tool-provider.js not found`);
  return source.slice(0, at);
}

const parameters = (list: string) => list.split(',').map(p => p.trim()).filter(Boolean);

export type Generated = {
  found: { owner: string, name: string, params: string[], body: string }[],
  classes: string[],
  /** Method-like lines, or body shapes, the scan cannot parse. */
  unparsed: string[],
};

/** Shapes inside a method or function body that hide a read from the scan. */
const hiddenShapes: [string, RegExp][] = [
  ['destructuring', /(?:\b(?:const|let|var)\s*[{[]|[)\s,;]\s*[{[][^\n=]*[}\]]\s*=(?!=))/],
  ['an alias of this', /(?<![=!<>])=\s*this\s*(?:[;,]|$)/],
  ['Reflect.get', /\bReflect\s*\.\s*get\b/],
  ['yield', /\byield\b/],
  ['await', /\bawait\b/],
  ['with', /\bwith\s*\(/],
];

/** Each method and exported function in the glue, with its parameters and body. */
export function generatedFunctions(glue: string, web: boolean): Generated {
  const lines = glue.split('\n');
  const found: Generated['found'] = [];
  const classes: string[] = [];
  const unparsed: string[] = [];
  const exported = new Set<string>();
  const check = (owner: string, name: string, body: string) => {
    for (const [what, pattern] of hiddenShapes) {
      for (const line of body.split('\n')) {
        if (pattern.test(line)) unparsed.push(`${owner}.${name}: ${what}: ${line.trim()}`);
      }
    }
  };
  for (const line of lines) {
    const m = /^exports\.(\w+) = \1;$/.exec(line);
    if (m) exported.add(m[1]);
  }
  for (let i = 0; i < lines.length; i++) {
    const klass = /^(?:export )?class (\w+) \{$/.exec(lines[i]);
    if (klass) {
      classes.push(klass[1]);
      for (i++; lines[i] !== '}'; i++) {
        const line = lines[i];
        const method = /^ {4}(?:static )?(?:get |set )?(\w+)\(([^)]*)\) \{$/.exec(line);
        if (!method) {
          // Anything at member level that is not a method, a doc comment or
          // blank is a shape this scan does not understand: async and
          // generator methods, computed names, fields, one-line methods.
          if (/^ {4}\S/.test(line) && !/^ {4}(?:\/\*\*|\*\/)/.test(line) && line !== '    }') {
            unparsed.push(`${klass[1]}: ${line.trim()}`);
          }
          continue;
        }
        const start = i;
        while (lines[i] !== '    }') i++;
        const body = lines.slice(start + 1, i).join('\n');
        found.push({ owner: klass[1], name: method[1], params: parameters(method[2]), body });
        check(klass[1], method[1], body);
      }
      continue;
    }
    if (/^(?:export )?(?:async )?class\b/.test(lines[i])) { unparsed.push(lines[i].trim()); continue; }
    const fn = /^(export )?function (\w+)\(([^)]*)\) \{$/.exec(lines[i]);
    if (fn && (web ? fn[1] !== undefined : exported.has(fn[2]))) {
      const start = i;
      while (lines[i] !== '}') i++;
      const body = lines.slice(start + 1, i).join('\n');
      found.push({ owner: 'module', name: fn[2], params: parameters(fn[3]), body });
      check('module', fn[2], body);
    } else {
      // An async or generator function that is part of the public API. The
      // web loader's own async helpers (__wbg_*) are not.
      const odd = /^(export )?(?:async function\s*\*?|function\s*\*)\s*(\w+)/.exec(lines[i]);
      if (odd && (web ? odd[1] !== undefined : exported.has(odd[2]))) unparsed.push(lines[i].trim());
    }
  }
  return { found, classes, unparsed };
}
