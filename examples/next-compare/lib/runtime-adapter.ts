// The renderer's evaluateRoot seam. Parser JSON stays unchanged for the
// comparison recorder; Rust owns evaluation for results made by its parser.
import type { ParseResult, EvalContext, ElementNode } from "@openuidev/lang-core";
import { RuntimeSession, StreamSession, type ParsedProgram } from "../wasm/openui_wasm.js";

type Evaluate = (context: EvalContext) => ElementNode | null;
const evaluations = new WeakMap<object, Evaluate>();
const rootEvaluations = new WeakMap<object, Evaluate>();
export const runtimeCalls = { rust: 0 };
const cleanup = new FinalizationRegistry<{runtime: RuntimeSession; stream: StreamSession}>(
  ({runtime, stream}) => { runtime.free(); stream.free(); },
);

export function retainRuntime(parser: object, runtime: RuntimeSession, stream: StreamSession): void {
  cleanup.register(parser, {runtime, stream});
}

export function attachRuntime(result: ParseResult, runtime: RuntimeSession, program: ParsedProgram): void {
  const evaluate: Evaluate = (context) => {
    runtime.setProgram(program);
    // Upstream owns renderer interactions and fetching. Supply its current
    // snapshot and query results to Rust before drawing the same screen.
    runtime.initialize(JSON.stringify(context.store?.getSnapshot() ?? {}));
    const references: Record<string, unknown> = {};
    for (const query of [...(result.queryStatements ?? []), ...(result.mutationStatements ?? [])]) {
      const value = context.ctx.resolveRef(query.statementId);
      if (value !== undefined) references[query.statementId] = value;
    }
    runtime.setReferenceValues(JSON.stringify(references));
    const output = runtime.evaluate();
    try {
      const tree = JSON.parse(output.json) as {root: ElementNode | null; errors: NonNullable<EvalContext["errors"]>};
      context.errors?.push(...tree.errors);
      runtimeCalls.rust++;
      return tree.root;
    } finally { output.free(); }
  };
  evaluations.set(result, evaluate);
  // Also support renderers that shallow-copy just root/rootExpression.
  if (result.root) rootEvaluations.set(result.root, evaluate);
  if (result.rootExpression) rootEvaluations.set(result.rootExpression, evaluate);
}

export function rustEvaluation(result: Pick<ParseResult, "root" | "rootExpression">, context: EvalContext): {root: ElementNode | null} | undefined {
  const evaluate = evaluations.get(result)
    ?? (result.root ? rootEvaluations.get(result.root) : undefined)
    ?? (result.rootExpression ? rootEvaluations.get(result.rootExpression) : undefined);
  return evaluate ? {root: evaluate(context)} : undefined;
}
