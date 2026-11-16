//! Thread-local storage keeps JavaScript values off the Send + Sync core bridge.
//! The session owns each bridge; dropping it releases the corresponding callback.
use openui_session::HostFunction;
use std::{cell::RefCell, collections::BTreeMap, sync::Arc};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[derive(Clone)]
    #[wasm_bindgen(typescript_type = "(argumentsJson: string) => string | undefined")]
    pub type FunctionCallback;
    #[wasm_bindgen(method, catch, js_name = call)]
    fn invoke(
        this: &FunctionCallback,
        receiver: &JsValue,
        arguments: &str,
    ) -> Result<JsValue, JsValue>;
}
thread_local! { static CALLBACKS: RefCell<BTreeMap<usize, FunctionCallback>> = const { RefCell::new(BTreeMap::new()) }; }
// The key allocation remains live until this bridge drops. Removal precedes
// deallocation, so reuse cannot name an old entry. No serial IDs or free-slot
// history accumulate on refused registrations or after session disposal.
struct Callback(Arc<()>);
impl Callback {
    fn key(&self) -> usize {
        Arc::as_ptr(&self.0) as usize
    }
    fn call(&self, json: String) -> Result<Option<String>, String> {
        // Release the registry borrow before executing host code. The package
        // guard and the inner session's try_borrow_mut both refuse reentry.
        let callback = CALLBACKS
            .with(|callbacks| callbacks.borrow().get(&self.key()).cloned())
            .ok_or_else(|| "host function released".to_owned())?;
        let value = callback
            .invoke(&JsValue::UNDEFINED, &json)
            .map_err(|error| {
                error
                    .as_string()
                    .unwrap_or_else(|| "host function threw".into())
            })?;
        if value.is_undefined() {
            Ok(None)
        } else {
            value
                .as_string()
                .map(Some)
                .ok_or_else(|| "expected host function JSON text or undefined".into())
        }
    }
}
impl Drop for Callback {
    fn drop(&mut self) {
        CALLBACKS.with(|callbacks| {
            callbacks.borrow_mut().remove(&self.key());
        });
    }
}
pub fn bridge(function: FunctionCallback) -> Arc<HostFunction> {
    let callback = Callback(Arc::new(()));
    CALLBACKS.with(|callbacks| {
        callbacks.borrow_mut().insert(callback.key(), function);
    });
    Arc::new(move |json| callback.call(json))
}
