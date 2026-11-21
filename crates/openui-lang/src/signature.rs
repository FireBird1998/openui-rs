//! Prompt signatures, as upstream's `signature.ts` builds them from zod:
//! `Name(a: string, b?: number)`, the props in positional order, an optional
//! one marked `?`. Upstream reads zod's introspection; this reads a
//! [`LibraryDefinition`](crate::LibraryDefinition)'s plain data and must give
//! the same text.

use crate::library_definition::{PropDefinition, TypeDescription};

/// `resolveTypeAnnotation`: the type as the prompt shows it.
pub(crate) fn type_annotation(ty: &TypeDescription) -> &'static str {
    match ty {
        TypeDescription::String => "string",
        TypeDescription::Number => "number",
        TypeDescription::Boolean => "boolean",
        TypeDescription::Any => "any",
    }
}

/// `schemaSignature` for a component: `Name(field, ...)`, with the props in
/// the order given (the caller's JavaScript property order).
pub(crate) fn signature<'a>(name: &str, props: impl Iterator<Item = &'a PropDefinition>) -> String {
    let mut out = String::with_capacity(name.len() + 2);
    out.push_str(name);
    out.push('(');
    for (i, prop) in props.enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(&prop.name);
        out.push_str(if prop.optional { "?: " } else { ": " });
        out.push_str(type_annotation(&prop.ty));
    }
    out.push(')');
    out
}
