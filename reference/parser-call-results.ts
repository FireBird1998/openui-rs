// From reference/: bun --no-install parser-call-results.ts
import {writeFileSync} from "node:fs";

process.env.OPENUI_TELEMETRY_DISABLED = "1";
process.env.DO_NOT_TRACK = "1";
const {compileSchema, parse} = await import("./lang-core/src/parser/parser");

const cases = ([
  ["empty-ref-string", {$ref: "", type: "string"}],
  ["empty-def-name-string", {$ref: "#/$defs/", type: "string"}],
  ["trailing-slash-enum", {$ref: "#/$defs/X/", enum: ["a", "b"]}],
  ["empty-ref-only", {$ref: ""}],
] as const).map(([name, v]) => ({
  name, program: "root = Box()", schema: {$defs: {Box: {properties: {v}, required: ["v"]}}},
}));

// The action schema from #202, including its string-valued required list.
const action = {
  name: "nested-action",
  program: "root = Col(@S0([@S0([@S0( [ [@S0(1, [1, 1,1], @S0(1,1,1)), @S0(1, [1, 1], @S0(1,1,1))],1], [@S0(1,1, 1)]), @S0( [ [@S0(1, [1, 1], @S0(1,1,1)), @S0(1, [1, 1,1], @S0(1,1,1))]], [@S0()])]), @S0([@S0( [ [@S0(1, [1, 1,1], @S0(1,1,1)), @S0(1, [1, 1,1], @S0(1,1,1))]], [@S0(1,1, 1)])])], []),1)",
  schema: {
    $defs: {
      Col: {properties: {action: {}, label: {}}},
      S0: {$ref: "#/$defs/OpenUrl"},
      OpenUrl: {
        type: "object",
        properties: {
          type: {const: "open_url"},
          params: {type: "object", properties: {url: {type: "string"}}, required: ["url"]},
        },
        required: ["type", "params"],
      },
      ActionExpression: {anyOf: [{$ref: "#/$defs/OpenUrl"}, {$ref: "#/$defs/ToAssistant"}]},
    },
    actions: {S0: {params: {
      type: "object",
      properties: {
        p: {type: "array", items: {items: {anyOf: [{$ref: "#/$defs/ActionExpression"}, {type: "string"}]}, type: "array"}},
        q: {type: "array", items: {const: {$ref: "#/$defs/S0"}}},
        r: {type: "array", items: {$ref: "#/$defs/OpenUrl"}, default: ["d"]},
      },
      required: "pq",
    }}},
  },
};
const rows = [...cases, action].map(({name, program, schema}) => {
  const expected = JSON.stringify(parse(program, compileSchema(schema as Parameters<typeof compileSchema>[0])));
  return JSON.stringify({name, program, schema, expected});
});
writeFileSync(new URL("../conformance/golden/parser-call-results.jsonl", import.meta.url), rows.join("\n") + "\n");
