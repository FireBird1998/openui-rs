#![allow(dead_code, unused_imports)]
pub use openui_session::{
    ActionHandle, ActionOutput, EvaluationOutput, OpenUiError, ParseLimits, RuntimeLimits,
};
use std::{cell::RefCell, ops::Deref, sync::Arc};
pub struct ComponentLibrary(openui_wasm::ComponentLibrary);
impl ComponentLibrary {
    pub fn new(schema: String) -> Result<Arc<Self>, OpenUiError> {
        openui_wasm::ComponentLibrary::new(&schema)
            .map(|v| Arc::new(Self(v)))
            .map_err(|_| OpenUiError::InvalidSchema {
                reason: "schema".into(),
            })
    }
}
pub struct RuntimeSession(openui_wasm::session::Session);
impl Deref for RuntimeSession {
    type Target = openui_wasm::session::Session;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl RuntimeSession {
    pub fn new(
        library: Arc<ComponentLibrary>,
        spec: Option<String>,
    ) -> Result<Arc<Self>, OpenUiError> {
        Self::with_limits(library, spec, RuntimeLimits::default())
    }
    #[allow(clippy::arc_with_non_send_sync)] // Keep the native suite's handle type; used on one thread.
    pub fn with_limits(
        library: Arc<ComponentLibrary>,
        spec: Option<String>,
        limits: RuntimeLimits,
    ) -> Result<Arc<Self>, OpenUiError> {
        openui_wasm::session::Session::with_limits(library.0.shared(), spec, limits)
            .map(|v| Arc::new(Self(v)))
    }
    pub fn dispose(&self) {
        self.0.dispose().unwrap();
    }
}
pub struct Output {
    pub program: Arc<openui_session::ParsedProgram>,
    pub json: String,
    pub size_limited: bool,
}
fn output(program: openui_wasm::ParsedProgram) -> Output {
    let program = program.shared();
    Output {
        json: program.inner.to_json(),
        size_limited: program.inner.size_limited,
        program,
    }
}
pub fn parse(source: String, library: Arc<ComponentLibrary>, root: Option<String>) -> Output {
    output(openui_wasm::parse_program(&source, &library.0, root))
}
pub struct StreamSession(RefCell<openui_wasm::StreamSession>);
impl StreamSession {
    pub fn new(library: Arc<ComponentLibrary>, root: Option<String>) -> Self {
        Self(RefCell::new(openui_wasm::StreamSession::new(
            &library.0, root,
        )))
    }
    pub fn push(&self, text: String) -> Output {
        let mut s = self.0.borrow_mut();
        s.feed(&text);
        output(s.program(false))
    }
    pub fn set(&self, text: String) -> Output {
        let mut s = self.0.borrow_mut();
        s.set(&text);
        output(s.program(false))
    }
    pub fn finish(&self) -> Output {
        output(self.0.borrow_mut().program(true))
    }
}
