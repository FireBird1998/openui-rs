//! Prompt signatures, as upstream's `signature.ts` builds them from zod:
//! `Name(a: string, b?: number)`, the props in positional order, an optional
//! one marked `?`. Upstream reads zod's introspection; this reads a
//! [`LibraryDefinition`](crate::LibraryDefinition)'s plain data and must give
//! the same text.
//!
//! Upstream's rules, copied: a nullable prop or field is marked `?` as an
//! optional one is (`isOptionalType`), and nullable is unwrapped before a
//! type is named (`unwrap`). A type upstream can't name (an empty enum, whose
//! name is the empty string, or an array of one) has no annotation: a prop
//! then prints as its bare name, or `[name]` when it is an array; a field as
//! its bare name; a record's values as `any`; and a union leaves it out,
//! printing `any` when it names none of its options.
//!
//! The kinds that link a library together print by name and never follow a
//! reference: a component reference as the component's name (upstream's
//! registry id), the action slot as `ActionExpression` and a named action as
//! `@Name`. A named type is upstream's `tagSchemaId`, a name on the schema
//! object of its type: `getSchemaId` reads it before anything else, so it
//! prints instead of the type's structure, but `unwrap` steps past a named
//! nullable to its inner type and drops the name, and everything that asks
//! what kind a schema is (array, union, nullable) sees the type under the
//! name. An empty name is none. A prop's binding prints `$binding<T>`, as
//! `resolveTypeAnnotation` does for a reactive schema.
//!
//! A function's and an action's signatures are built the same way from
//! their params (`schemaSignature`), a function's followed by ` → T` when it
//! declares a return type with an annotation.

use crate::library_definition::{TypeDescription, in_property_order};

/// One entry of a signature: a prop's or a param's name, type, and whether
/// it is optional and a binding.
pub(crate) struct Entry<'a> {
    pub(crate) name: &'a str,
    pub(crate) ty: &'a TypeDescription,
    pub(crate) optional: bool,
    pub(crate) binding: bool,
}

/// The type a named type names, under its names: the kind of schema it is.
fn structure(mut ty: &TypeDescription) -> &TypeDescription {
    while let TypeDescription::Named(_, inner) = ty {
        ty = inner;
    }
    ty
}

/// `unwrap`: the type inside any nullables, a named nullable's name lost
/// with it.
fn unwrap(mut ty: &TypeDescription) -> &TypeDescription {
    while let TypeDescription::Nullable(inner) = structure(ty) {
        ty = inner;
    }
    ty
}

/// Whether the schema is a union: upstream's `getUnionOptions`. The action
/// slot's schema is one.
fn is_union(ty: &TypeDescription) -> bool {
    matches!(
        structure(ty),
        TypeDescription::Union(_) | TypeDescription::Action(None)
    )
}

/// An enum's words as zod keeps them: in JavaScript property order (array
/// indices first), each once, since `z.enum` makes them an object's keys.
pub(crate) fn enum_words(values: &[String]) -> Vec<&str> {
    in_property_order(values, String::as_str)
        .into_iter()
        .map(String::as_str)
        .collect()
}

/// `resolveTypeAnnotation`: the type as the prompt shows it, or `None` where
/// upstream's is `undefined`. The walk goes as deep as the type, which
/// reading bounded, growing the stack on demand.
pub(crate) fn type_annotation(ty: &TypeDescription) -> Option<String> {
    crate::stack::grow_stack!(type_annotation(ty));
    // `if (!baseType) return undefined`: the empty name is none.
    base_type(unwrap(ty)).filter(|name| !name.is_empty())
}

/// `resolveBaseType`, for a type already unwrapped.
fn base_type(ty: &TypeDescription) -> Option<String> {
    // The action ref's tag, then the registry id or the schema's tag.
    let ty = match ty {
        TypeDescription::Action(Some(name)) => return Some(format!("@{name}")),
        TypeDescription::Action(None) => return Some("ActionExpression".to_owned()),
        TypeDescription::Component(name) => return Some(name.clone()),
        TypeDescription::Named(name, _) if !name.is_empty() => return Some(name.clone()),
        TypeDescription::Named(_, inner) => structure(inner),
        other => other,
    };
    Some(match ty {
        TypeDescription::Union(options) => {
            let names: Vec<String> = options.iter().filter_map(type_annotation).collect();
            if names.is_empty() {
                // Upstream falls through to its fallback for a kind it
                // doesn't name.
                "any".to_owned()
            } else {
                names.join(" | ")
            }
        }
        TypeDescription::Array(items) => {
            let inner = type_annotation(items)?;
            if is_union(unwrap(items)) {
                format!("({inner})[]")
            } else {
                format!("{inner}[]")
            }
        }
        TypeDescription::String => "string".to_owned(),
        TypeDescription::Number => "number".to_owned(),
        TypeDescription::Boolean => "boolean".to_owned(),
        TypeDescription::Any => "any".to_owned(),
        TypeDescription::Record(values) => {
            let values = type_annotation(values);
            format!("Record<string, {}>", values.as_deref().unwrap_or("any"))
        }
        // Unescaped, as upstream writes `"${v}"`.
        TypeDescription::Enum(values) => enum_words(values)
            .iter()
            .map(|word| format!("\"{word}\""))
            .collect::<Vec<_>>()
            .join(" | "),
        TypeDescription::Object(fields) => {
            let fields: Vec<String> = in_property_order(fields, |f| &f.name)
                .into_iter()
                .map(|field| {
                    let optional = if is_optional(field.optional, &field.ty) {
                        "?"
                    } else {
                        ""
                    };
                    match type_annotation(&field.ty) {
                        Some(ty) => format!("{}{optional}: {ty}", field.name),
                        None => format!("{}{optional}", field.name),
                    }
                })
                .collect();
            format!("{{{}}}", fields.join(", "))
        }
        TypeDescription::Nullable(inner) => return base_type(unwrap(inner)),
        // `structure` never gives a named type, and the others answered
        // above.
        TypeDescription::Component(_) | TypeDescription::Action(_) | TypeDescription::Named(..) => {
            return base_type(structure(ty));
        }
    })
}

/// `isOptionalType`: optional, or a nullable type (named or not).
fn is_optional(optional: bool, ty: &TypeDescription) -> bool {
    optional || matches!(structure(ty), TypeDescription::Nullable(_))
}

/// `schemaSignature`: `Name(field, ...)`, with the entries in the order
/// given (the caller's JavaScript property order), as upstream's
/// `buildSignature` writes them, then ` → T` for a declared return type
/// with an annotation.
pub(crate) fn call_signature<'a>(
    name: &str,
    entries: impl Iterator<Item = Entry<'a>>,
    returns: Option<&TypeDescription>,
) -> String {
    let mut out = signature(name, entries);
    if let Some(ty) = returns.and_then(type_annotation) {
        out.push_str(" \u{2192} ");
        out.push_str(&ty);
    }
    out
}

/// `buildSignature`: `Name(field, ...)`, with the entries in the order given.
fn signature<'a>(name: &str, props: impl Iterator<Item = Entry<'a>>) -> String {
    let mut out = String::with_capacity(name.len() + 2);
    out.push_str(name);
    out.push('(');
    for (i, prop) in props.enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        let optional = is_optional(prop.optional, prop.ty);
        match type_annotation(prop.ty) {
            Some(ty) => {
                out.push_str(prop.name);
                out.push_str(if optional { "?: " } else { ": " });
                if prop.binding {
                    out.push_str("$binding<");
                    out.push_str(&ty);
                    out.push('>');
                } else {
                    out.push_str(&ty);
                }
            }
            None if matches!(structure(unwrap(prop.ty)), TypeDescription::Array(_)) => {
                out.push('[');
                out.push_str(prop.name);
                out.push_str(if optional { "]?" } else { "]" });
            }
            None => {
                out.push_str(prop.name);
                if optional {
                    out.push('?');
                }
            }
        }
    }
    out.push(')');
    out
}
