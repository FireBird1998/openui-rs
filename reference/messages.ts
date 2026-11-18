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

// The wire answer offers an exact alternate only for the core's reserved
// scalar collision in serialized read results. Keep written text, exception
// kinds, ordering and every other output byte in the comparison.
const reservedScalar = /[\u{10F800}-\u{10FFFF}]/u;

function replaceReservedScalars(output: string) {
  return output.replace(/[\u{10F800}-\u{10FFFF}]/gu, scalar =>
    "\\u" + (scalar.codePointAt(0)! - 0x10f800 + 0xd800).toString(16));
}

function projectReadOutput(output: string) {
  if (!reservedScalar.test(output)) return output;
  const { content, context } = JSON.parse(output);
  const contextJson = JSON.stringify(context);
  if (!reservedScalar.test(contextJson)) return replaceReservedScalars(output);

  // The core's JSON reader joins surrogate halves in context strings and
  // keys. Reparse only the context after substitution to reproduce that join
  // and any duplicate key it creates. Attributes use the unpaired units.
  const prefix = '{"content":' + JSON.stringify(content) + ',"context":';
  const throughContext = prefix + contextJson;
  if (!output.startsWith(throughContext)) return output;
  return replaceReservedScalars(prefix)
    + JSON.stringify(JSON.parse(replaceReservedScalars(contextJson)))
    + replaceReservedScalars(output.slice(throughContext.length));
}

function projectRead(read: ReturnType<typeof readMessage>) {
  return {
    complete: projectReadOutput(read.complete),
    contextNumber: read.contextNumber,
    cuts: read.cuts.map(projectReadOutput),
  };
}

export function answerMessageForFuzz(req: MessageRequest) {
  const result = answerMessage(req);
  const answer = JSON.stringify(result);
  if (!reservedScalar.test(answer)) return { answer };
  const projected = "complete" in result
    ? projectRead(result)
    : "read" in result && result.read
      ? { ...result, read: projectRead(result.read) }
      : result;
  const alternate = JSON.stringify(projected);
  return alternate === answer ? { answer } : { answer, reservedScalarCollision: alternate };
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
