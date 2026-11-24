//! `Clone`, `Debug` and `PartialEq` on library definitions at every depth the
//! reader accepts (#231). A type nests as deep as `Limits::depth` allows
//! (256 by default, more when raised), and a definition, a defined library
//! and a type each copy, print and compare their whole tree. None may need
//! more stack than the caller has: each deep case runs on a 64 KiB thread,
//! and `scripts/check.sh` runs this file unoptimised too (frames are largest
//! at opt-level 0).
//!
//! The same file proves the hand-written impls change nothing: a clone equals
//! its source, equality gives the answers a plain recursive comparison
//! gives, and `Debug` prints byte for byte what the derived impls printed
//! (`definition_traits_golden/golden.rs`: the strings `golden_texts` made,
//! written out once with the derived impls still in place, before #231
//! replaced them).

use std::fmt::{self, Write};

use openui_lang::{
    DefinedLibrary, LibraryDefinition, Limits, TypeDescription, create_library, free_json,
};
use serde_json::{Value, json};

#[path = "definition_traits_golden/golden.rs"]
mod golden;

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);

const SMALL_STACK: usize = 64 * 1024;

// ---------------------------------------------------------------------
// Building definitions
// ---------------------------------------------------------------------

/// `{"kind": kind, field: inner}`, moving `inner` in (`json!` would copy it,
/// recursively).
fn holding(kind: &str, field: &str, inner: Value) -> Value {
    let mut ty = json!({"kind": kind});
    ty[field] = inner;
    ty
}

fn in_array(ty: Value) -> Value {
    holding("array", "items", ty)
}

fn in_min_array(ty: Value) -> Value {
    let mut array = in_array(ty);
    array["minItems"] = json!(2);
    array
}

fn in_object(ty: Value) -> Value {
    let mut field = json!({"name": "f", "optional": true});
    field["type"] = ty;
    holding("object", "fields", Value::Array(vec![field]))
}

fn in_union(ty: Value) -> Value {
    holding(
        "union",
        "options",
        Value::Array(vec![json!({"kind": "number"}), ty]),
    )
}

fn in_record(ty: Value) -> Value {
    holding("record", "values", ty)
}

fn in_nullable(ty: Value) -> Value {
    holding("nullable", "type", ty)
}

/// A named type, or a nullable around one: a named type names no named type.
fn in_named(ty: Value) -> Value {
    if ty["kind"] == "named" {
        return in_nullable(ty);
    }
    let mut named = holding("named", "type", ty);
    named["name"] = json!("N");
    named
}

type Wrap = fn(Value) -> Value;

const WRAPPERS: [(&str, Wrap); 7] = [
    ("object", in_object),
    ("array", in_array),
    ("union", in_union),
    ("nullable", in_nullable),
    ("record", in_record),
    ("named", in_named),
    ("min-array", in_min_array),
];

/// `leaf` wrapped in `depth - 1` levels, with a loop.
fn chain(depth: usize, wrap: Wrap, mut leaf: Value) -> Value {
    for _ in 1..depth {
        leaf = wrap(leaf);
    }
    leaf
}

/// Two components (`Deep` holds the prop `v` of type `ty`) and what else the
/// fragments may refer to.
fn definition_of(ty: Value, default: Option<Value>) -> Value {
    let mut prop = json!({"name": "v"});
    prop["type"] = ty;
    if let Some(default) = default {
        prop["default"] = default;
    }
    let mut component = json!({"name": "Deep", "description": ""});
    component["props"] = Value::Array(vec![prop]);
    let mut definition = json!({});
    definition["components"] = Value::Array(vec![
        component,
        json!({"name": "Other", "description": "", "props": []}),
    ]);
    definition
}

fn leaf_kind(variant: u8) -> Value {
    json!({"kind": if variant == 0 { "string" } else { "boolean" }})
}

/// A JSON default of `depth` arrays, each around an object, around a leaf
/// that differs by `variant`. Built with a loop.
fn deep_default(depth: usize, variant: u8) -> Value {
    let mut default = json!(if variant == 0 { "leaf" } else { "different" });
    for _ in 0..depth {
        let mut object = json!({});
        object["key"] = default;
        default = Value::Array(vec![object]);
    }
    default
}

/// A function returning a type `depth` levels deep, and a library action
/// whose one param is as deep, as the prop is, to put every kind of type
/// slot at the limit.
fn calls_definition(depth: usize, variant: u8) -> Value {
    let deep = || chain(depth, in_array, leaf_kind(variant));
    let mut definition = json!({
        "components": [{"name": "Deep", "description": "", "props": [
            {"name": "go", "type": {"kind": "action", "name": "Go"}}
        ]}],
        "functions": [{"name": "F", "description": "", "params": []}],
        "actions": [{"name": "Go", "description": "", "params": [{"name": "p"}]}]
    });
    definition["functions"][0]["returns"] = deep();
    definition["functions"][0]["params"] = json!([{"name": "q"}]);
    definition["functions"][0]["params"][0]["type"] = deep();
    definition["actions"][0]["params"][0]["type"] = deep();
    definition
}

#[derive(Clone, Copy, Debug)]
enum Shape {
    Wrapped(&'static str, Wrap),
    JsonDefault,
    Calls,
}

impl Shape {
    fn name(self) -> &'static str {
        match self {
            Shape::Wrapped(name, _) => name,
            Shape::JsonDefault => "json-default",
            Shape::Calls => "calls",
        }
    }
}

fn shapes() -> Vec<Shape> {
    let mut shapes: Vec<Shape> = WRAPPERS
        .iter()
        .map(|&(name, wrap)| Shape::Wrapped(name, wrap))
        .collect();
    shapes.push(Shape::JsonDefault);
    shapes.push(Shape::Calls);
    shapes
}

fn definition_json(shape: Shape, depth: usize, variant: u8) -> Value {
    match shape {
        Shape::Wrapped(_, wrap) => definition_of(chain(depth, wrap, leaf_kind(variant)), None),
        Shape::JsonDefault => definition_of(
            json!({"kind": "string"}),
            Some(deep_default(depth, variant)),
        ),
        Shape::Calls => calls_definition(depth, variant),
    }
}

fn read(json: Value, depth: usize) -> LibraryDefinition {
    let limits = Limits::default().with_depth(depth.max(Limits::default().depth()));
    let definition = LibraryDefinition::from_json_with_limits(&json, limits).unwrap();
    free_json(json);
    definition
}

fn read_shape(shape: Shape, depth: usize, variant: u8) -> LibraryDefinition {
    read(definition_json(shape, depth, variant), depth)
}

/// The type a case is about: a prop's, or a function's return.
fn main_type(shape: Shape, definition: &LibraryDefinition) -> &TypeDescription {
    match shape {
        Shape::Calls => definition.functions[0].returns.as_ref().unwrap(),
        _ => &definition.components[0].props[0].ty,
    }
}

fn on_small_stack(work: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(SMALL_STACK)
        .spawn(work)
        .unwrap()
        .join()
        .unwrap();
}

// ---------------------------------------------------------------------
// Deep values on a 64 KiB thread
// ---------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
enum Subject {
    Type,
    Definition,
    Library,
}

#[derive(Clone, Copy, Debug)]
enum Operation {
    Clone,
    Compact,
    Pretty,
    Equal,
}

/// Bytes written, not kept: pretty `Debug` indents each line by its depth,
/// so its text grows with the square of the depth.
#[derive(Default)]
struct ByteCount(usize);

impl fmt::Write for ByteCount {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        self.0 += text.len();
        Ok(())
    }
}

fn exercise<T: Clone + fmt::Debug + PartialEq>(
    operation: Operation,
    [original, equal, different]: [&T; 3],
    what: &str,
) {
    match operation {
        Operation::Clone => {
            let copy = original.clone();
            assert!(&copy == original, "{what}: a clone equals its source");
            assert!(&copy != different, "{what}: and not another value");
        }
        Operation::Equal => {
            assert!(original == equal, "{what}: equal values");
            assert!(equal == original, "{what}: equality is symmetric");
            assert!(
                original != different,
                "{what}: values that differ deep inside"
            );
            assert!(different != original, "{what}: both ways");
        }
        Operation::Compact => {
            let text = format!("{original:?}");
            assert_eq!(text, format!("{equal:?}"), "{what}");
            assert_ne!(text, format!("{different:?}"), "{what}");
        }
        Operation::Pretty => {
            let mut count = ByteCount::default();
            write!(&mut count, "{original:#?}").unwrap();
            let mut other = ByteCount::default();
            write!(&mut other, "{equal:#?}").unwrap();
            assert_eq!(count.0, other.0, "{what}");
            // Pretty output has an indent on most lines: more than compact.
            assert!(count.0 > format!("{original:?}").len(), "{what}");
        }
    }
}

fn deep(subject: Subject, operation: Operation) {
    on_small_stack(move || {
        for shape in shapes() {
            for depth in [256, 1_024] {
                let what = format!("{subject:?} {operation:?} {} depth {depth}", shape.name());
                let definitions = [0, 0, 1].map(|variant| read_shape(shape, depth, variant));
                match subject {
                    Subject::Type => {
                        let [a, b, c] = &definitions;
                        exercise(
                            operation,
                            [
                                main_type(shape, a),
                                main_type(shape, b),
                                main_type(shape, c),
                            ],
                            &what,
                        );
                    }
                    Subject::Definition => {
                        exercise(operation, definitions.each_ref(), &what);
                    }
                    Subject::Library => {
                        let libraries = definitions.map(|d| create_library(d).unwrap());
                        exercise(operation, libraries.each_ref(), &what);
                    }
                }
            }
        }
    });
}

macro_rules! deep_tests {
    ($($name:ident: $subject:ident, $operation:ident;)*) => {
        $(
            #[test]
            fn $name() {
                deep(Subject::$subject, Operation::$operation);
            }
        )*
    };
}

deep_tests! {
    type_description_clone_on_a_small_stack: Type, Clone;
    type_description_debug_on_a_small_stack: Type, Compact;
    type_description_pretty_debug_on_a_small_stack: Type, Pretty;
    type_description_equality_on_a_small_stack: Type, Equal;
    library_definition_clone_on_a_small_stack: Definition, Clone;
    library_definition_debug_on_a_small_stack: Definition, Compact;
    library_definition_pretty_debug_on_a_small_stack: Definition, Pretty;
    library_definition_equality_on_a_small_stack: Definition, Equal;
    defined_library_clone_on_a_small_stack: Library, Clone;
    defined_library_debug_on_a_small_stack: Library, Compact;
    defined_library_pretty_debug_on_a_small_stack: Library, Pretty;
    defined_library_equality_on_a_small_stack: Library, Equal;
}

/// The array chain's text is known exactly, at the limit and at the depth a
/// caller who raised it can use.
#[test]
fn a_deep_array_chain_prints_exactly_on_a_small_stack() {
    on_small_stack(|| {
        for depth in [256, 1_024] {
            let definition = read_shape(Shape::Wrapped("array", in_array), depth, 0);
            let ty = &definition.components[0].props[0].ty;
            let expected = format!(
                "{}String{}",
                "Array(".repeat(depth - 1),
                ")".repeat(depth - 1)
            );
            assert_eq!(format!("{ty:?}"), expected);
            let text = format!("{definition:?}");
            assert!(text.contains(&format!("ty: {expected},")), "depth {depth}");
        }
    });
}

/// Built with the public constructors, a type may nest past any limit the
/// reader has, so the three operations hold on deeper trees than it reads.
#[test]
fn a_type_built_directly_nests_to_any_depth_on_a_small_stack() {
    on_small_stack(|| {
        let build = |leaf: TypeDescription| {
            let mut ty = leaf;
            for _ in 0..100_000 {
                ty = TypeDescription::Array(Box::new(ty));
            }
            ty
        };
        let ty = build(TypeDescription::String);
        let equal = build(TypeDescription::String);
        let different = build(TypeDescription::Boolean);
        let copy = ty.clone();
        assert!(copy == ty && ty == equal && ty != different);
        let text = format!("{copy:?}");
        assert_eq!(text.len(), 100_000 * "Array()".len() + "String".len());
        let definition = LibraryDefinition {
            components: vec![openui_lang::ComponentDefinition {
                name: "Deep".to_owned(),
                description: String::new(),
                props: vec![openui_lang::PropDefinition {
                    name: "v".to_owned(),
                    ty: copy,
                    optional: false,
                    binding: false,
                }],
            }],
            component_groups: None,
            functions: vec![],
            actions: vec![],
            root: None,
            id: None,
        };
        let copy = definition.clone();
        assert!(copy == definition);
        assert!(format!("{copy:?}").contains(&text));
    });
}

// ---------------------------------------------------------------------
// The same answers as before
// ---------------------------------------------------------------------

/// A plain recursive comparison, written out for every kind of type.
fn reference_equal(a: &TypeDescription, b: &TypeDescription) -> bool {
    use TypeDescription as T;
    match (a, b) {
        (T::String, T::String)
        | (T::Number, T::Number)
        | (T::Boolean, T::Boolean)
        | (T::Any, T::Any)
        | (T::Unknown, T::Unknown) => true,
        (T::Enum(a), T::Enum(b)) => a == b,
        (T::Array(a), T::Array(b))
        | (T::Record(a), T::Record(b))
        | (T::Nullable(a), T::Nullable(b)) => reference_equal(a, b),
        (T::ArrayWithMinimum(a, min_a), T::ArrayWithMinimum(b, min_b)) => {
            min_a == min_b && reference_equal(a, b)
        }
        (T::Union(a), T::Union(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| reference_equal(a, b))
        }
        (T::Object(a), T::Object(b)) => {
            a.len() == b.len()
                && a.iter().zip(b).all(|(a, b)| {
                    a.name == b.name && a.optional == b.optional && reference_equal(&a.ty, &b.ty)
                })
        }
        (T::Default(a, value_a), T::Default(b, value_b)) => {
            value_a.as_ref() == value_b.as_ref() && reference_equal(a, b)
        }
        (T::Component(a), T::Component(b)) => a == b,
        (T::Action(a), T::Action(b)) => a == b,
        (T::Named(name_a, a), T::Named(name_b, b)) => name_a == name_b && reference_equal(a, b),
        (a, b) => {
            // Every pair of different kinds is unequal; this arm must not
            // hide a kind the match above forgot.
            assert_ne!(std::mem::discriminant(a), std::mem::discriminant(b));
            false
        }
    }
}

/// Type fragments that differ in one place each, so that wrapped in the same
/// chain they differ only deep inside.
fn fragments() -> Vec<Value> {
    let default = |value: Value| {
        json!({"kind": "object", "fields": [
            {"name": "f", "type": {"kind": "string"}, "default": value}
        ]})
    };
    let field = |name: &str, optional: bool| {
        json!({"kind": "object", "fields": [
            {"name": name, "type": {"kind": "number"}, "optional": optional}
        ]})
    };
    vec![
        json!({"kind": "string"}),
        json!({"kind": "number"}),
        json!({"kind": "boolean"}),
        json!({"kind": "any"}),
        json!({"kind": "unknown"}),
        json!({"kind": "enum", "values": ["a", "b"]}),
        json!({"kind": "enum", "values": ["b", "a"]}),
        json!({"kind": "enum", "values": ["a"]}),
        json!({"kind": "component", "name": "Deep"}),
        json!({"kind": "component", "name": "Other"}),
        json!({"kind": "action"}),
        json!({"kind": "action", "name": "OpenUrl"}),
        json!({"kind": "array", "items": {"kind": "number"}, "minItems": 2}),
        json!({"kind": "array", "items": {"kind": "number"}, "minItems": 3}),
        json!({"kind": "array", "items": {"kind": "number"}}),
        field("f", false),
        field("f", true),
        field("g", false),
        json!({"kind": "object", "fields": []}),
        json!({"kind": "union", "options": []}),
        default(json!({"a": [1, 2], "b": null})),
        default(json!({"a": [1, 3], "b": null})),
        default(json!({"b": null, "a": [1, 2]})),
        default(json!({"a": [1, 2.5], "b": null})),
        default(json!(null)),
        default(json!("s")),
        json!({"kind": "named", "name": "N", "type": {"kind": "string"}}),
        json!({"kind": "named", "name": "M", "type": {"kind": "string"}}),
        json!({"kind": "named", "name": "N", "type": {"kind": "number"}}),
    ]
}

/// Many shapes at depths 1 to 20, in pairs that are equal, differ at the
/// top, or differ only at the bottom.
fn medium_values() -> Vec<(String, LibraryDefinition)> {
    let mut values = Vec::new();
    for (index, fragment) in fragments().into_iter().enumerate() {
        for (name, wrap) in WRAPPERS {
            for depth in [1, 4, 20] {
                let json = definition_of(chain(depth, wrap, fragment.clone()), None);
                values.push((
                    format!("{name} {depth} fragment {index}"),
                    read(json, depth),
                ));
            }
        }
    }
    for depth in [1, 5, 20] {
        for variant in 0..2 {
            values.push((
                format!("default {depth} {variant}"),
                read_shape(Shape::JsonDefault, depth, variant),
            ));
            values.push((
                format!("calls {depth} {variant}"),
                read_shape(Shape::Calls, depth, variant),
            ));
        }
    }
    values
}

/// What the old derived equality said of two definitions that share all but
/// one type: the answer for that type. A definition's other parts agree, and
/// the calls shape's types are compared too.
fn reference_definitions_equal(a: &LibraryDefinition, b: &LibraryDefinition) -> bool {
    let types = |d: &LibraryDefinition| {
        let mut types = vec![d.components[0].props[0].ty.clone()];
        if let Some(function) = d.functions.first() {
            types.extend(function.returns.clone());
            types.extend(function.params.iter().map(|p| p.ty.clone()));
            types.extend(d.actions[0].params.iter().map(|p| p.ty.clone()));
        }
        types
    };
    let (a_types, b_types) = (types(a), types(b));
    a.components.len() == b.components.len()
        && a.functions.len() == b.functions.len()
        && a_types.len() == b_types.len()
        && a_types
            .iter()
            .zip(&b_types)
            .all(|(a, b)| reference_equal(a, b))
}

#[test]
fn clones_equal_their_sources_and_equality_matches_a_recursive_comparison() {
    // `create_library` refuses a few of the combinations (a named type
    // around a component reference, an action slot or another named type):
    // those are left out.
    let (values, libraries): (Vec<_>, Vec<DefinedLibrary>) = medium_values()
        .into_iter()
        .filter_map(|(name, d)| Some((create_library(d.clone()).ok()?, (name, d))))
        .map(|(library, value)| (value, library))
        .unzip();
    assert!(values.len() > 500, "{}", values.len());
    let mut equal_pairs = 0;
    for (i, (name_a, a)) in values.iter().enumerate() {
        let copy = a.clone();
        assert!(copy == *a, "{name_a}: a clone equals its source");
        assert_eq!(format!("{copy:?}"), format!("{a:?}"), "{name_a}");
        assert_eq!(format!("{copy:#?}"), format!("{a:#?}"), "{name_a}");
        assert!(libraries[i].clone() == libraries[i], "{name_a}");
        assert!(a.components[0].props[0].ty.clone() == a.components[0].props[0].ty);
        for (j, (name_b, b)) in values.iter().enumerate() {
            let expected = reference_definitions_equal(a, b);
            equal_pairs += usize::from(expected);
            assert_eq!(a == b, expected, "{name_a} == {name_b}");
            assert_eq!(
                libraries[i] == libraries[j],
                expected,
                "{name_a} == {name_b}"
            );
            let (ty_a, ty_b) = (&a.components[0].props[0].ty, &b.components[0].props[0].ty);
            if a.functions.is_empty() && b.functions.is_empty() {
                assert_eq!(
                    ty_a == ty_b,
                    reference_equal(ty_a, ty_b),
                    "{name_a} == {name_b}"
                );
            }
        }
    }
    // The pairs include many equal ones (each value with itself and the
    // duplicates fragments make) and many that differ.
    assert!(equal_pairs > values.len() && equal_pairs < values.len() * values.len() / 2);
}

/// Cases for the golden text, each a definition. The type the first prop
/// holds is printed alone as well.
fn golden_cases() -> Vec<(&'static str, Value, bool)> {
    let wide = {
        let mut definition = definition_of(
            json!({"kind": "union", "options": [
                {"kind": "array", "minItems": 2, "items": {"kind": "number"}},
                {"kind": "object", "fields": [
                    {"name": "f", "type": {"kind": "string"}, "optional": true,
                     "default": {"b": [true, null, -1.5, "s\"q"], "a": {}, "c": []}}
                ]},
                {"kind": "enum", "values": ["x", "y\n"]},
                {"kind": "component", "name": "Other"},
                {"kind": "action", "name": "OpenUrl"},
                {"kind": "action"},
                {"kind": "record", "values": {"kind": "boolean"}},
                {"kind": "nullable", "type": {"kind": "any"}},
                {"kind": "named", "name": "Label", "type": {"kind": "unknown"}},
                {"kind": "object", "fields": []},
                {"kind": "union", "options": []}
            ]}),
            Some(json!({"x": [1, {"y": null}]})),
        );
        definition["componentGroups"] = json!([{"name": "G", "components": ["Deep", "Other"], "notes": ["n1", "n2"]},
                   {"name": "H", "components": []}]);
        definition["functions"] = json!([{"name": "F", "description": "d", "params": [
            {"name": "p", "type": {"kind": "array", "items": {"kind": "string"}}, "optional": true}
        ], "returns": {"kind": "nullable", "type": {"kind": "number"}}}]);
        definition["actions"] = json!([{"name": "Go", "description": "g", "params": [
            {"name": "q", "type": {"kind": "object", "fields": [
                {"name": "r", "type": {"kind": "boolean"}}]}}
        ]}]);
        definition["root"] = json!("Deep");
        definition["id"] = json!("lib");
        definition
    };
    let mut cases = vec![("wide", wide, true)];
    for (name, wrap) in WRAPPERS {
        cases.push((
            Box::leak(format!("{name}-chain-20").into_boxed_str()) as &'static str,
            definition_of(chain(20, wrap, leaf_kind(0)), None),
            name == "object" || name == "union",
        ));
    }
    cases.push((
        "object-chain-20-differing-leaf",
        definition_of(chain(20, in_object, leaf_kind(1)), None),
        false,
    ));
    cases.push((
        "json-default-6",
        definition_of(json!({"kind": "string"}), Some(deep_default(6, 0))),
        true,
    ));
    cases
}

/// Every (case, what, form) string and what prints it now.
fn golden_texts() -> Vec<(String, String, String, String)> {
    let mut texts = Vec::new();
    for (name, json, library_level) in golden_cases() {
        let definition = read(json, 20);
        let ty = &definition.components[0].props[0].ty;
        let mut push = |what: &str, compact: String, pretty: String| {
            texts.push((
                name.to_owned(),
                what.to_owned(),
                "compact".to_owned(),
                compact,
            ));
            texts.push((
                name.to_owned(),
                what.to_owned(),
                "pretty".to_owned(),
                pretty,
            ));
        };
        push("type", format!("{ty:?}"), format!("{ty:#?}"));
        if library_level {
            push(
                "definition",
                format!("{definition:?}"),
                format!("{definition:#?}"),
            );
            let library = create_library(definition.clone()).unwrap();
            push("library", format!("{library:?}"), format!("{library:#?}"));
        }
    }
    texts
}

#[test]
fn debug_prints_what_the_derived_impls_printed() {
    let texts = golden_texts();
    assert_eq!(texts.len(), golden::GOLDEN.len(), "a golden went missing");
    for ((name, what, form, text), (g_name, g_what, g_form, golden)) in
        texts.iter().zip(golden::GOLDEN)
    {
        assert_eq!(
            (name.as_str(), what.as_str(), form.as_str()),
            (*g_name, *g_what, *g_form)
        );
        assert_eq!(text, golden, "{name} {what} {form}");
    }
}
