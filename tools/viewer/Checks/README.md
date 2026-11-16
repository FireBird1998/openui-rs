# Offline Swift compatibility observations

The reference is lang-core 1.1.0 and react-ui 0.18.1 at upstream commit
`47b9cae0d3b26a8c9d7514d5887a6d6608801ac9`. No installation is needed.
From `reference/`:

```sh
bun --no-install ../tools/viewer/Checks/validation-reference.ts
bun --no-install ../tools/viewer/Checks/fields-reference.ts /path/to/local/upstream-checkout
```

`--write` regenerates either adjacent JSON fixture. The validation probe calls
vendored `validate(value, parseStructuredRules(rules))`. The field probe reads
only that commit's Git objects with lazy fetching disabled, substitutes inert
React hooks and visual components, and invokes the actual genui callback. It
uses the vendored `resolveStateField`; it does not render a browser or test
React's scheduling. Temporary transpiled files are removed on exit. The Swift
public-interface tests consume both fixtures, without needing the checkout.

The expected validation answers include all six named URLs, ASCII `\w`/`\d`
and word boundaries, JavaScript `\s` (which is Unicode even without flags),
flagless `\p{L}`, `[]a]`, `[[a]]`, and ignored invalid `(?i)` patterns. react-ui
calls `parseStructuredRules` and registers those rules; lang-core's pattern
validator catches `new RegExp(arg)` syntax errors and returns no error.

Structured `pattern` is a string passed to `new RegExp` with **no flags**.
There is no representable `u`/`v` flag in this interface. A string such as
`/pattern/u` is literal pattern text, not a flag request. ICU remains the
matcher: astral code points versus JavaScript UTF-16 code units, case folding,
some identity escapes, backreferences, and engine-specific syntax are still
outside the compatibility claim. URL checks do not implement full WHATWG
IDNA, IPv6 validation or normalization. The fixtures pin acceptance, not URL
serialization. `validation-remainders.json` also pins four concrete remaining
differences in public native form tests: `^.$` with an astral character, ICU's
`\z` escape (a literal `z` in flagless JavaScript), a percent-encoded
no-break-space in a special host that still needs IDNA rejection, and ICU
case folding of the Kelvin sign inside a scoped case-insensitive word class.
Valid scoped modifiers such as `(?i:a)` remain supported; bare `(?i)` is ignored
as an invalid JavaScript pattern.

The upstream FormControl union enumerates these nine field components.
`Input` and `Select` already had native views on main; the other seven now do.

| Component | System control | Stored value / default behavior |
| --- | --- | --- |
| Input | TextField / SecureField (existing) | String; `value` binding or field |
| Select | Picker (existing) | String; `value` binding or field |
| TextArea | TextEditor | String; placeholder and rows; `value` binding or field |
| RadioGroup | Radio Picker | String; `items: RadioItem[]`; display falls back to `defaultValue` |
| CheckBoxGroup | Checkbox Toggles | Record of item names to booleans; each item falls back to `defaultChecked` |
| Slider | Slider per array entry | Number array; display falls back to `defaultValue` or `[min]`; sorted on interaction |
| DatePicker | DatePicker per endpoint | Date's JSON ISO-8601 UTC string; range `{from,to}` strings |
| Chips | Row of button Toggles | Single string/undefined or string array; defaults hydrated; `componentType: "Chips"` |
| OptionCards | Checkbox rows | Single string/undefined or string array; defaults hydrated; `componentType: "OptionCards"` |

The first seven write unbound fields without component metadata, through
`useStateField` upstream. The binding props are marked by the imported prompt
spec. Chips/OptionCards use named fields rather than a `value` binding upstream.
Their invalid stored shapes can display a normalized choice while required
validation rejects the stored shape; the native controls preserve this quirk.
Radio/Slider visual defaults and checkbox item defaults do not by themselves
satisfy required validation of a missing field. Chips/OptionCards defaults are
hydrated with the runtime's missing-field guard when not streaming.

All controls are disabled while streaming. Chips/OptionCards also honor item
`disabled`; no group-level disabled prop exists in these upstream schemas.
Slider's discrete default step is one. Round 2 explicitly preserves a supplied
fractional continuous step, and an absent continuous step has no native snapping;
continuous tracks never receive a SwiftUI step. The pinned SliderBlock wrapper
actually clamps continuous steps to one (the primitive Slider passes them
through); the explicit Round 2 instruction governs this intentional correction.
A discrete track gets a SwiftUI step only when it fits the span and produces at most 1,000 positions,
including endpoints. Larger grids use a tickless track and constant-space
min-relative snapping on write. Platform position allocation is bounded by the fixed cap, regardless of
the numeric range. At most 16 thumbs are drawn; larger arrays remain unchanged
and show a read-only notice. Invalid/reversed/equal/nonfinite bounds, a nonfinite
span, a nonfinite reciprocal span and nonpositive/nonfinite supplied steps use the unusable-props fallback.
List-driven controls have no view-level count limit: children, fields, buttons,
tabs, options, rows, columns, padded cells and chart points retain every entry.
Their data is bounded by the runtime’s program/result/work/depth limits. Option
controls use lazy viewports (240 points vertically, 44 horizontally); long viewer
fallback collections also use lazy stacks. TextArea rows clamp only its viewport
to 1...20 rows. The slider position and thumb bounds above remain in force.
An empty number array still draws one slider so it can be filled. The older
bench gallery schema uses scalar Slider defaults; those also draw one slider.
Duplicate checkbox names share one aggregate and the last item's default,
as upstream's record construction does. Date range writes keep `from,to` order,
including when an existing start date is edited. Dates use the system calendar control; absent values display the epoch until
edited and remain missing for validation. A picked day is normalized to local
midnight in the active calendar/time zone before ISO UTC serialization; the
non-UTC callback observation pins Asia/Kolkata midnight. Option icons/images, card decoration,
and react-ui's extra Slider text inputs/menus are deliberately omitted.

Validation rules, callbacks, real AppKit control interactions, changed control
pixels after a value update, app replacement hooks, and form event shapes are
checked by `Issue71Tests.swift`. `aRequiredRadioDefaultCanBeChosenOnScreen`
checks that even a sole already-displayed radio default can satisfy required
validation through a click.


Offline drawing differential (requires the already-built binding and local SDK):

```sh
python3 tools/viewer/Checks/compare-native-rendering.py --baseline main
```

The viewer suite also runs this as
`RenderingCompatibility.everyCorpusAndBenchmarkProgramPreservesMainDrawing`
(about 45 seconds on this machine, with no timing assertion). It requires the
repository’s local `main` ref, corpus and already-built binding. No installs, networking or checkout changes are performed.
It extracts main's Swift package sources into a temporary package, copies the
current sources into a second package, and links both against the same existing
Rust binding. It reads exactly the five manifests used by `Corpus.load` (972
programs, including all seven benchmark cases and the timing dashboard/x2/x8),
plus both main's and the current component gallery, feeding identical programs
to both packages. Every program is tested parsed and evaluated.

Each element, including offscreen children and hidden tabs, is separately
materialized with its actual props, live state and enclosing form using a real
800x600 `ImageRenderer`. Descendants are rendered separately to avoid ancestor
multiplication and mistaking lazy visibility for omission. Instrumentation in
these temporary source copies records native/fallback acceptance, text
construction, field values/placeholders, chart titles and chart data labels.
The taps return the original SwiftUI types: no custom layout/view wrappers are
introduced. Chart SDK automatic axis-tick elision is not compared as text;
the data labels supplied to Charts are. This is a text/acceptance differential,
not a pixel or interactive-event comparison.

It rejects any native-to-fallback change, unexpected native gain, error mismatch,
node-inventory mismatch or text mismatch for unchanged acceptance. Only the seven
new field controls may gain native rendering; their text is recorded but cannot
match main's fallback text. Standalone results, exact source hashes, JSONL observations and
build/run logs are saved in `target/swift-71-r3-differential/`; the suite uses
`target/swift-71-r3-differential-suite/`. `summary.json`
is the machine-readable result; a nonzero exit means a regression.
