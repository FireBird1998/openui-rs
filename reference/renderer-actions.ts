// Transcribed from @openuidev/react-lang 1.1.0 (MIT), dist/index.mjs,
// src/hooks/useOpenUIState.ts: built-file lines 570–574, 716–824,
// and the default-persistence condition at line 241 (hook lines 237–252).
// Reference copy: .agent/react-lang-1.1.0-index.mjs; licence: reference/react-lang-LICENSE.
// React useCallback wrappers/dependency lists are removed; evaluate can be
// injected to use the harness's fresh-query-manager-per-operation policy. evaluate,
// store, builtin constants and ownership filtering remain lang-core's code.
// The caller supplies queryManager: tool work is an explicit host handoff in
// this harness; the transcription itself retains the original await/return.
import { ACTION_STEPS as ACTION_STEPS$1 } from './lang-core/src/parser/builtins';
import { BuiltinActionType as BuiltinActionType$1 } from './lang-core/src/parser/types';
import { evaluate as upstreamEvaluate } from './lang-core/src/runtime/evaluator';

export function unwrapFieldValue(v) {
	if (v && typeof v === "object" && !Array.isArray(v) && "value" in v) return v.value;
	return v;
}

export function rendererState({store, evaluationContext, schema, resultRef, queryManager, propsRef, bundleBlocked=false, evaluate=upstreamEvaluate}) {
	const getFieldValue = ((formName, name) => {
		if (!formName) return unwrapFieldValue(store.get(name));
		const formData = store.get(formName);
		if (!formData || typeof formData !== "object" || Array.isArray(formData)) return void 0;
		return unwrapFieldValue(formData[name]);
	});
	const setFieldValue = ((formName, componentType, name, value, shouldTriggerSaveCallback = true) => {
		const wrapped = {
			value,
			componentType
		};
		if (!formName) store.set(name, wrapped);
		else {
			const raw = store.get(formName);
			const formData = raw && typeof raw === "object" && !Array.isArray(raw) ? raw : {};
			store.set(formName, {
				...formData,
				[name]: wrapped
			});
		}
		if (shouldTriggerSaveCallback) propsRef.current.onStateUpdate?.(store.getSnapshot());
	});
	const getFormPayload = ((formName) => {
		if (formName) {
			const raw = store.get(formName);
			if (raw && typeof raw === "object" && !Array.isArray(raw)) return { [formName]: raw };
		}
		return store.getSnapshot();
	});
	const triggerAction = (async (userMessage, formName, action) => {
		if (bundleBlocked) return;
		const { onAction: handler } = propsRef.current;
		if (action && !("steps" in action)) {
			const formPayload = getFormPayload(formName);
			const actionType = action.type || BuiltinActionType$1.ContinueConversation;
			if (schema.actions && Object.prototype.hasOwnProperty.call(schema.actions, actionType)) return;
			const params = { ...action.params || {} };
			if (action.url) params.url = action.url;
			if (action.context) params.context = action.context;
			handler?.({
				type: actionType,
				params,
				humanFriendlyMessage: userMessage,
				formState: formPayload,
				formName
			});
			return;
		}
		const actionPlan = action;
		if (actionPlan?.steps) {
			for (const step of actionPlan.steps) switch (step.type) {
				case ACTION_STEPS$1.Run:
					if (step.refType === "mutation") {
						const mn = resultRef.current?.mutationStatements?.find((m) => m.statementId === step.statementId);
						const evaluatedArgs = mn?.argsAST ? evaluate(mn.argsAST, evaluationContext) : {};
						if (!await queryManager.fireMutation(step.statementId, evaluatedArgs)) return;
					} else queryManager.invalidate([step.statementId]);
					break;
				case ACTION_STEPS$1.ToAssistant:
					handler?.({
						type: BuiltinActionType$1.ContinueConversation,
						params: step.context !== void 0 ? { context: step.context } : {},
						humanFriendlyMessage: step.message,
						formState: getFormPayload(formName),
						formName
					});
					break;
				case ACTION_STEPS$1.OpenUrl:
					handler?.({
						type: BuiltinActionType$1.OpenUrl,
						params: { url: step.url },
						humanFriendlyMessage: "",
						formState: getFormPayload(formName),
						formName
					});
					break;
				case "custom_action":
					handler?.({
						type: step.name,
						params: step.params,
						humanFriendlyMessage: "",
						formState: getFormPayload(formName),
						formName
					});
					break;
				case ACTION_STEPS$1.Set: {
					if (!step.valueAST) {
						console.warn(`[openui] Set action for ${step.target} has no valueAST — skipping`);
						break;
					}
					const value = evaluate(step.valueAST, evaluationContext);
					store.set(step.target, value);
					break;
				}
				case ACTION_STEPS$1.Reset: {
					const decls = resultRef.current?.stateDeclarations ?? {};
					for (const target of step.targets) store.set(target, decls[target] ?? null, { pristine: true });
					break;
				}
			}
			return;
		}
		handler?.({
			type: BuiltinActionType$1.ContinueConversation,
			params: {},
			humanFriendlyMessage: userMessage,
			formState: getFormPayload(formName),
			formName
		});
	});
	return {getFieldValue,setFieldValue,getFormPayload,triggerAction};
}

// useSetDefaultValue's effect body; React dependencies are outside this seam.
export function persistDefaultValue({isStreaming, existingValue, defaultValue,
 formName, componentType, name, shouldTriggerSaveCallback=false}, setFieldValue) {
 if (!isStreaming && existingValue === void 0 && defaultValue !== void 0) setFieldValue(formName, componentType, name, defaultValue, shouldTriggerSaveCallback);
}
