// MIT, thesysdev/openui 47b9cae0, packages/react-lang/src/hooks/useOpenUIState.ts
// lines 235–245. The driver supplies React's dependency changes and blocked gate.
import { evaluate } from './lang-core/src/runtime/evaluator';
export function registerMutations(blocked, result, evaluationContext, queryManager) {
  if (blocked) return;
  const mutStmts = result?.mutationStatements ?? [];
  const nodes = mutStmts.map((mn) => ({
    statementId: mn.statementId,
    toolName: mn.toolAST ? evaluate(mn.toolAST, evaluationContext) : "",
  }));
  queryManager.registerMutations(nodes);
}
