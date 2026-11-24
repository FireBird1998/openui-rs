//! Immutable schema indexes for runtime validation; parsing does not use them.
use super::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

/// A borrowed, immutable schema with bounded, reusable validation indexes.
/// Construct through `ExpressionEvaluator::prepare_validation_schema`. Borrowing
/// prevents mutation/address reuse while the indexes exist. Each session pays
/// admission and indexing work once; another session must admit it again.
/// ```compile_fail
/// # #[global_allocator]
/// # static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);
/// use openui_lang::{EvaluationSession, Value};
/// let mut raw=serde_json::json!({"type":"object"});
/// let session=EvaluationSession::new(&(),0);
/// let prepared=session.with_evaluator(|e|e.prepare_validation_schema(&raw).unwrap());
/// raw["type"]=serde_json::json!("string");
/// session.with_evaluator(|e|e.check_value(&Value::Null,&prepared));
/// ```
pub struct PreparedValidationSchema<'s> {
    pub(crate) root: &'s Json,
    pub(crate) token: Arc<u8>,
    pub(crate) index_work: usize,
    objects: BTreeMap<usize, ObjectSchema<'s>>,
    root_names: Vec<String>,
    root_required: BTreeSet<&'s str>,
}
pub(super) struct ObjectSchema<'s> {
    projects_number_fields: bool,
    pub(super) props: Vec<(String, &'s Json)>,
    pub(super) required: Vec<String>,
    lookup: BTreeMap<String, usize>,
    exact_required: BTreeSet<&'s str>,
    inherited: Vec<usize>,
}
impl crate::ExpressionEvaluator<'_> {
    /// Prepare a library function's return schema. JSON may have projected a
    /// non-finite number to null in the returned object or in a returned row.
    pub fn prepare_function_return_schema<'s>(
        &self,
        schema: &'s Json,
    ) -> Option<PreparedValidationSchema<'s>> {
        let mut prepared = self.prepare_validation_schema(schema)?;
        let record = match schema.get("type").and_then(Json::as_str) {
            Some("object") => Some(schema),
            Some("array") => schema.get("items"),
            _ => None,
        };
        if let Some(object) = record.and_then(Json::as_object)
            && let Some(index) = prepared.objects.get_mut(&(object as *const _ as usize))
        {
            index.projects_number_fields = true;
        }
        Some(prepared)
    }
}
impl<'s> PreparedValidationSchema<'s> {
    pub(crate) fn build(root: &'s Json) -> Option<Self> {
        let mut out = Self {
            root,
            token: Arc::new(0),
            index_work: 0,
            objects: BTreeMap::new(),
            root_names: Vec::new(),
            root_required: BTreeSet::new(),
        };
        if out.walk(root) && out.root_metadata() {
            Some(out)
        } else {
            None
        }
    }
    fn root_object(&self) -> Option<&ObjectSchema<'s>> {
        self.root.as_object().and_then(|o| self.object(o))
    }
    fn root_metadata(&mut self) -> bool {
        let props = self.root.get("properties");
        if self.root_object().is_none() || matches!(props, Some(Json::String(_))) {
            self.root_names = match props {
                Some(Json::Object(p)) => {
                    let bytes = p
                        .keys()
                        .fold(0usize, |n, k| n.saturating_add(k.len().saturating_add(1)));
                    let log = p.len().checked_ilog2().unwrap_or(0) as usize + 2;
                    if !crate::evaluate::validation_work(bytes.saturating_mul(log)) {
                        return false;
                    }
                    json_entries(p)
                        .into_iter()
                        .map(|(k, _)| k.clone())
                        .collect()
                }
                Some(Json::Array(a)) => {
                    if !crate::evaluate::validation_work(a.len().saturating_mul(24)) {
                        return false;
                    }
                    (0..a.len()).map(|i| i.to_string()).collect()
                }
                Some(Json::String(s)) => {
                    let len = s.encode_utf16().count();
                    if !crate::evaluate::validation_work(len.saturating_mul(24)) {
                        return false;
                    }
                    (0..len).map(|i| i.to_string()).collect()
                }
                _ => Vec::new(),
            };
        }
        if self.root_object().is_none()
            && let Some(Json::Array(a)) = self.root.get("required")
        {
            let log = a.len().checked_ilog2().unwrap_or(0) as usize + 2;
            for s in a.iter().filter_map(Json::as_str) {
                if !crate::evaluate::validation_work(s.len().saturating_add(1).saturating_mul(log))
                {
                    return false;
                }
                self.root_required.insert(s);
            }
        }
        true
    }
    /// Root `properties` names in JavaScript Object.keys order. Preparing
    /// materializes/indexes these once, including array/string property keys.
    pub fn property_names(&self) -> impl Iterator<Item = &str> {
        self.root_object()
            .into_iter()
            .flat_map(|o| o.props.iter().map(|(k, _)| k.as_str()))
            .chain(self.root_names.iter().map(String::as_str))
    }
    /// Exact string membership in the root required array, without coercion.
    /// Other required kinds are deliberately left to the caller's semantics.
    pub fn requires(&self, name: &str) -> bool {
        self.root_object().map_or_else(
            || self.root_required.contains(name),
            |o| o.is_required(name),
        )
    }
    fn walk(&mut self, node: &'s Json) -> bool {
        crate::stack::grow_stack!(self.walk(node), budget else false);
        if !crate::evaluate::validation_work(1) {
            return false;
        }
        match node {
            Json::Object(o) => {
                if o.get("type").and_then(Json::as_str) == Some("object") {
                    let props: Vec<(String, &Json)> = match o.get("properties") {
                        Some(Json::Object(p)) => {
                            let bytes = p
                                .keys()
                                .fold(0usize, |n, k| n.saturating_add(k.len().saturating_add(1)));
                            let sort = p.len().checked_ilog2().unwrap_or(0) as usize + 2;
                            if !crate::evaluate::validation_work(bytes.saturating_mul(sort)) {
                                return false;
                            }
                            json_entries(p)
                                .into_iter()
                                .map(|(k, v)| (k.clone(), v))
                                .collect()
                        }
                        Some(Json::Array(p)) => {
                            if !crate::evaluate::validation_work(p.len().saturating_mul(24)) {
                                return false;
                            }
                            p.iter()
                                .enumerate()
                                .map(|(i, v)| (i.to_string(), v))
                                .collect()
                        }
                        _ => Vec::new(),
                    };
                    let mut required = Vec::new();
                    let mut exact_required = BTreeSet::new();
                    if let Some(Json::Array(items)) = o.get("required") {
                        let logarithm = items.len().checked_ilog2().unwrap_or(0) as usize + 2;
                        for item in items {
                            let Some(key) = crate::evaluate::validation_name(item) else {
                                return false;
                            };
                            if !crate::evaluate::validation_work(
                                key.len().saturating_add(1).saturating_mul(logarithm),
                            ) {
                                return false;
                            }
                            required.push(key);
                            if let Some(s) = item.as_str() {
                                exact_required.insert(s);
                            }
                        }
                    }
                    let mut lookup = BTreeMap::new();
                    let mut inherited = Vec::new();
                    for (i, (key, _)) in props.iter().enumerate() {
                        if !crate::evaluate::validation_work(key.len().saturating_add(1)) {
                            return false;
                        }
                        lookup.insert(key.clone(), i);
                        if OBJECT_PROTOTYPE_KEYS.contains(&key.as_str()) {
                            inherited.push(i);
                        }
                    }
                    self.objects.insert(
                        o as *const _ as usize,
                        ObjectSchema {
                            projects_number_fields: false,
                            props,
                            required,
                            lookup,
                            exact_required,
                            inherited,
                        },
                    );
                }
                for v in o.values() {
                    if !self.walk(v) {
                        return false;
                    }
                }
            }
            Json::Array(a) => {
                for v in a {
                    if !self.walk(v) {
                        return false;
                    }
                }
            }
            _ => {}
        }
        true
    }
    pub(super) fn object(
        &self,
        schema: &serde_json::Map<String, Json>,
    ) -> Option<&ObjectSchema<'s>> {
        self.objects.get(&(schema as *const _ as usize))
    }
}
impl ObjectSchema<'_> {
    pub(super) fn accepts_projected_null(&self, key: &str) -> bool {
        self.projects_number_fields
            && self.prop(key).and_then(Json::as_object).is_some_and(|s| {
                s.get("type").and_then(Json::as_str) == Some("number")
                    && !is_composite(s)
                    && !s.contains_key("enum")
                    && !s.contains_key("const")
            })
    }
    pub(super) fn prop(&self, key: &str) -> Option<&Json> {
        self.lookup
            .get(key)
            .and_then(|i| self.props.get(*i))
            .map(|(_, v)| *v)
    }
    pub(super) fn is_required(&self, key: &str) -> bool {
        self.exact_required.contains(key)
    }
    pub(super) fn order(&self, obj: &JsObject<Value>) -> Option<Vec<usize>> {
        let mut out = Vec::new();
        for (key, _) in obj.iter() {
            if !crate::evaluate::validation_work(key.len().saturating_add(1)) {
                return None;
            }
            if let Some(i) = self.lookup.get(key) {
                out.push(*i);
            }
        }
        for i in &self.inherited {
            if let Some((key, _)) = self.props.get(*i)
                && !obj.has_own(key)
            {
                out.push(*i);
            }
        }
        if !crate::evaluate::validation_work(out.len().saturating_add(1)) {
            return None;
        }
        // Bounded-word radix order: linear even for large objects. Small lists
        // use insertion sort, with at most 32 comparisons per retained index.
        if out.len() <= 32 {
            for i in 1..out.len() {
                let mut j = i;
                while j > 0 && out.get(j) < out.get(j - 1) {
                    out.swap(j, j - 1);
                    j -= 1;
                }
            }
        } else {
            let mut scratch = vec![0; out.len()];
            for shift in (0..usize::BITS).step_by(8) {
                let mut counts = [0usize; 256];
                for n in &out {
                    if let Some(c) = counts.get_mut((n >> shift) & 255) {
                        *c = c.saturating_add(1);
                    }
                }
                let mut start = 0usize;
                for count in &mut counts {
                    let old = *count;
                    *count = start;
                    start = start.saturating_add(old);
                }
                for n in &out {
                    if let Some(c) = counts.get_mut((n >> shift) & 255) {
                        if let Some(slot) = scratch.get_mut(*c) {
                            *slot = *n;
                        }
                        *c = c.saturating_add(1);
                    }
                }
                std::mem::swap(&mut out, &mut scratch);
            }
        }
        Some(out)
    }
}
