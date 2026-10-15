# 04 — Native components

**What to build:** Screens that look like screens. A registry maps component names to native SwiftUI views. These come from upstream's example library and are among its most used in the corpus:
- text content, stack, card and card header;
- button and button group;
- table with its columns;
- form, form control, input, and select with its items;
- line, bar and pie charts with their series and slices, drawn with Swift Charts;
- tabs with their items, tag and callout.

Child parts (columns, select items, series, slices, tab items) are drawn by their parent. Every other component keeps the labeled-box fallback. Controls look real but do nothing.

**Blocked by:** 01 — One corpus case, drawn as boxes

**Status:** done (2026-10-15)

- [x] Each listed component is drawn natively, following its props as upstream's example library describes them.
- [x] Child parts are drawn by their parents, not as boxes of their own.
- [x] Components the registry doesn't have still draw as labeled boxes.
- [x] An element whose props are missing or of the wrong type falls back to a labeled box instead of crashing.
- [x] A sample program that uses every native component opens from the viewer's menu.
- [x] A Swift test draws every corpus case's final result offscreen without crashing.

## Comments

**2026-10-15, done.** `scripts/check.sh` and `scripts/check-viewer.sh` pass, with 27 Swift tests. The new tests:
- every corpus case's final result draws offscreen without crashing;
- the gallery parses with no errors, contains every native component, and none of its elements falls back;
- props of the wrong type fall back to a box;
- a missing prop falls back only once the element is complete;
- dynamic text is drawn as its source;
- two loads started back to back end in the second one's state.

Looked at in the real window: the gallery's header, the three card variants, the tag, the line, grouped bar and donut charts with legends, the table with right-aligned numbers and action buttons, the form's field and buttons, switchable tabs with wrapping tags, and the callouts. The form's select menu sat below the fold and wasn't looked at. The phone-width drawing wasn't looked at either, because the screen was locked by then.

**How it works:**
- **`ElementView` draws every element.** It asks `NativeComponents` for a native view and otherwise draws the labeled box, which in turn draws its own children through `ElementView`. Native views keep the selection, the scroll target and the dashed "still arriving" border.
- **`PropReader` decides what draws.** A prop a view needs must be there and of the right type, or the element falls back. A style prop with an unknown value uses its default. An element that's still arriving reads a missing prop as empty, so it doesn't flip between box and native view mid-stream.
- **Paths stay the same.** Children sit at the same paths as in the inspector's tree, such as `/children/2` and `/items/1/content/0`.
- **Dynamic text.** A prop that's an expression is drawn as its source in italics.
- **One more than listed.** `TextCallout` shares the callout view, so 18 component names are drawn natively. A wrapping row (`wrap: true`) uses a small flow layout.
- **Drawing width.** The drawing now scrolls only vertically and fits a width: the window, a phone (390 pt) or a tablet (820 pt). That setting is in the playback bar.
- **The gallery.** `File > Open Component Gallery` (⇧⌘G) opens a 14-statement program against upstream's example library that uses every native component and parses with no errors. `VIEWER_PLAY=gallery` plays it in a scripted run, and an absolute path plays any program file.

**Two bugs found and fixed:**
- **Overlapping loads crashed the viewer.** Changing the chunking while a program was loading started a second load. It emptied the chunks while the old position stood, and the view read past the end. Loads now run one after another and swap their chunks in all at once, and the viewer skips reloads when the chunking hasn't changed. A test covers loads started back to back.
- **A layout loop crashed AppKit.** With scroll bars always shown, as on this Mac, a drawing just about the window's height could loop: the bar appears, text wraps narrower and grows taller, the bar goes, and so on, until AppKit gave up with "more Update Constraints in Window passes than there are views". Fallback boxes with long text in a row of cards, plus the form, reproduced it 3 times out of 3. The drawing's width now comes from the pane, less room for a scroll bar, never from the space beside the bar. The same program then ran 5 out of 5 times, and so did the full gallery with its charts drawn as boxes.

**Speed, release build, at 1 code point per chunk:**

| | redraw median | worst | longest stall |
|---|---:|---:|---:|
| gallery | 96 ms | 131 ms | 157 ms |
| `bench/dashboard` | 85 ms | 116 ms | 136 ms |
| `synthetic/dashboard-x8` | 93 ms | 163 ms | 165 ms |

Native drawing is slower than the boxes were, which redrew in about 50 ms.
- **Not the charts.** Drawing them as boxes left `bench/dashboard` at 106 ms.
- **Not rebuilding the views.** Making element views `Equatable`, so SwiftUI skips unchanged subtrees, didn't move the numbers, so it was taken out again.

The time goes into layout and drawing, which is ticket 05's question.
