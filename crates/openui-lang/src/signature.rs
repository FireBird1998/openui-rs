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

use crate::library_definition::{PropDefinition, TypeDescription, in_property_order};

/// `unwrap`: the type inside any nullables.
fn unwrap(mut ty: &TypeDescription) -> &TypeDescription {
    while let TypeDescription::Nullable(inner) = ty {
        ty = inner;
    }
    ty
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
            if matches!(unwrap(items), TypeDescription::Union(_)) {
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
    })
}

/// `isOptionalType`: optional, or a nullable type.
fn is_optional(optional: bool, ty: &TypeDescription) -> bool {
    optional || matches!(ty, TypeDescription::Nullable(_))
}

/// `schemaSignature` for a component: `Name(field, ...)`, with the props in
/// the order given (the caller's JavaScript property order), as upstream's
/// `buildSignature` writes them.
pub(crate) fn signature<'a>(name: &str, props: impl Iterator<Item = &'a PropDefinition>) -> String {
    let mut out = String::with_capacity(name.len() + 2);
    out.push_str(name);
    out.push('(');
    for (i, prop) in props.enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        let optional = is_optional(prop.optional, &prop.ty);
        match type_annotation(&prop.ty) {
            Some(ty) => {
                out.push_str(&prop.name);
                out.push_str(if optional { "?: " } else { ": " });
                out.push_str(&ty);
            }
            None if matches!(unwrap(&prop.ty), TypeDescription::Array(_)) => {
                out.push('[');
                out.push_str(&prop.name);
                out.push_str(if optional { "]?" } else { "]" });
            }
            None => {
                out.push_str(&prop.name);
                if optional {
                    out.push('?');
                }
            }
        }
    }
    out.push(')');
    out
}
