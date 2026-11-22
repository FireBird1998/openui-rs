//! A component library built from a [`LibraryDefinition`]: upstream's
//! `createLibrary` (its `library.ts`), with its `toJSONSchema()` and
//! `toSpec()`. Upstream asks zod 4 for the JSON Schema; this writes what zod
//! 4.6.7 (the version in the reference harness) writes for the equivalent
//! declaration, byte for byte, and builds the signatures as upstream's
//! `signature.ts` does ([`crate::signature`]).
//!
//! The parser never sees the definition: [`DefinedLibrary::library`] compiles
//! the JSON Schema with [`Library::from_json_schema`], the parser's one way
//! in, so a defined library parses exactly as its JSON would.
//!
//! What upstream does with JavaScript objects is copied too. The components
//! are a record keyed by name, so two of one name keep the first one's place
//! with the later one's declaration, and a component named `__proto__` is lost
//! (assigning it sets the record's prototype). The root check,
//! `!record[root]`, is a truthiness test: it accepts a name every object
//! inherits (`toString`), and, when there is a `__proto__` component, a field
//! of the last one, as `defineComponent` returns it: `name`, `props` and
//! `ref` always, `description` only when it isn't empty, and `component`, its
//! renderer. The plain-data form has no renderer; every real library has a
//! function there, so `component` is taken as present (the reference harness
//! gives its samples a function renderer so they compare like with like).
//! Names that are array indices come first, ascending, for components and
//! props alike.
//!
//! The kinds that link a library together follow upstream's registry: every
//! component's props are registered under its name, and the action shapes
//! (`ActionExpression`, `OpenUrl`, `ToAssistant`) under theirs unless the
//! record has a component of that name. zod writes a registered schema as a
//! `$ref` wherever it is used and gives it a `$defs` entry when it is met,
//! so the `$defs` come in the order a depth-first walk from the record's
//! components first meets each one. That walk visits each component once,
//! with a loop and a stack of its own, so a reference chain as long as the
//! library or a cycle costs nothing more; writing a component's schema stops
//! at each reference.
//!
//! Functions and actions follow upstream's `createLibrary` too: their names
//! are claimed in order, functions first, each against the built-ins, the
//! reserved calls, the record's components and the names claimed before it;
//! a library action's ref is registered under its name, so it is written as
//! a `$ref` with a `$defs` entry when a component refers to it; and a call's
//! params and return type are written with the same registry, their own
//! `$defs` left out (`toCallJSONSchema`).

use std::collections::{HashMap, HashSet};

use serde_json::{Map, Value as Json, json};

use crate::js::{JsObject, OBJECT_PROTOTYPE_KEYS};
use crate::library_definition::{
    ActionDefinition, ComponentDefinition, FieldDefinition, FunctionDefinition, LibraryDefinition,
    LibraryDefinitionError, PropDefinition, TypeDescription, check_references, in_property_order,
};
use crate::signature::{Entry, call_signature, enum_words};
use crate::{CallKind, Library, is_builtin, is_reserved_call};

/// The `$schema` zod 4 writes.
const DRAFT_2020_12: &str = "https://json-schema.org/draft/2020-12/schema";

/// The fields of the object upstream's `defineComponent` returns that are
/// always truthy, which a component named `__proto__` puts on the component
/// record's prototype. Its `description` is truthy only when it isn't empty;
/// `component`, the renderer, is taken as present (see the module docs).
const TRUTHY_DEFINED_COMPONENT_FIELDS: [&str; 4] = ["name", "props", "component", "ref"];

/// A component library made from a [`LibraryDefinition`] by
/// [`create_library`]: its JSON Schema, its prompt spec and the [`Library`]
/// the parser compiles from that JSON Schema.
#[derive(Clone, Debug, PartialEq)]
pub struct DefinedLibrary {
    definition: LibraryDefinition,
    /// Indices into `definition.components` of the components the record
    /// keeps, in JavaScript property order.
    components: Vec<usize>,
}

/// Upstream's `createLibrary`: checks the root and the names of the
/// functions and actions, and makes the library.
///
/// Refuses a root that names no component with
/// [`LibraryDefinitionError::RootNotFound`], whose message is upstream's.
/// An empty root isn't checked, as upstream doesn't check it. Then refuses a
/// function or action name that is a built-in, a reserved call, a
/// component's or an earlier function's or action's with
/// [`LibraryDefinitionError::NameCollision`], whose message is upstream's,
/// checking the functions first, each in order. Then refuses a
/// component reference to a name no component has with
/// [`LibraryDefinitionError::UnknownComponent`], an action slot naming an
/// action a slot can't name with [`LibraryDefinitionError::UnknownAction`],
/// and, as invalid, a reference to the component with the empty name and a
/// named type whose type is a reference, a slot or another named type, a
/// reference to the library action with the empty name, and a library
/// action named `ActionExpression`.
pub fn create_library(
    definition: LibraryDefinition,
) -> Result<DefinedLibrary, LibraryDefinitionError> {
    // `componentsRecord[comp.name] = comp`
    let mut record: JsObject<usize> = JsObject::with_capacity(definition.components.len());
    for (i, component) in definition.components.iter().enumerate() {
        record.set(component.name.as_str(), i);
    }
    // The record's prototype: the last component named `__proto__`.
    let prototype = definition
        .components
        .iter()
        .rev()
        .find(|c| c.name == "__proto__");
    if let Some(root) = definition.root.as_deref().filter(|root| !root.is_empty()) {
        // `!componentsRecord[input.root]`
        let found = record.has_own(root)
            || OBJECT_PROTOTYPE_KEYS.contains(&root)
            || prototype.is_some_and(|proto| {
                TRUTHY_DEFINED_COMPONENT_FIELDS.contains(&root)
                    || (root == "description" && !proto.description.is_empty())
            });
        if !found {
            return Err(LibraryDefinitionError::RootNotFound {
                root: root.to_owned(),
                available: record.iter().map(|(name, _)| name.clone()).collect(),
            });
        }
    }
    // `claim`: the record's component names, then each function's and
    // action's in turn.
    let mut names: HashSet<&str> = record.iter().map(|(name, _)| name.as_str()).collect();
    let calls = (definition.functions.iter())
        .map(|f| (CallKind::Function, f.name.as_str()))
        .chain((definition.actions.iter()).map(|a| (CallKind::Action, a.name.as_str())));
    for (kind, name) in calls {
        if is_builtin(name) || is_reserved_call(name) || !names.insert(name) {
            return Err(LibraryDefinitionError::NameCollision {
                kind,
                name: name.to_owned(),
            });
        }
    }
    check_references(&definition)?;
    let components = record.values().copied().collect();
    Ok(DefinedLibrary {
        definition,
        components,
    })
}

impl DefinedLibrary {
    /// The definition it was made from.
    pub fn definition(&self) -> &LibraryDefinition {
        &self.definition
    }

    fn components(&self) -> impl Iterator<Item = &ComponentDefinition> {
        self.components
            .iter()
            .filter_map(|&i| self.definition.components.get(i))
    }

    /// Upstream's `toJSONSchema()`: the library's JSON Schema, keys in the
    /// order zod writes them.
    pub fn to_json_schema(&self) -> Json {
        let registry = self.registry();
        let mut properties = Map::new();
        let mut required = Vec::new();
        for component in self.components() {
            required.push(Json::String(component.name.clone()));
            if component.name.is_empty() {
                // zod skips an empty id: the schema is written in place with
                // its `id`, it gets no `$defs` entry, and so upstream finds no
                // entry to merge the description into.
                let mut body = props_schema(component, &registry);
                body.insert("id".to_owned(), "".into());
                properties.insert(String::new(), Json::Object(body));
                continue;
            }
            properties.insert(component.name.clone(), reference(&component.name));
        }
        let mut defs = Map::new();
        for node in self.definition_order(&registry) {
            match node {
                Node::Component(i) => {
                    let Some(component) = self.definition.components.get(i) else {
                        continue;
                    };
                    if component.name.is_empty() {
                        continue;
                    }
                    let mut body = props_schema(component, &registry);
                    // Upstream merges a non-empty description in after zod's
                    // keys, for the components of its record.
                    if !component.description.is_empty()
                        && registry.record.contains(component.name.as_str())
                    {
                        body.insert(
                            "description".to_owned(),
                            component.description.as_str().into(),
                        );
                    }
                    defs.insert(component.name.clone(), Json::Object(body));
                }
                Node::Action(action) => {
                    defs.insert(action.name().to_owned(), action.schema());
                }
                Node::CustomAction(i) => {
                    if let Some(action) = self.definition.actions.get(i) {
                        defs.insert(action.name.clone(), custom_action_schema(action, &registry));
                    }
                }
            }
        }
        let mut schema = Map::new();
        schema.insert("$schema".to_owned(), DRAFT_2020_12.into());
        schema.insert("type".to_owned(), "object".into());
        schema.insert("properties".to_owned(), Json::Object(properties));
        if !required.is_empty() {
            schema.insert("required".to_owned(), Json::Array(required));
        }
        schema.insert("additionalProperties".to_owned(), false.into());
        if !defs.is_empty() {
            schema.insert("$defs".to_owned(), Json::Object(defs));
        }
        // `callSchemas`: each call's description, params and return type,
        // keyed by name; left out when there are none.
        let functions = self
            .functions()
            .map(|function| {
                let mut entry = Map::new();
                entry.insert(
                    "description".to_owned(),
                    function.description.as_str().into(),
                );
                entry.insert(
                    "params".to_owned(),
                    Json::Object(params_schema(&function.params, &registry)),
                );
                if let Some(returns) = &function.returns {
                    entry.insert("returns".to_owned(), type_schema(returns, &registry));
                }
                (function.name.clone(), Json::Object(entry))
            })
            .collect::<Map<_, _>>();
        if !functions.is_empty() {
            schema.insert("functions".to_owned(), Json::Object(functions));
        }
        let actions = self
            .actions()
            .map(|action| {
                let entry = object([
                    ("description", action.description.as_str().into()),
                    (
                        "params",
                        Json::Object(params_schema(&action.params, &registry)),
                    ),
                ]);
                (action.name.clone(), entry)
            })
            .collect::<Map<_, _>>();
        if !actions.is_empty() {
            schema.insert("actions".to_owned(), Json::Object(actions));
        }
        Json::Object(schema)
    }

    /// The functions, keyed by name: in JavaScript property order. Names
    /// are unique, as `create_library` checked.
    fn functions(&self) -> impl Iterator<Item = &FunctionDefinition> {
        in_property_order(&self.definition.functions, |f| &f.name).into_iter()
    }

    /// The library's own actions, keyed by name: in JavaScript property
    /// order.
    fn actions(&self) -> impl Iterator<Item = &ActionDefinition> {
        in_property_order(&self.definition.actions, |a| &a.name).into_iter()
    }

    /// Upstream's `toSpec()`: the prompt spec, with each component's
    /// signature built here, and the JSON Schema as its `schema`.
    pub fn to_spec(&self) -> Json {
        let mut spec = Map::new();
        if let Some(root) = &self.definition.root {
            spec.insert("root".to_owned(), root.as_str().into());
        }
        let components = self
            .components()
            .map(|component| {
                let entries = ordered_props(&component.props).into_iter().map(|p| Entry {
                    name: &p.name,
                    ty: &p.ty,
                    optional: p.optional,
                    binding: p.binding,
                });
                let entry = object([
                    (
                        "signature",
                        call_signature(&component.name, entries, None).into(),
                    ),
                    ("description", component.description.as_str().into()),
                ]);
                (component.name.clone(), entry)
            })
            .collect();
        spec.insert("components".to_owned(), Json::Object(components));
        if let Some(groups) = &self.definition.component_groups {
            let groups = groups
                .iter()
                .map(|group| {
                    let mut entry = Map::new();
                    entry.insert("name".to_owned(), group.name.as_str().into());
                    entry.insert("components".to_owned(), strings(&group.components));
                    if let Some(notes) = &group.notes {
                        entry.insert("notes".to_owned(), strings(notes));
                    }
                    Json::Object(entry)
                })
                .collect();
            spec.insert("componentGroups".to_owned(), Json::Array(groups));
        }
        // `buildCallSpecs`, left out when there are none.
        let call_spec = |name: &str, description: &str, params, returns| {
            object([
                (
                    "signature",
                    call_signature(name, param_entries(params), returns).into(),
                ),
                ("description", description.into()),
            ])
        };
        let functions: Map<String, Json> = self
            .functions()
            .map(|f| {
                let entry = call_spec(&f.name, &f.description, &f.params, f.returns.as_ref());
                (f.name.clone(), entry)
            })
            .collect();
        if !functions.is_empty() {
            spec.insert("functions".to_owned(), Json::Object(functions));
        }
        let actions: Map<String, Json> = self
            .actions()
            .map(|a| {
                (
                    a.name.clone(),
                    call_spec(&a.name, &a.description, &a.params, None),
                )
            })
            .collect();
        if !actions.is_empty() {
            spec.insert("actions".to_owned(), Json::Object(actions));
        }
        spec.insert("schema".to_owned(), self.to_json_schema());
        if let Some(id) = &self.definition.id {
            spec.insert("id".to_owned(), id.as_str().into());
        }
        Json::Object(spec)
    }

    /// The library the parser uses: [`Library::from_json_schema`] of
    /// [`DefinedLibrary::to_json_schema`].
    ///
    /// Writing the schema grows the stack as deep as the types go, and
    /// compiling it and freeing it use loops, so this needs no stack of its
    /// own at any depth, and costs time and memory in step with the
    /// definition's size: library actions that refer to each other share
    /// their shapes in the compiled library ([`Library::from_json_schema`]).
    pub fn library(&self) -> Library {
        let schema = self.to_json_schema();
        let library = Library::from_json_schema(&schema);
        crate::free_json(schema);
        library
    }

    /// Upstream's registry, for this library: which component each name
    /// refers to, and which names the record has.
    fn registry(&self) -> Registry<'_> {
        let mut last = HashMap::with_capacity(self.definition.components.len());
        for (i, component) in self.definition.components.iter().enumerate() {
            last.insert(component.name.as_str(), i);
        }
        Registry {
            last,
            record: self.components().map(|c| c.name.as_str()).collect(),
            actions: (self.definition.actions.iter().enumerate())
                .map(|(i, a)| (a.name.as_str(), i))
                .collect(),
        }
    }

    /// The registered schemas in the order zod first meets them: depth first
    /// from the record's components in order, each one's references in the
    /// order its props hold them. Each node's references are listed once,
    /// when it is first met, and the walk keeps its own stack, so this is
    /// linear in the library's size whatever its references' shape.
    fn definition_order(&self, registry: &Registry<'_>) -> Vec<Node> {
        let count = self.definition.components.len();
        let mut seen_components = vec![false; count];
        let mut seen_actions = [false; 3];
        let mut seen_custom = vec![false; self.definition.actions.len()];
        let mut order = Vec::new();
        let mut stack: Vec<(Vec<Node>, usize)> = Vec::new();
        let mut visit = |node: Node, order: &mut Vec<Node>| -> Option<Vec<Node>> {
            let seen = match node {
                Node::Component(i) => seen_components.get_mut(i)?,
                Node::Action(action) => seen_actions.get_mut(action as usize)?,
                Node::CustomAction(i) => seen_custom.get_mut(i)?,
            };
            if *seen {
                return None;
            }
            *seen = true;
            order.push(node);
            let mut edges = Vec::new();
            match node {
                Node::Component(i) => {
                    if let Some(component) = self.definition.components.get(i) {
                        for prop in ordered_props(&component.props) {
                            references(&prop.ty, registry, &mut edges);
                        }
                    }
                }
                // The action's data: `type` (a literal), then its params.
                Node::CustomAction(i) => {
                    if let Some(action) = self.definition.actions.get(i) {
                        for param in in_property_order(&action.params, |p| &p.name) {
                            references(&param.ty, registry, &mut edges);
                        }
                    }
                }
                Node::Action(_) => {}
            }
            Some(edges)
        };
        for &root in &self.components {
            if let Some(edges) = visit(Node::Component(root), &mut order) {
                stack.push((edges, 0));
            }
            while let Some((edges, next)) = stack.last_mut() {
                let Some(&node) = edges.get(*next) else {
                    stack.pop();
                    continue;
                };
                *next += 1;
                if let Some(edges) = visit(node, &mut order) {
                    stack.push((edges, 0));
                }
            }
        }
        order
    }
}

/// What a name refers to, as upstream's registry and component record say.
struct Registry<'a> {
    /// Each component name's last component, the one a reference means.
    last: HashMap<&'a str, usize>,
    /// The names the component record has (all but `__proto__`).
    record: HashSet<&'a str>,
    /// The library's own actions by name, each registered under its name.
    actions: HashMap<&'a str, usize>,
}

impl Registry<'_> {
    /// Whether an action's shape is registered: unless the record has a
    /// component of its name (`if (!componentsRecord[id]) reg.add(...)`).
    fn registers(&self, action: ActionShape) -> bool {
        !self.record.contains(action.name())
    }
}

/// A registered schema: a component's props, a built-in action shape, or a
/// library action's data.
#[derive(Clone, Copy)]
enum Node {
    /// An index into the definition's components.
    Component(usize),
    Action(ActionShape),
    /// An index into the definition's actions.
    CustomAction(usize),
}

/// The built-in action shapes, upstream's `ACTION_DEFS`.
#[derive(Clone, Copy)]
enum ActionShape {
    Expression = 0,
    OpenUrl = 1,
    ToAssistant = 2,
}

impl ActionShape {
    fn of(name: Option<&str>) -> Option<ActionShape> {
        match name {
            None => Some(ActionShape::Expression),
            Some("OpenUrl") => Some(ActionShape::OpenUrl),
            Some("ToAssistant") => Some(ActionShape::ToAssistant),
            Some(_) => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            ActionShape::Expression => "ActionExpression",
            ActionShape::OpenUrl => "OpenUrl",
            ActionShape::ToAssistant => "ToAssistant",
        }
    }

    /// What zod writes for the shape, from lang-core's built-ins:
    /// `actionSchema` is a union of the open-url data, the to-assistant data
    /// and a generic `{type, params?}`; a named step's is its data.
    fn schema(self) -> Json {
        match self {
            ActionShape::Expression => json!({"anyOf": [
                open_url_data(),
                to_assistant_data(),
                {
                    "type": "object",
                    "properties": {
                        "type": {"type": "string"},
                        "params": {
                            "type": "object",
                            "propertyNames": {"type": "string"},
                            "additionalProperties": {}
                        }
                    },
                    "required": ["type"],
                    "additionalProperties": false
                }
            ]}),
            ActionShape::OpenUrl => open_url_data(),
            ActionShape::ToAssistant => to_assistant_data(),
        }
    }
}

/// `z.object({ type: z.literal("open_url"), url: z.string() })`.
fn open_url_data() -> Json {
    json!({
        "type": "object",
        "properties": {
            "type": {"type": "string", "const": "open_url"},
            "url": {"type": "string"}
        },
        "required": ["type", "url"],
        "additionalProperties": false
    })
}

/// `z.object({ type: z.literal("continue_conversation"), context:
/// z.string().optional() })`.
fn to_assistant_data() -> Json {
    json!({
        "type": "object",
        "properties": {
            "type": {"type": "string", "const": "continue_conversation"},
            "context": {"type": "string"}
        },
        "required": ["type"],
        "additionalProperties": false
    })
}

/// The registered schemas a type uses, in the order zod meets them. The
/// walk goes as deep as the type and no further: a reference is a node to
/// visit later, not followed here.
fn references(ty: &TypeDescription, registry: &Registry<'_>, out: &mut Vec<Node>) {
    crate::stack::grow_stack!(references(ty, registry, out));
    match ty {
        TypeDescription::Component(name) => {
            if let Some(&i) = registry.last.get(name.as_str()) {
                out.push(Node::Component(i));
            }
        }
        TypeDescription::Action(name) => match ActionShape::of(name.as_deref()) {
            Some(action) => {
                if registry.registers(action) {
                    out.push(Node::Action(action));
                }
            }
            None => {
                if let Some(&i) = name.as_deref().and_then(|n| registry.actions.get(n)) {
                    out.push(Node::CustomAction(i));
                }
            }
        },
        TypeDescription::Array(t)
        | TypeDescription::Record(t)
        | TypeDescription::Nullable(t)
        | TypeDescription::Named(_, t) => references(t, registry, out),
        TypeDescription::Union(options) => {
            for option in options {
                references(option, registry, out);
            }
        }
        TypeDescription::Object(fields) => {
            for field in in_property_order(fields, |f| &f.name) {
                references(&field.ty, registry, out);
            }
        }
        TypeDescription::String
        | TypeDescription::Number
        | TypeDescription::Boolean
        | TypeDescription::Any
        | TypeDescription::Enum(_) => {}
    }
}

/// `{"$ref": "#/$defs/<name>"}`, the name escaped as a JSON Pointer token.
fn reference(name: &str) -> Json {
    let target = format!("#/$defs/{}", json_pointer_token(name));
    object([("$ref", target.into())])
}

/// What `defineAction` registers under a library action's name: `z.object({
/// type: z.literal(name), params })`.
fn custom_action_schema(action: &ActionDefinition, registry: &Registry<'_>) -> Json {
    object([
        ("type", "object".into()),
        (
            "properties",
            object([
                (
                    "type",
                    object([
                        ("type", "string".into()),
                        ("const", action.name.as_str().into()),
                    ]),
                ),
                (
                    "params",
                    Json::Object(params_schema(&action.params, registry)),
                ),
            ]),
        ),
        ("required", json!(["type", "params"])),
        ("additionalProperties", false.into()),
    ])
}

/// A call's params as zod writes its object schema, in property order.
fn params_schema(params: &[FieldDefinition], registry: &Registry<'_>) -> Map<String, Json> {
    object_schema(
        in_property_order(params, |p| &p.name)
            .into_iter()
            .map(|p| (p.name.as_str(), &p.ty, p.optional)),
        registry,
    )
}

/// A call's params as signature entries, in property order; a param is no
/// binding.
fn param_entries(params: &[FieldDefinition]) -> impl Iterator<Item = Entry<'_>> {
    in_property_order(params, |p| &p.name)
        .into_iter()
        .map(|p| Entry {
            name: &p.name,
            ty: &p.ty,
            optional: p.optional,
            binding: false,
        })
}

/// A component's props as zod writes its object schema.
fn props_schema(component: &ComponentDefinition, registry: &Registry<'_>) -> Map<String, Json> {
    object_schema(
        ordered_props(&component.props)
            .into_iter()
            .map(|p| (p.name.as_str(), &p.ty, p.optional)),
        registry,
    )
}

/// An object's entries (a component's props, or an inline object's fields,
/// already in property order) as zod writes its object schema: a nullable
/// entry stays in `required`, an optional one is left out.
fn object_schema<'a>(
    entries: impl Iterator<Item = (&'a str, &'a TypeDescription, bool)>,
    registry: &Registry<'_>,
) -> Map<String, Json> {
    let mut properties = Map::new();
    let mut required = Vec::new();
    for (name, ty, optional) in entries {
        properties.insert(name.to_owned(), type_schema(ty, registry));
        if !optional {
            required.push(Json::String(name.to_owned()));
        }
    }
    let mut schema = Map::new();
    schema.insert("type".to_owned(), "object".into());
    schema.insert("properties".to_owned(), Json::Object(properties));
    if !required.is_empty() {
        schema.insert("required".to_owned(), Json::Array(required));
    }
    schema.insert("additionalProperties".to_owned(), false.into());
    schema
}

/// What zod writes for a type. The walk goes as deep as the type, which
/// reading bounded (see the `library_definition` module docs), growing the
/// stack on demand; a reference is written as a `$ref`, not followed.
fn type_schema(ty: &TypeDescription, registry: &Registry<'_>) -> Json {
    crate::stack::grow_stack!(type_schema(ty, registry));
    let schema = |ty| type_schema(ty, registry);
    match ty {
        TypeDescription::String => object([("type", "string".into())]),
        TypeDescription::Number => object([("type", "number".into())]),
        TypeDescription::Boolean => object([("type", "boolean".into())]),
        TypeDescription::Any => Json::Object(Map::new()),
        TypeDescription::Enum(values) => {
            let words = enum_words(values);
            if words.is_empty() {
                // An empty enum accepts nothing, as z.never().
                return object([("not", Json::Object(Map::new()))]);
            }
            let words = words.into_iter().map(|w| Json::String(w.to_owned()));
            object([
                ("type", "string".into()),
                ("enum", Json::Array(words.collect())),
            ])
        }
        TypeDescription::Array(items) => {
            object([("type", "array".into()), ("items", schema(items))])
        }
        TypeDescription::Union(options) => any_of(options.iter().map(schema).collect()),
        TypeDescription::Object(fields) => Json::Object(object_schema(
            in_property_order(fields, |f| &f.name)
                .into_iter()
                .map(|f| (f.name.as_str(), &f.ty, f.optional)),
            registry,
        )),
        TypeDescription::Record(values) => object([
            ("type", "object".into()),
            ("propertyNames", object([("type", "string".into())])),
            ("additionalProperties", schema(values)),
        ]),
        TypeDescription::Nullable(inner) => {
            any_of(vec![schema(inner), object([("type", "null".into())])])
        }
        // A name is for the prompt only.
        TypeDescription::Named(_, inner) => schema(inner),
        TypeDescription::Component(name) => reference(name),
        TypeDescription::Action(name) => match ActionShape::of(name.as_deref()) {
            Some(action) if registry.registers(action) => reference(action.name()),
            Some(action) => action.schema(),
            // A library action, registered under its name; create_library
            // refused any other name and the empty one.
            None => reference(name.as_deref().unwrap_or_default()),
        },
    }
}

/// `{"anyOf": options}`, compacted as zod 4.6.7's `compactTypeUnion` does:
/// when every option is a bare type assertion (an object whose only key is
/// `type`, a string or a list of strings), the options become one `type`,
/// each type once in first-seen order, a single one as a string. The options
/// were compacted when they were written, as zod compacts each one first.
fn any_of(options: Vec<Json>) -> Json {
    let mut types: Vec<&str> = Vec::new();
    let bare = !options.is_empty()
        && options.iter().all(|option| {
            bare_types(option).is_some_and(|members| {
                for member in members {
                    if !types.contains(&member) {
                        types.push(member);
                    }
                }
                true
            })
        });
    if !bare {
        return object([("anyOf", Json::Array(options))]);
    }
    let ty = match types.as_slice() {
        [single] => Json::String((*single).to_owned()),
        _ => Json::Array(
            types
                .iter()
                .map(|t| Json::String((*t).to_owned()))
                .collect(),
        ),
    };
    object([("type", ty)])
}

/// The types a bare type assertion (`{"type": "string"}` or `{"type":
/// ["string", "null"]}`, nothing else) allows; `None` for anything else.
fn bare_types(option: &Json) -> Option<Vec<&str>> {
    let map = option.as_object().filter(|m| m.len() == 1)?;
    match map.get("type")? {
        Json::String(member) => Some(vec![member.as_str()]),
        Json::Array(members) => members.iter().map(Json::as_str).collect(),
        _ => None,
    }
}

/// The props in JavaScript property order, as a zod shape iterates them.
fn ordered_props(props: &[PropDefinition]) -> Vec<&PropDefinition> {
    in_property_order(props, |p| &p.name)
}

/// A name as one JSON Pointer reference token: `~` as `~0`, `/` as `~1`.
fn json_pointer_token(name: &str) -> std::borrow::Cow<'_, str> {
    if name.contains(['~', '/']) {
        name.replace('~', "~0").replace('/', "~1").into()
    } else {
        name.into()
    }
}

fn object<const N: usize>(entries: [(&str, Json); N]) -> Json {
    Json::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

fn strings(items: &[String]) -> Json {
    Json::Array(items.iter().map(|s| Json::String(s.clone())).collect())
}
