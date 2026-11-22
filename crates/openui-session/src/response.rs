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

/// What each Cloud script name costs against `max_json_bytes` beside its
/// UTF-8 text. A name is held as a `String` (24 bytes on a 64-bit target)
/// before the session deduplicates the names, so counting only the text let
/// about 2.8 Mi empty names through 8 MiB (#143). Four bytes is more than an
/// empty name's `"",` in a JSON array, so the WebAssembly package's bound on
/// the scripts JSON never refuses before this count for a name without escapes.
const SCRIPT_NAME_BYTES: usize = 4;

/// Read a JSON array of Cloud script names, the form the WebAssembly binding
/// passes, scanning `json` for its string tokens and calling `each` with each
/// token, quotes included. Anything else is InvalidJson.
fn scan_script_names(
    json: &str,
    mut each: impl FnMut(&str) -> Result<(), OpenUiError>,
) -> Result<(), OpenUiError> {
    let invalid = || OpenUiError::InvalidJson {
        reason: "expected a JSON array of script names".into(),
    };
    let bytes = json.as_bytes();
    let skip = |mut at: usize| {
        while matches!(bytes.get(at), Some(b' ' | b'\n' | b'\r' | b'\t')) {
            at += 1;
        }
        at
    };
    let end = |at: usize| {
        if skip(at) == bytes.len() {
            Ok(())
        } else {
            Err(invalid())
        }
    };
    let mut at = skip(0);
    if bytes.get(at) != Some(&b'[') {
        return Err(invalid());
    }
    at = skip(at + 1);
    if bytes.get(at) == Some(&b']') {
        return end(at + 1);
    }
    // Every step moves `at` forward through the text, so both loops end.
    loop {
        if bytes.get(at) != Some(&b'"') {
            return Err(invalid());
        }
        let start = at;
        at += 1;
        loop {
            match bytes.get(at) {
                None => return Err(invalid()),
                Some(b'"') => break,
                Some(b'\\') => at += 2,
                Some(_) => at += 1,
            }
        }
        at += 1;
        each(json.get(start..at).ok_or_else(invalid)?)?;
        at = skip(at);
        match bytes.get(at) {
            Some(b',') => at = skip(at + 1),
            Some(b']') => return end(at + 1),
            _ => return Err(invalid()),
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
        // Charge each name its text and SCRIPT_NAME_BYTES, empty names included.
        let mut used = state
            .error
            .as_ref()
            .map_or(0, String::len)
            .saturating_add(state.script_revision.as_ref().map_or(0, String::len))
            .saturating_add(state.scripts.len().saturating_mul(SCRIPT_NAME_BYTES));
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
            // One insert at a time: collecting would reserve a slot per
            // name, repeated names included, before any is deduplicated.
            let mut names = std::collections::HashSet::new();
            for name in state.scripts {
                names.insert(name);
            }
            Some(Arc::new(openui_runtime::CloudScripts {
                names,
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
    /// The Cloud script names in `json`, a JSON array of strings: the form the
    /// WebAssembly binding passes them in. The text is held to
    /// `max_json_bytes` before it is read, and the names are counted, and
    /// charged as a draw charges them, before any is built, so no more are
    /// built than a draw admits. A refusal is LimitExceeded, malformed text
    /// InvalidJson.
    pub fn script_names(&self, json: &str) -> Result<Vec<String>, OpenUiError> {
        self.text(json)?;
        let limit_bytes = self.limits.max_json_bytes;
        let over = |used: usize| used as u64 > limit_bytes;
        let mut count = 0usize;
        scan_script_names(json, |_| {
            count = count.saturating_add(1);
            if over(count.saturating_mul(SCRIPT_NAME_BYTES)) {
                Err(limit("response state byte limit"))
            } else {
                Ok(())
            }
        })?;
        let mut names = Vec::with_capacity(count);
        let mut used = 0usize;
        scan_script_names(json, |token| {
            let name: String =
                serde_json::from_str(token).map_err(|error| OpenUiError::InvalidJson {
                    reason: error.to_string(),
                })?;
            used = used
                .saturating_add(name.len())
                .saturating_add(SCRIPT_NAME_BYTES);
            if over(used) {
                return Err(limit("response state byte limit"));
            }
            names.push(name);
            Ok(())
        })?;
        Ok(names)
    }

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
