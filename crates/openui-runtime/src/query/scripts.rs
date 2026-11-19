//! The renderer's extraDeps for queries calling settled Cloud scripts.
use super::*;

/// Cloud script names of one settled response and their opaque revision.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct CloudScripts {
    /// Tool names in the reader's scripts set.
    pub names: HashSet<String>,
    /// Opaque identifier of the scripts section, absent for a null revision.
    pub revision: Option<String>,
}

impl CloudScripts {
    pub(super) fn dependency(
        &self,
        tool: Option<&Value>,
        eval: &ExpressionEvaluator<'_>,
    ) -> Result<Option<(String, Ast)>, QueryCommandError> {
        // Set.has tests the evaluated name before the manager coerces it to a string.
        if !matches!(tool, Some(Value::Str(name)) if self.names.contains(name)) {
            return Ok(None);
        }
        let value = match self.revision.as_deref() {
            Some(text) => Ast::Str(eval.copy_text(text).ok_or_else(|| {
                QueryCommandError::Limit(LimitError {
                    flags: eval.flags(),
                })
            })?),
            None => Ast::Null,
        };
        Ok(Some(("__openuiScriptRevision".into(), value)))
    }
}
