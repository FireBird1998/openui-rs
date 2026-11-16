#!/usr/bin/env python3
"""Offline main/current native acceptance + rendered text differential.

Run after scripts/swift-binding.sh:
  python3 tools/viewer/Checks/compare-native-rendering.py [--baseline main]

Builds isolated package-source copies with the SAME existing Rust binding and
SDK. No installs, downloads, checkout changes or Rust edits. See Checks/README.md.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[3]
CHECKS = Path(__file__).resolve().parent
MANIFESTS = ('manifest.json', 'imported.json', 'fuzz.json', 'synthetic.json', 'recorded.json')
NEW_CONTROLS = {'TextArea', 'RadioGroup', 'CheckBoxGroup', 'Slider', 'DatePicker', 'Chips', 'OptionCards'}


def git(*args):
    return subprocess.check_output(['git', '-C', str(ROOT), *args])


def dataset(baseline):
    corpus = ROOT / 'conformance'
    schemas, cases = {}, []
    for name in MANIFESTS:
        manifest = json.loads((corpus / name).read_text())
        schemas.update(manifest.get('schemas', {}))
        cases += manifest['cases']
    bindings = json.loads((corpus / 'bindings.json').read_text())
    samples = []
    for case in cases:
        name = case['schema']
        spec_path = corpus / bindings.get(name, f'prompts/{name}-schema.json')
        samples.append(dict(name=case['name'], schema=(corpus / schemas[name]).read_text(),
                            program=(corpus / case['file']).read_text(), root=case.get('root'),
                            spec=spec_path.read_text() if spec_path.exists() else None,
                            source=case.get('source', '')))
    # viewer-timing.sh's dashboard/x2/x8 samples are already in the corpus.
    # Include BOTH gallery programs, each fed identically to both packages.
    for version in ('main', 'current'):
        path = 'tools/viewer/Sources/ViewerKit/Gallery.swift'
        source = git('show', f'{baseline}:{path}').decode() if version == 'main' else (ROOT / path).read_text()
        match = re.search(r'static let program = #"""\n(.*?)\n"""#', source, re.S)
        if not match:
            raise RuntimeError('gallery program not found')
        spec_path = corpus / 'prompts/bench-schema.json'
        samples.append(dict(name=f'audit/gallery-{version}', schema=(corpus / schemas['bench']).read_text(),
                            program=match[1], root=None, spec=spec_path.read_text() if spec_path.exists() else None,
                            source='benchmark-gallery'))
    return samples


INSTRUMENTATION = r'''
import SwiftUI

/// Test-copy-only constructor taps. They return the original SwiftUI types;
/// no proxy View or extra layout node changes how the component is drawn.
@MainActor public enum RenderingAudit {
    static var active = false
    static var text: [String: Int] = [:]
    static func record(_ kind: String, _ value: String) {
        if active { text[kind + ":" + value, default: 0] += 1 }
    }
    public static func capture(_ work: () -> Void) -> [String: Int] {
        text = [:]; active = true
        defer { active = false }
        work()
        return text
    }
}
@MainActor public func auditText(_ value: String) -> Text {
    RenderingAudit.record("text", value)
    return Text(value)
}
@MainActor func auditButton(_ value: String, action: @escaping () -> Void) -> Button<Text> {
    RenderingAudit.record("button", value)
    return Button(value, action: action)
}
@MainActor func auditTextField(_ title: String, text: Binding<String>) -> TextField<Text> {
    RenderingAudit.record("input", text.wrappedValue.isEmpty ? title : text.wrappedValue)
    return TextField(title, text: text)
}
@MainActor func auditSecureField(_ title: String, text: Binding<String>) -> SecureField<Text> {
    RenderingAudit.record("secure-input-length", String(text.wrappedValue.count))
    return SecureField(title, text: text)
}
@MainActor func auditString(_ value: String) -> String {
    RenderingAudit.record("axis-title", value)
    return value
}
@MainActor func auditPoint(_ value: ChartPoint) -> ChartPoint {
    RenderingAudit.record("chart-series", value.series)
    RenderingAudit.record("chart-label", value.label)
    RenderingAudit.record("chart-value", JSONValue.format(value.value))
    return value
}
'''


def instrument(source):
    source = re.sub(r'\bText\(', 'auditText(', source)
    source = source.replace('Button(s)', 'auditButton(s)')
    source = source.replace('TextField(placeholder,', 'auditTextField(placeholder,')
    source = source.replace('SecureField(placeholder,', 'auditSecureField(placeholder,')
    source = source.replace('.chartXAxisLabel(x)', '.chartXAxisLabel(auditString(x))')
    source = source.replace('.chartYAxisLabel(y)', '.chartYAxisLabel(auditString(y))')
    # Capture all supplied chart labels/values, including SDK-generated legend
    # and axis text inputs, without relying on automatic tick suppression.
    source = re.sub(r'points.append\((ChartPoint\([^\n]+\))\)', r'points.append(auditPoint(\1))', source)
    source = re.sub(r'return (ChartPoint\([^\n]+\))', r'return auditPoint(\1)', source)
    return source


def stage(where, baseline=None):
    native = 'swift/OpenUINative/Sources/OpenUINative'
    paths = git('ls-tree', '-r', '--name-only', baseline, native).decode().splitlines() if baseline else [str(p.relative_to(ROOT)) for p in (ROOT / native).glob('*.swift')]
    (where / 'Native').mkdir(parents=True)
    (where / 'Probe').mkdir()
    for path in paths:
        source = git('show', f'{baseline}:{path}').decode() if baseline else (ROOT / path).read_text()
        (where / 'Native' / Path(path).name).write_text(instrument(source))
    (where / 'Native' / 'RenderingAudit.swift').write_text(INSTRUMENTATION)
    for name in ('ElementBox.swift', 'Selection.swift'):
        path = f'tools/viewer/Sources/ViewerKit/{name}'
        source = git('show', f'{baseline}:{path}').decode() if baseline else (ROOT / path).read_text()
        # Keep the child-suppression drawing hook when probing a fallback node.
        source = source.replace('.environment(' + chr(92) + '.elementDrawing, .inspecting)', '')
        (where / 'Probe' / name).write_text(instrument(source))
    (where / 'Probe' / 'render-audit.swift').write_text((CHECKS / 'render-audit.swift').read_text())
    (where / 'FFI').symlink_to(ROOT / 'swift/OpenUINative/Sources/OpenUIFFI', target_is_directory=True)
    (where / 'Frameworks').symlink_to(ROOT / 'swift/OpenUINative/Frameworks', target_is_directory=True)
    (where / 'Package.swift').write_text('''// swift-tools-version: 6.0
import PackageDescription
let package = Package(name: "RenderingAudit", platforms: [.macOS("27.0")], targets: [
 .binaryTarget(name: "openui_ffiFFI", path: "Frameworks/openui_ffiFFI.xcframework"),
 .target(name: "OpenUIFFI", dependencies: ["openui_ffiFFI"], path: "FFI", swiftSettings: [.swiftLanguageMode(.v5)]),
 .target(name: "OpenUINative", dependencies: ["OpenUIFFI"], path: "Native"),
 .executableTarget(name: "RenderProbe", dependencies: ["OpenUINative"], path: "Probe")])
''')


def compare(before, after):
    old = {(r['name'], r['mode']): r for r in before}
    new = {(r['name'], r['mode']): r for r in after}
    if old.keys() != new.keys():
        raise RuntimeError('program/mode inventories differ')
    regressions, mismatches, improvements, errors = [], [], [], []
    elements = text_nodes = 0
    for key, previous in old.items():
        current = new[key]
        if previous.get('error') != current.get('error'):
            errors.append(dict(case=key, before=previous.get('error'), after=current.get('error')))
        pn = {n['path']: n for n in previous['nodes']}
        cn = {n['path']: n for n in current['nodes']}
        if pn.keys() != cn.keys():
            errors.append(dict(case=key, missing=sorted(pn.keys()-cn.keys()), extra=sorted(cn.keys()-pn.keys())))
        for path in pn.keys() & cn.keys():
            a, b = pn[path], cn[path]
            elements += 1
            if a['native'] and not b['native']:
                regressions.append(dict(case=key, path=path, type=a['type']))
            if not a['native'] and b['native']:
                improvements.append(dict(case=key, path=path, type=b['type']))
                if b['type'] not in NEW_CONTROLS:
                    errors.append(dict(case=key, path=path, unexpected_native_gain=b['type']))
            elif a['text'] != b['text']:
                mismatches.append(dict(case=key, path=path, type=a['type'], before=a['text'], after=b['text']))
            else:
                text_nodes += 1
    return dict(programs=len(old)//2, modes=len(old), compared_elements=elements,
                identical_text_nodes=text_nodes, native_gains=len(improvements),
                programs_gaining_fallback=len({tuple(r['case'])[:1] for r in regressions}),
                native_regressions=regressions, text_mismatches=mismatches, errors=errors,
                improvements=improvements)


def main():
    args = argparse.ArgumentParser(description=__doc__)
    args.add_argument('--baseline', default='main')
    args.add_argument('--output', type=Path, default=ROOT/'target/swift-71-r3-differential')
    options = args.parse_args()
    options.output.mkdir(parents=True, exist_ok=True)
    samples = dataset(options.baseline)
    inputs = options.output/'programs.json'
    inputs.write_text(json.dumps(samples))
    snapshots = []
    env = {**os.environ, 'CLANG_MODULE_CACHE_PATH':str(ROOT/'target/clang-cache'),
           'SWIFTPM_MODULECACHE_OVERRIDE':str(ROOT/'target/swift-cache')}
    with tempfile.TemporaryDirectory(prefix='openui-render-diff-', dir='/private/tmp') as temporary:
        for name, revision in [('main', options.baseline), ('current', None)]:
            directory = Path(temporary)/name
            stage(directory, revision)
            output = options.output/f'{name}.jsonl'
            with (options.output/f'{name}.log').open('w') as log:
                subprocess.run(['swift','run','--package-path',str(directory),'--disable-sandbox','RenderProbe',str(inputs),str(output)],env=env,stdout=log,stderr=subprocess.STDOUT,check=True)
            snapshots.append([json.loads(line) for line in output.read_text().splitlines()])
    summary = compare(*snapshots)
    summary['baseline_commit'] = git('rev-parse', options.baseline).decode().strip()
    summary['manifests'] = list(MANIFESTS)
    summary['corpus_programs'] = len(samples)-2
    summary['benchmark_programs_in_corpus'] = sum(s['source']=='benchmark' for s in samples)
    summary['gallery_programs'] = 2
    summary['current_source_sha256'] = {str(p.relative_to(ROOT)):hashlib.sha256(p.read_bytes()).hexdigest() for p in (ROOT/'swift/OpenUINative/Sources/OpenUINative').glob('*.swift')}
    for name in ('ElementBox.swift', 'Selection.swift'):
        path = ROOT/'tools/viewer/Sources/ViewerKit'/name
        summary['current_source_sha256'][str(path.relative_to(ROOT))] = hashlib.sha256(path.read_bytes()).hexdigest()
    (options.output/'summary.json').write_text(json.dumps(summary,indent=2)+'\n')
    brief = {k:v for k,v in summary.items() if k not in {'improvements','current_source_sha256'}}
    print(json.dumps(brief,indent=2))
    if summary['native_regressions'] or summary['text_mismatches'] or summary['errors']:
        raise SystemExit(1)

if __name__ == '__main__': main()
