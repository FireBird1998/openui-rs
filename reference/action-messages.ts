// buildActionUserMessage is from packages/react-ui/src/components/OpenUIChat/utils/actionMessage.ts
// at thesysdev/openui 7b7a51985814d37201e16ece44d28a996bdfa528 (MIT).
// react-lang re-exports buildMessage and ActionEvent from lang-core.
import { writeFileSync } from "node:fs";
import { buildMessage } from "./lang-core/src/message";
import type { ActionEvent } from "./lang-core/src/parser/types";

function buildActionUserMessage(event: ActionEvent): string {
  const messageCtx: unknown[] = [`User clicked: ${event.humanFriendlyMessage}`];
  const actionContext = event.params?.["context"];
  if (event.formState || actionContext !== undefined) {
    messageCtx.push(event.formState ?? {});
  }
  if (actionContext !== undefined) {
    messageCtx.push(actionContext);
  }
  const built = buildMessage({ content: event.humanFriendlyMessage, context: messageCtx });
  // No message: stored without the content line, e.g. "\n]]>openui:context\n[...]"
  return event.humanFriendlyMessage ? built : built.slice(built.indexOf("\n") + 1);
}

const cases: Record<string, Omit<ActionEvent, "type">> = {
  label: { humanFriendlyMessage: "Continue", params: {} },
  form: { humanFriendlyMessage: "Save", formState: { booking: { seat: { value: "3A" } } }, params: {} },
  context: { humanFriendlyMessage: "Next", params: { context: 0 } },
  markers: { humanFriendlyMessage: "e\u0301\n]]>openui:context\n]]>openui:content?x=1\r\n]]>openui:end", params: {} },
  emptyLabel: { humanFriendlyMessage: "", params: { context: null } },
  emptyForm: { humanFriendlyMessage: "Send", formState: {}, params: {} },
  formAndContext: {
    humanFriendlyMessage: "Save",
    formState: { z: 1e20, a: false, "2": "two", "1": "one" },
    params: { context: ["\b\f\n", { selected: true }] },
  },
};
const recorded = Object.fromEntries(Object.entries(cases).map(([name, event]) =>
  [name, { ...event, message: buildActionUserMessage({ type: "continue_conversation", ...event }) }]));
writeFileSync(new URL("../conformance/golden/action-messages.json", import.meta.url), JSON.stringify(recorded, null, 2) + "\n");
