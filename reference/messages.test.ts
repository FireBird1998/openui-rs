import { expect, test } from "bun:test";
import corpus from "../conformance/core-divergences.json";
import pins from "../conformance/divergences/core-divergences.json";
import { answerMessageForFuzz, readMessage } from "./messages";

test("message projections match the independently pinned core results", () => {
  for (const { name, operation, input } of corpus.cases) {
    if (operation !== "message" || input === undefined) continue;
    const expected = pins[name as keyof typeof pins];
    const reply = answerMessageForFuzz({ op: "parseMessage", raw: input });
    expect(JSON.parse(reply.answer).complete).toBe(expected.reference.output);
    expect(JSON.parse(reply.reservedScalarCollision!).complete).toBe(expected.rust.output);
  }
});

test("the scalar immediately below the reserved range stays in the exact answer", () => {
  const raw = ']]>openui:context\n"\\uDBFD\\uDFFF"';
  const reply = answerMessageForFuzz({ op: "parseMessage", raw });
  expect(reply.answer).toBe(JSON.stringify(readMessage(raw)));
  expect(reply.reservedScalarCollision).toBeUndefined();
});
