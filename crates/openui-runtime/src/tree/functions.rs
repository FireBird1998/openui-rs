//! Synchronous embedder implementations paired with original schemas.
use super::*;

/// A synchronous function's outcome. None is JavaScript undefined, distinct
/// from explicit `Some(Value::Null)`. Err supplies the thrown error's message.
pub type FunctionResult = Result<Option<Value>, String>;
type Implementation<'a> = dyn Fn(Value) -> FunctionResult + Send + Sync + 'a;

/// Library function definitions and their synchronous implementations.
///
/// Definitions are the original JSON Schema `functions` object, preserving
/// whole parameter schemas and return types. Only registered names are called.
/// Implementations receive owned, named, validated arguments (normally an
/// object). Own keys holding undefined are omitted, including nested keys:
/// this is the nested-undefined representation divergence, and implementations
/// enumerating keys can observe it. Containers remain shared when passed
/// through. Value retains its owned String representation. Stable supplied
/// arguments reuse a session-local named argument object: text is projected
/// once, including direct member/index reads, and unchanged validated objects
/// retain that allocation. The cache holds one current argument projection,
/// replacing it before a miss allocates another; unchanged validation entries
/// hold weak identities. At most one transformed validation result is retained
/// in addition. Both caches end with the operation.
/// Fresh computed text, transforms, and validation rebuilds still copy and
/// charge their produced bytes; returning a cloned String also makes a copy.
/// Returning the same shared argument container reuses its measured admission,
/// while every final output position still pays for its full serialized size.
/// Registering the same name replaces its implementation.
///
/// Host computation, allocation, and side effects are the embedder's
/// responsibility. Returned values undergo the session's bounded admission
/// and projection and never earn input allowance. Err becomes a diagnostic and
/// null. Host panics propagate; evaluator contexts restore during unwinding,
/// and evaluation leaves Store values and its version unchanged. Catching a host panic allows
/// subsequent evaluations to use that Store normally. Host side effects cannot
/// be rolled back by the runtime. Callbacks may read and evaluate the same
/// Store recursively; nested evaluator contexts restore on return and unwind.
/// A mutable Store operation cannot alias its active shared borrow in safe
/// Rust. Locks held by the embedder, recursive host calls and closure-captured
/// allocations are host responsibilities, outside the runtime allowance.
pub struct LibraryFunctions<'a> {
    pub(super) definitions: &'a openui_lang::JsonValue,
    implementations: BTreeMap<String, Box<Implementation<'a>>>,
    limits: Option<Limits>,
    name_bytes: usize,
}
impl<'a> LibraryFunctions<'a> {
    /// Pair implementations with the original library's `functions` object.
    /// Default registration imposes no new quota on host-supplied names.
    pub fn new(definitions: &'a openui_lang::JsonValue) -> Self {
        Self {
            definitions,
            implementations: BTreeMap::new(),
            limits: None,
            name_bytes: 0,
        }
    }
    /// Borrow definitions and set limits for retained implementation names.
    /// Construction does not walk schemas. Evaluation admits each used schema
    /// under the evaluation's own shared limits.
    pub fn with_limits(definitions: &'a openui_lang::JsonValue, limits: Limits) -> Self {
        Self {
            limits: Some(limits),
            ..Self::new(definitions)
        }
    }
    /// Register or replace a synchronous implementation. Names are literal.
    /// A name absent from the definitions is ignored during evaluation.
    /// Retained names cost their JSON string size plus one separator byte.
    /// Refusal leaves the previous implementation and registry total unchanged.
    pub fn register(
        &mut self,
        name: impl AsRef<str>,
        implementation: impl Fn(Value) -> FunctionResult + Send + Sync + 'a,
    ) -> Result<(), crate::LimitError> {
        let name = name.as_ref();
        let total = if let Some(limits) = self.limits {
            let bytes = crate::admission::text(name, limits)?.saturating_add(1);
            let old = if self.implementations.contains_key(name) {
                bytes
            } else {
                0
            };
            let total = self
                .name_bytes
                .saturating_sub(old)
                .checked_add(bytes)
                .ok_or_else(crate::admission::size_error)?;
            if total > limits.result_size() {
                return Err(crate::admission::size_error());
            }
            Some(total)
        } else {
            None
        };
        self.implementations
            .insert(name.to_owned(), Box::new(implementation));
        if let Some(total) = total {
            self.name_bytes = total;
        }
        Ok(())
    }
    pub(super) fn get(
        &self,
        name: &str,
    ) -> Option<(&'a openui_lang::JsonValue, &Implementation<'a>)> {
        let definition = self.definitions.as_object()?.get(name)?;
        Some((definition, self.implementations.get(name)?.as_ref()))
    }
}

impl TreeScope<'_> {
    pub(super) fn report_call(&self, message: String, eval: &ExpressionEvaluator<'_>) {
        if let Some(bytes) = self.report_bytes.get()
            && eval.reserve_retained_result(bytes.saturating_add(text_bytes(&message)))
        {
            self.errors.borrow_mut().push(RuntimeError {
                code: "runtime-error",
                component: None,
                statement_id: None,
                message,
                hint: None,
            });
        }
    }
}

pub(super) struct ArgumentCache {
    source: std::sync::Weak<JsObject<Value>>,
    schema: usize,
    params: CachedParams,
    depth: usize,
}
enum CachedParams {
    Unchanged(std::sync::Weak<JsObject<Value>>),
    Transformed(Value),
}
impl CachedParams {
    fn value(&self) -> Option<Value> {
        match self {
            Self::Unchanged(v) => v.upgrade().map(Value::Obj),
            Self::Transformed(v) => Some(v.clone()),
        }
    }
}
impl TreeScope<'_> {
    pub(super) fn cached_arguments(
        &self,
        source: &Value,
        schema: Option<&openui_lang::JsonValue>,
        eval: &ExpressionEvaluator<'_>,
    ) -> Option<Value> {
        let Value::Obj(source) = source else {
            return None;
        };
        let schema = schema? as *const _ as usize;
        self.function_arguments
            .borrow()
            .as_ref()
            .filter(|entry| {
                entry.schema == schema
                    && eval.remaining_depth() >= entry.depth
                    && entry.source.ptr_eq(&Arc::downgrade(source))
            })
            .and_then(|entry| entry.params.value())
    }
    pub(super) fn remember_arguments(
        &self,
        source: Value,
        schema: &openui_lang::JsonValue,
        params: Value,
        eval: &ExpressionEvaluator<'_>,
    ) {
        let Value::Obj(source) = &source else { return };
        let cached = if matches!(&params,Value::Obj(p) if Arc::ptr_eq(p,source)) {
            CachedParams::Unchanged(Arc::downgrade(source))
        } else {
            CachedParams::Transformed(params)
        };
        *self.function_arguments.borrow_mut() = Some(ArgumentCache {
            source: Arc::downgrade(source),
            schema: schema as *const _ as usize,
            params: cached,
            depth: eval.remaining_depth(),
        });
    }
    pub(super) fn invoke_function(
        &self,
        name: &str,
        params: Value,
        eval: &ExpressionEvaluator<'_>,
    ) -> Option<Value> {
        let (definition, implementation) = self.functions?.get(name)?;
        let result = match implementation(params) {
            Ok(result) => result,
            Err(message) => {
                if let Some(message) = eval.copy_text(&message) {
                    self.report_call(format!("@{name} threw: {message}"), eval);
                }
                return Some(Value::Null);
            }
        };
        let returns = definition.get("returns").filter(|s| match s {
            openui_lang::JsonValue::Null => false,
            openui_lang::JsonValue::Bool(b) => *b,
            openui_lang::JsonValue::Number(n) => n.as_f64().is_some_and(|n| n != 0.0),
            openui_lang::JsonValue::String(s) => !s.is_empty(),
            _ => true,
        });
        let Some(returns) = returns else {
            return result;
        };
        let key = returns as *const _ as usize;
        let cached = self.checked_schemas.borrow().get(&key).cloned();
        let schema = if let Some(schema) = cached {
            schema
        } else {
            let schema = std::rc::Rc::new(eval.prepare_validation_schema(returns)?);
            self.checked_schemas
                .borrow_mut()
                .insert(key, schema.clone());
            schema
        };
        let (checked, messages) =
            eval.validate_value(result.as_ref().unwrap_or(&Value::Null), &schema, "/returns");
        for message in messages {
            self.report_call(format!("@{name}: {message}"), eval);
        }
        Some(checked.unwrap_or(Value::Null))
    }
}
