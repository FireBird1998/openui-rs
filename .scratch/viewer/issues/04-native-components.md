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

**Status:** ready-for-agent

- [ ] Each listed component is drawn natively, following its props as upstream's example library describes them.
- [ ] Child parts are drawn by their parents, not as boxes of their own.
- [ ] Components the registry doesn't have still draw as labeled boxes.
- [ ] An element whose props are missing or of the wrong type falls back to a labeled box instead of crashing.
- [ ] A sample program that uses every native component opens from the viewer's menu.
- [ ] A Swift test draws every corpus case's final result offscreen without crashing.
