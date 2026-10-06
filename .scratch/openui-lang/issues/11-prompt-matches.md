# 11 — System prompt matches the reference

**What to build:** Prompt generation. It takes a prompt spec in the reference parser's JSON shape (components, groups, tools, examples, rules, edit, inline, tool-call and binding modes) and produces the same system prompt string as the reference `generatePrompt` and `generateSystemPrompt`. The test corpus has prompt specs lifted from the reference prompt tests plus specs built from the benchmark schema.

**Blocked by:** 01 — Walking skeleton.

**Status:** ready-for-agent

- [ ] Every prompt spec in the corpus produces exactly the reference prompt string.
- [ ] Every mode combination (tools, no tools, edit mode, inline mode, bindings on and off) has a case.
- [ ] `openui-lang prompt <spec.json>` prints the prompt.
