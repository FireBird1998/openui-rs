//! The two response gates and the Cloud scripts retained across a streaming edit.
use super::*;

/// The reader's flags and Cloud scripts for one draw. A bare response needs
/// only `streaming`; the default is a settled bare program.
#[derive(Clone, Debug)]
pub struct ResponseState {
    pub streaming: bool,
    pub is_bundle: bool,
    pub complete: bool,
    pub error: Option<String>,
    pub scripts: Vec<String>,
    pub script_revision: Option<String>,
}

impl From<bool> for ResponseState {
    fn from(streaming: bool) -> Self {
        Self {
            streaming,
            ..Self::default()
        }
    }
}

impl Default for ResponseState {
    fn default() -> Self {
        Self {
            streaming: false,
            is_bundle: false,
            complete: true,
            error: None,
            scripts: Vec::new(),
            script_revision: None,
        }
    }
}

impl ResponseState {
    /// Take the response metadata from the reader, keeping its program available to parse.
    pub fn from_bundle(streaming: bool, bundle: &openui_lang::ResponseBundle) -> Self {
        Self {
            streaming,
            is_bundle: bundle.is_bundle,
            complete: bundle.complete,
            error: bundle.error.map(str::to_owned),
            scripts: bundle.script_names.clone(),
            script_revision: bundle.script_revision().map(str::to_owned),
        }
    }
}

#[derive(Default)]
pub(super) struct DrawResponse {
    pub blocked: bool,
    pub bundle_blocked: bool,
    pub streaming: bool,
    pub scripts: Option<Arc<openui_runtime::CloudScripts>>,
    pub parser_error_json: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum RegistrationEffects {
    None,
    Queries,
    QueriesAndMutations,
}

impl DrawResponse {
    pub fn effects(&self, register_statements: bool, keys_changed: bool) -> RegistrationEffects {
        if self.blocked {
            RegistrationEffects::None
        } else if register_statements {
            RegistrationEffects::QueriesAndMutations
        } else if keys_changed {
            RegistrationEffects::Queries
        } else {
            RegistrationEffects::None
        }
    }

    pub fn pending() -> Self {
        Self {
            blocked: true,
            streaming: true,
            ..Self::default()
        }
    }

    pub fn prepare(
        &self,
        state: ResponseState,
        limit_bytes: u64,
        limits: Limits,
    ) -> Result<Self, OpenUiError> {
        // Charge a byte per name as well as its text, including empty names.
        let mut used = state
            .error
            .as_ref()
            .map_or(0, String::len)
            .saturating_add(state.script_revision.as_ref().map_or(0, String::len))
            .saturating_add(state.scripts.len());
        for name in &state.scripts {
            used = used.saturating_add(name.len());
            if used as u64 > limit_bytes {
                return Err(limit("response state byte limit"));
            }
        }
        if used as u64 > limit_bytes {
            return Err(limit("response state byte limit"));
        }
        let error = state.error.filter(|message| !message.is_empty());
        let blocked = state.streaming || !state.complete || error.is_some();
        let scripts = if blocked {
            self.scripts.clone()
        } else if state.scripts.is_empty() {
            None
        } else {
            Some(Arc::new(openui_runtime::CloudScripts {
                names: state.scripts.into_iter().collect(),
                revision: state.script_revision,
            }))
        };
        let parser_error_json = error
            .map(|message| {
                let mut fields = JsObject::new();
                fields.define("source", Value::Str("parser".into()));
                fields.define("code", Value::Str("parse-failed".into()));
                fields.define("message", Value::Str(message));
                openui_runtime::value_to_json_with_limits(&Value::Obj(Arc::new(fields)), limits)
                    .map_err(|error| {
                        runtime_error(RuntimeFailure::Limit(error), limits.result_size())
                    })
            })
            .transpose()?;
        Ok(Self {
            blocked,
            bundle_blocked: state.is_bundle && blocked,
            streaming: state.streaming,
            scripts,
            parser_error_json,
        })
    }

    pub fn input<'a>(&'a self, mut input: TreeInput<'a>) -> TreeInput<'a> {
        input.cloud_scripts = self.scripts.as_deref();
        input
    }

    pub fn is_query_loading(
        &self,
        program: &ParseResult,
        manager: Option<&openui_runtime::QueryManager>,
    ) -> bool {
        (self.streaming && self.bundle_blocked && !program.query_statements.is_empty())
            || manager.is_some_and(|manager| matches!(manager.snapshot().get("__openui_loading"), Some(Value::Arr(ids)) if !ids.is_empty()))
    }
}

impl SessionCore {
    /// Supply the app's extra cache-key values for one tool on the attached provider.
    /// The next eligible query evaluation merges these before the Cloud script revision.
    pub fn set_query_extra_values(
        &mut self,
        tool_name: &str,
        values_json: &str,
    ) -> Result<(), OpenUiError> {
        self.text(tool_name)?;
        self.text(values_json)?;
        let manager = self
            .manager
            .as_mut()
            .ok_or_else(|| OpenUiError::RefusedWrite {
                reason: "query extra values require an attached tool provider".into(),
            })?;
        let values = Json(
            openui_lang::parse_json_with_limits(values_json, self.core_limits).map_err(
                |error| {
                    if error.depth_limited() {
                        limit("query extra values depth limit")
                    } else {
                        OpenUiError::InvalidJson {
                            reason: error.to_string(),
                        }
                    }
                },
            )?,
        );
        let fields = values
            .0
            .as_object()
            .ok_or_else(|| OpenUiError::InvalidJson {
                reason: "expected an object of extra cache-key values".into(),
            })?;
        manager.set_extra_key_values(
            tool_name,
            fields.iter().map(|(name, value)| (name.as_str(), value)),
        )?;
        self.extra_values_changed = true;
        Ok(())
    }
}
