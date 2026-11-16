//! The renderer's store-backed form fields.
use super::*;

/// The result of a form write. The save flag requests an explicit persistence
/// callback even if the store did not change; ordinary change notifications
/// still follow Store::version independently. This crate invokes no callbacks.
#[derive(Debug)]
#[non_exhaustive]
pub struct FieldWrite {
    /// Whether the store changed.
    pub changed: bool,
    /// Whether the caller requested the renderer's extra save callback.
    pub save_requested: bool,
}

pub(super) fn unwrap_field(store: &Store, value: crate::StateValue) -> crate::StateValue {
    if value
        .as_ref()
        .is_some_and(|v| store.undefined_fields.owns(v))
    {
        return None;
    }
    if let Some(Value::Ast(a)) = &value
        && let Ast::Assign { value, .. } = a.as_ref()
    {
        return Some(Value::Ast(value.clone()));
    }
    match value {
        Some(Value::Obj(ref o)) if o.has("value") => o.get("value").cloned(),
        value => value,
    }
}

/// Read a renderer form field. No/empty form name reads a top-level Store key;
/// a named form reads an object property. Entries containing `value` unwrap.
/// Absent or undefined returns None; null stays null. Inherited JS functions
/// project to undefined in Value, like the core's expression interface.
pub fn get_field_value(store: &Store, form_name: Option<&str>, name: &str) -> crate::StateValue {
    if (name != "__proto__" && !store.possible_name(name))
        || form_name.is_some_and(|form| !store.possible_name(form))
    {
        return None;
    }
    if let Some(form) = form_name.filter(|s| !s.is_empty()) {
        let snapshot = store.snapshot();
        let raw = snapshot.get(form)?.as_ref()?;
        if !matches!(raw, Value::Obj(_) | Value::Element(_) | Value::Ast(_)) {
            return None;
        }
        let fields = own_fields(raw);
        let value = fields.get(name).cloned().or_else(|| {
            (name == "__proto__" && fields.has(name)).then(|| Value::Obj(Arc::new(JsObject::new())))
        });
        unwrap_field(store, value)
    } else {
        unwrap_field(store, store.get(name))
    }
}

/// Store `{value,componentType}` under the field name, inside the named form
/// or at top level. None omits an undefined property. componentType is metadata,
/// not a field name or a value transform. `false` suppresses only the explicit
/// save request, never the underlying Store write/change notification.
pub fn set_field_value(
    store: &mut Store,
    form_name: Option<&str>,
    component_type: Option<&str>,
    name: &str,
    value: crate::StateValue,
    should_trigger_save_callback: bool,
) -> Result<FieldWrite, BindingWriteError> {
    set_field_value_with_limits(
        store,
        form_name,
        component_type,
        name,
        value,
        should_trigger_save_callback,
        Limits::DEFAULT,
    )
}

/// Persist an unbound field's defined default only after streaming settles and
/// while its current value is undefined. Null is an existing value. The usual
/// renderer default uses save false; callers can request persistence themselves
/// when this returns Some. State-bound fields use their marker instead.
pub fn initialize_field_default(
    store: &mut Store,
    form_name: Option<&str>,
    component_type: Option<&str>,
    name: &str,
    value: crate::StateValue,
    streaming: bool,
) -> Result<Option<FieldWrite>, BindingWriteError> {
    initialize_field_default_with_limits(
        store,
        form_name,
        component_type,
        name,
        value,
        streaming,
        Limits::DEFAULT,
    )
}

/// Bounded default persistence, with the renderer's undefined/streaming guard.
/// Inherited function-valued properties count as present even though the core's
/// public Value projects a JavaScript function to undefined.
pub fn initialize_field_default_with_limits(
    store: &mut Store,
    form_name: Option<&str>,
    component_type: Option<&str>,
    name: &str,
    value: crate::StateValue,
    streaming: bool,
    limits: Limits,
) -> Result<Option<FieldWrite>, BindingWriteError> {
    if streaming || value.is_none() || get_field_value(store, form_name, name).is_some() {
        return Ok(None);
    }
    if let Some(form) = form_name.filter(|s| !s.is_empty()) {
        let snapshot = store.snapshot();
        if let Some(Some(raw @ (Value::Obj(_) | Value::Ast(_) | Value::Element(_)))) =
            snapshot.get(form)
        {
            let fields = own_fields(raw);
            if !fields.has_own(name) && fields.has(name) {
                return Ok(None);
            }
        }
    }
    set_field_value_with_limits(store, form_name, component_type, name, value, false, limits)
        .map(Some)
}

/// A bounded form write; refusal leaves value and pristine status untouched.
/// Defaults are a renderer responsibility: while streaming do nothing; once
/// settled, write only if the existing value is undefined and the default is defined,
/// normally with the save flag false (`useSetDefaultValue`, renderer 1.1.0).
/// A reactive marker uses write_binding instead and never creates a form entry.
pub fn set_field_value_with_limits(
    store: &mut Store,
    form_name: Option<&str>,
    component_type: Option<&str>,
    name: &str,
    value: crate::StateValue,
    should_trigger_save_callback: bool,
    limits: Limits,
) -> Result<FieldWrite, BindingWriteError> {
    if let Some(v) = &value {
        runner::admit(v, limits)?;
    }
    let name = runner::copy_text(name, limits)?;
    let form_name = form_name
        .map(|s| runner::copy_text(s, limits))
        .transpose()?;
    let component_type = component_type
        .map(|s| runner::copy_text(s, limits))
        .transpose()?;
    let undefined = value.is_none();
    let mut wrapped = JsObject::new();
    if let Some(v) = value {
        wrapped.define("value", v);
    }
    if let Some(t) = component_type {
        wrapped.define("componentType", Value::Str(t));
    }
    let wrapped = Arc::new(wrapped);
    if undefined {
        store.undefined_fields.mark(&wrapped);
    }
    let wrapped = Value::Obj(wrapped);
    let (key, value) = if let Some(form) = form_name.as_deref().filter(|s| !s.is_empty()) {
        let snapshot = store.snapshot();
        let raw = snapshot.get(form).and_then(|v| v.as_ref());
        if let Some(v @ (Value::Obj(_) | Value::Ast(_) | Value::Element(_))) = raw {
            runner::admit(v, limits)?;
        }
        // Object spread creates an ordinary object, even from null-prototype
        // input, and defines own __proto__ properties without invoking setters.
        let mut fields = JsObject::new();
        if let Some(v @ (Value::Obj(_) | Value::Ast(_) | Value::Element(_))) = raw {
            for (key, value) in own_fields(v).iter() {
                fields.define(key, value.clone());
            }
        }
        fields.define(name, wrapped);
        (form, Value::Obj(Arc::new(fields)))
    } else {
        (name.as_str(), wrapped)
    };
    runner::admit(&value, limits)?;
    let changed = store
        .set(key, Some(value))
        .map_err(BindingWriteError::Store)?;
    Ok(FieldWrite {
        changed,
        save_requested: should_trigger_save_callback,
    })
}

pub(super) fn form_payload(
    store: &Store,
    form_name: Option<&str>,
    limits: Limits,
    eval: &ExpressionEvaluator<'_>,
) -> Result<Value, BindingWriteError> {
    if let Some(form) = form_name.filter(|s| !s.is_empty()) {
        let snapshot = store.snapshot();
        if let Some(Some(raw @ (Value::Obj(_) | Value::Element(_) | Value::Ast(_)))) =
            snapshot.get(form)
        {
            if !eval.read_state_value(form, raw) || !eval.reserve_value(raw) {
                return Err(BindingWriteError::Cut(eval.flags()));
            }
            if !eval.reserve_container_storage(1) {
                return Err(BindingWriteError::Cut(eval.flags()));
            }
            let mut out = JsObject::new();
            out.define(
                eval.copy_text(form)
                    .ok_or_else(|| BindingWriteError::Cut(eval.flags()))?,
                raw.clone(),
            );
            return Ok(Value::Obj(Arc::new(out)));
        }
    }
    let snapshot = store.snapshot();
    if !eval.reserve_container_storage(snapshot.len()) {
        return Err(BindingWriteError::Cut(eval.flags()));
    }
    let mut out = JsObject::new();
    let admitted = (|| {
        if !eval.reserve_result(2) {
            return false;
        }
        for (key, value) in snapshot.iter() {
            if !eval.charge_work(1) {
                return false;
            }
            let Some(value) = value else {
                continue;
            };
            if !eval.read_state_key(key) {
                return false;
            }
            if !eval.reserve_result(2) {
                return false;
            }
            if !runner::reserve_text(eval, key, limits) {
                return false;
            }
            let Some(key) = eval.copy_text(key) else {
                return false;
            };
            if !eval.read_state_value(&key, value) || !eval.reserve_value(value) {
                return false;
            }
            out.define(key, value.clone());
        }
        true
    })();
    if !admitted {
        return Err(BindingWriteError::Cut(eval.flags()));
    }
    Ok(Value::Obj(Arc::new(out)))
}
