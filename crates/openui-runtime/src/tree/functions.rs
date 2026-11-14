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
/// through; strings are owned String values and are copied on each argument
/// projection. Admission rereads their bytes, and schema-driven object
/// rebuilds can copy string fields again. These walks/copies spend work per call;
/// sharing those strings would require changing Value's public representation.
/// Registering the same name replaces its implementation.
///
/// Host computation, allocation, and side effects are the embedder's
/// responsibility. Returned values undergo the session's bounded admission
/// and projection and never earn input allowance. Err becomes a diagnostic and
/// null. Host panics propagate; evaluator contexts restore during unwinding,
/// and evaluation leaves Store values and its version unchanged. Catching a host panic allows
/// subsequent evaluations to use that Store normally. Host side effects cannot
/// be rolled back by the runtime.
pub struct LibraryFunctions<'a> {
    pub(super) definitions: &'a openui_lang::JsonValue,
    implementations: BTreeMap<String, Box<Implementation<'a>>>,
}
impl<'a> LibraryFunctions<'a> {
    /// Pair implementations with the original library's `functions` object.
    pub fn new(definitions: &'a openui_lang::JsonValue) -> Self {
        Self {
            definitions,
            implementations: BTreeMap::new(),
        }
    }
    /// Register or replace a synchronous implementation. Names are literal.
    /// A name absent from the definitions is ignored during evaluation.
    pub fn register(
        &mut self,
        name: impl Into<String>,
        implementation: impl Fn(Value) -> FunctionResult + Send + Sync + 'a,
    ) {
        self.implementations
            .insert(name.into(), Box::new(implementation));
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
            && eval.reserve_result(bytes.saturating_add(text_bytes(&message)))
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
