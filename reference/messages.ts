// Message fixtures, also used by the differential worker in serve.ts.
import { readFileSync, mkdirSync, writeFileSync } from "fs";
import { buildMessage, parseMessage, type BuildMessageInput } from "./lang-core/src/message";

export type MessageRequest =
  | { op: "parseMessage"; raw: string }
  | { op: "buildMessage"; build: BuildMessageInput };

export function readMessage(raw: string) {
  const parsed = parseMessage(raw);
  const contextNumber = typeof parsed.context === "number"
    ? Object.is(parsed.context, -0) ? "-0" : String(parsed.context)
    : null;
  return {
    complete: JSON.stringify(parsed),
    contextNumber,
    cuts: Array.from({ length: raw.length + 1 }, (_, cut) =>
      JSON.stringify(parseMessage(raw.slice(0, cut), { streaming: true }))),
  };
}

export function answerMessage(req: MessageRequest) {
  if (req.op === "parseMessage") return readMessage(req.raw);
  try {
    const text = buildMessage(req.build);
    return { text, read: readMessage(text) };
  } catch (error) {
    if (error instanceof URIError) return { threw: error.name };
    throw error;
  }
}

if (import.meta.main) {
  const base = new URL("../conformance/", import.meta.url);
  const cases = JSON.parse(readFileSync(new URL("messages.json", base), "utf8"));
  const outputs = Object.fromEntries(cases.map(({ name, raw, build }) => [
    name, answerMessage(build === undefined ? { op: "parseMessage", raw } : { op: "buildMessage", build }),
  ]));
  mkdirSync(new URL("golden/", base), { recursive: true });
  writeFileSync(new URL("golden/messages.json", base), JSON.stringify(outputs, null, 2) + "\n");
  console.log(`${cases.length} message cases`);
}
