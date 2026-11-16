// Capture react-ui callbacks from the exact pinned commit, without React or a
// browser. Hooks and visual components are inert; genui field logic is upstream.
// cd reference && bun --no-install ../tools/viewer/Checks/fields-reference.ts <local-upstream-git-checkout> [--write]
import { dirname, join, normalize } from 'node:path';
import { readFileSync, writeFileSync, mkdtempSync, mkdirSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { parseStructuredRules } from '../../../reference/lang-core/src/utils/validation';
import { resolveStateField } from '../../../reference/lang-core/src/runtime/state-field';

const checkout = process.argv[2];
if (!checkout || checkout.startsWith('--')) throw Error('a local upstream Git checkout is required; nothing is fetched');
const transpiler = new Bun.Transpiler({loader: 'tsx', tsconfig: {compilerOptions: {jsx: 'react', jsxFactory: 'React.createElement', jsxFragmentFactory: 'React.Fragment'}}});
const commit = '47b9cae0d3b26a8c9d7514d5887a6d6608801ac9';
function source(path: string): string | undefined {
  path = path.slice(path.indexOf('packages/'));
  const result = Bun.spawnSync(['git', '-C', checkout, 'show', `${commit}:${path}`], {
    env: {...process.env, GIT_NO_LAZY_FETCH: '1'}, stdout: 'pipe', stderr: 'pipe',
  });
  return result.exitCode === 0 ? result.stdout.toString() : undefined;
}
if (JSON.parse(source('packages/react-ui/package.json') ?? '{}').version !== '0.18.1') throw Error('wrong react-ui version');
const state: any = {value: undefined, writes: [], rules: []};
(globalThis as any).__fieldProbe = {
  state, parseStructuredRules,
  field: (name: string, value: unknown) => resolveStateField(name, value, null, null, () => state.value,
    (name, value) => state.writes.push({name, value})),
};
const visuals = ['TextArea', 'RadioGroup', 'RadioItem', 'CheckBoxGroup', 'CheckBoxItem', 'SliderBlock', 'DatePicker', 'Chips', 'OptionCards'];
const temporary = mkdtempSync(join(tmpdir(), 'swift-71-field-probe-'));
function mock(name: string, contents: string) {
  const path = join(temporary, name + '.mjs');
  writeFileSync(path, contents);
  return path;
}
const visualModule = mock('visuals', visuals.map(n => `export const ${n} = '${n}';`).join('\n'));
const partsModule = mock('parts', `export const Icon = {ref: {}}; export const Image = {ref: {}};`);
const reactModule = mock('react', `
  export const useMemo = f => f(); export const useCallback = f => f;
  export const useEffect = f => f();
  export const createElement = (type, props, ...children) => ({type, props: {...props, children}});
  export const Fragment = 'Fragment';
  export default {useMemo, useCallback, useEffect, createElement, Fragment};`);
const langModule = mock('lang', `
  const p = globalThis.__fieldProbe;
  export const ComponentRenderProps = undefined;
  export const defineComponent = d => ({...d, ref: {}});
  export const reactive = s => s;
  export const parseStructuredRules = p.parseStructuredRules;
  export const useStateField = p.field;
  export const useIsStreaming = () => false;
  export const useFormName = () => 'f';
  export const useGetFieldValue = () => () => p.state.value;
  export const useSetFieldValue = () => (form, componentType, name, value, save) => p.state.writes.push({name, value, componentType});
  export const useFormValidation = () => ({registerField: (name, rules, value) => p.state.rules.push({name, rules, value}), unregisterField() {}, clearFieldError() {}, validateField() {}});`);
const compiled = new Map<string, string>();
function compile(path: string): string {
  const cached = compiled.get(path);
  if (cached) return cached;
  for (const extension of ['', '.ts', '.tsx', '/index.tsx']) {
    const contents = source(path + extension);
    if (contents === undefined) continue;
    const destination = join(temporary, path + '.mjs');
    compiled.set(path, destination);
    const code = transpiler.transformSync(contents).replace(/from\s*["']([^"']+)["']/g, (_, specifier) => {
      let resolved;
      if (specifier === 'react') resolved = reactModule;
      else if (specifier === '@openuidev/react-lang') resolved = langModule;
      else if (specifier.startsWith('zod')) resolved = Bun.resolveSync(specifier, join(import.meta.dir, '../../../reference'));
      else if (specifier.includes('/components/')) resolved = visualModule;
      else if (['Icon', 'Image'].includes(specifier.split('/').at(-1))) resolved = partsModule;
      else resolved = compile(normalize(join(dirname(path), specifier)));
      return 'from ' + JSON.stringify(resolved);
    });
    mkdirSync(dirname(destination), {recursive: true});
    writeFileSync(destination, code);
    return destination;
  }
  throw Error(`missing local upstream source ${path}`);
}

function find(node: any, prop: string): any {
  if (node?.props?.[prop]) return node.props[prop];
  for (const child of [node?.props?.children ?? []].flat(Infinity)) {
    const found = find(child, prop);
    if (found) return found;
  }
}
// Calendar/day-picker produces local-midnight Date objects. A non-UTC
// observation distinguishes a picked day from an arbitrary input timestamp.
process.env.TZ = 'Asia/Kolkata';
const inputs = [
  {type: 'TextArea', props: {}, change: 'onChange', value: {target: {value: 'hello\nworld'}}},
  {type: 'RadioGroup', props: {items: [{props: {label: 'Alpha', value: 'a'}}]}, change: 'onValueChange', value: 'a'},
  {type: 'CheckBoxGroup', props: {items: [{props: {name: 'a', label: 'Alpha'}}]}, change: 'onChange', value: true},
  {type: 'CheckBoxGroup', scenario: 'duplicateNames', props: {items: [{props: {name: 'a', label: 'First', defaultChecked: false}}, {props: {name: 'a', label: 'Second', defaultChecked: true}}]}, change: 'onChange', value: false},
  {type: 'Slider', props: {min: 0, max: 10, step: 1, variant: 'discrete'}, change: 'onValueCommit', value: [7]},
  {type: 'DatePicker', props: {}, change: 'setSelectedSingleDate', value: new Date('2026-11-17T00:00:00Z')},
  {type: 'DatePicker', scenario: 'localMidnight', props: {}, change: 'setSelectedSingleDate', value: new Date(2026, 10, 17)},
  {type: 'DatePicker', props: {mode: 'range'}, change: 'setSelectedRangeDates', value: {from: new Date('2026-11-17T00:00:00Z'), to: new Date('2026-11-20T00:00:00Z')}},
  {type: 'Chips', props: {type: 'multiple', items: [{props: {value: 'a', label: 'Alpha'}}]}, change: 'onToggle', value: 'a'},
  {type: 'OptionCards', props: {type: 'single', items: [{props: {value: 'a', title: 'Alpha'}}]}, change: 'onToggle', value: 'a'},
];
try {
const observed = [];
for (const input of inputs) {
  state.value = undefined; state.writes = []; state.rules = [];
  const module = await import(compile(`packages/react-ui/src/genui-lib/${input.type}/index.tsx`));
  const tree = module[input.type].component({props: {name: 'x', rules: {required: true}, ...input.props}, renderNode: (x: unknown) => x});
  const change = find(tree, input.change);
  if (!change) throw Error(`missing ${input.type}.${input.change}`);
  change(input.value);
  if (state.writes.length !== 1 || state.rules.length !== 1) throw Error(`unexpected field registration/writes for ${input.type}`);
  observed.push({type: input.type, ...(input.scenario ? {scenario: input.scenario} : {}), ...(input.props.mode ? {mode: input.props.mode} : {}), ...state.writes[0]});
}
const fixture = new URL('./field-observations.json', import.meta.url);
if (process.argv.includes('--write')) writeFileSync(fixture, JSON.stringify(observed, null, 2) + '\n');
else if (JSON.stringify(JSON.parse(readFileSync(fixture, 'utf8'))) !== JSON.stringify(observed)) throw Error('field observations differ');
console.log(`${observed.length} upstream field callbacks observed`);

} finally { rmSync(temporary, {recursive: true, force: true}); delete (globalThis as any).__fieldProbe; }
