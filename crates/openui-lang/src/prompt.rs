//! System prompt generation, as the reference package's `generatePrompt` and
//! `generateSystemPrompt`: the same text for the same spec, section by section.
//!
//! The spec is taken as JSON, the shape the reference documents, and read the
//! way JavaScript would: `??` defaults only replace a missing or null value,
//! `if (x)` tests truthiness, `${x}` stringifies, and object keys come in
//! property order. So a spec that round-trips through JSON gives the same
//! prompt here as there.

use std::fmt;

use serde_json::Value as Json;

use crate::js::{js_string, json_entries, json_value_string, truthy};

/// Why a system prompt couldn't be generated: the reference throws in the
/// same cases, with the same message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromptError(String);

impl fmt::Display for PromptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PromptError {}

/// The reference's `generateSystemPrompt`. Takes `{library, promptOptions}`,
/// a prompt spec directly (the deprecated form), or `{cloud: true, ...}` for
/// OpenUI Cloud's config block.
pub fn generate_system_prompt(spec: &Json) -> Result<String, PromptError> {
    let Some(object) = spec.as_object() else {
        return Err(PromptError(
            "[generateSystemPrompt] spec must be an object.".to_owned(),
        ));
    };
    if object.get("cloud") == Some(&Json::Bool(true)) {
        return cloud_config(spec);
    }
    // `typeof library === "object"`: null counts, and spreads to nothing.
    match object.get("library") {
        Some(library @ (Json::Object(_) | Json::Null | Json::Array(_))) => {
            let mut merged = serde_json::Map::new();
            if let Some(entries) = library.as_object() {
                for (key, value) in entries {
                    merged.insert(key.clone(), value.clone());
                }
            }
            if let Some(options) = object.get("promptOptions").and_then(Json::as_object) {
                for (key, value) in options {
                    merged.insert(key.clone(), value.clone());
                }
            }
            Ok(generate_prompt(&Json::Object(merged)))
        }
        _ => Ok(generate_prompt(spec)),
    }
}

// ─── Reading the spec ───────────────────────────────────────────────────────

fn field<'a>(value: &'a Json, key: &str) -> Option<&'a Json> {
    value.as_object()?.get(key)
}

/// `value ?? default`: present and not null.
fn present<'a>(value: &'a Json, key: &str) -> Option<&'a Json> {
    field(value, key).filter(|v| !v.is_null())
}

/// An object's entries in JavaScript property order; nothing for a non-object.
fn entries(value: Option<&Json>) -> Vec<(&String, &Json)> {
    match value.and_then(Json::as_object) {
        Some(map) => json_entries(map),
        None => Vec::new(),
    }
}

fn items(value: Option<&Json>) -> &[Json] {
    value.and_then(Json::as_array).map_or(&[], Vec::as_slice)
}

/// `required.includes(key)`: an array matches a string element, and a string
/// (a schema author's slip the reference tolerates) matches a substring.
fn requires(schema: &Json, key: &str) -> bool {
    match field(schema, "required") {
        Some(Json::Array(names)) => names.iter().any(|n| n.as_str() == Some(key)),
        Some(Json::String(names)) => names.contains(key),
        _ => false,
    }
}

// ─── JSON Schema → type string ──────────────────────────────────────────────

fn json_schema_type_str(schema: &Json) -> String {
    let ty = field(schema, "type").and_then(Json::as_str);
    match ty {
        Some("string") => match field(schema, "enum") {
            Some(Json::Array(values)) => values
                .iter()
                .map(|v| format!("\"{}\"", js_string(v)))
                .collect::<Vec<_>>()
                .join(" | "),
            _ => "string".to_owned(),
        },
        Some("number" | "integer") => "number".to_owned(),
        Some("boolean") => "boolean".to_owned(),
        Some("array") => match field(schema, "items") {
            Some(items) if truthy(Some(items)) => format!("{}[]", json_schema_type_str(items)),
            _ => "any[]".to_owned(),
        },
        Some("object") => {
            let props = entries(field(schema, "properties"));
            if props.is_empty() {
                return "object".to_owned();
            }
            let fields: Vec<String> = props
                .into_iter()
                .map(|(k, v)| {
                    let opt = if requires(schema, k) { "" } else { "?" };
                    format!("{k}{opt}: {}", json_schema_type_str(v))
                })
                .collect();
            format!("{{{}}}", fields.join(", "))
        }
        _ => "any".to_owned(),
    }
}

/// A default-values hint for an output schema.
fn default_for_schema(schema: &Json) -> Json {
    match field(schema, "type").and_then(Json::as_str) {
        Some("string") => Json::String(String::new()),
        Some("number" | "integer") => Json::from(0),
        Some("boolean") => Json::Bool(false),
        Some("array") => Json::Array(Vec::new()),
        Some("object") => {
            let props = entries(field(schema, "properties"));
            let mut result = serde_json::Map::new();
            for (k, v) in props {
                result.insert(k.clone(), default_for_schema(v));
            }
            Json::Object(result)
        }
        _ => Json::Null,
    }
}

// ─── Sections ───────────────────────────────────────────────────────────────

const PREAMBLE: &str = "You are an AI assistant that responds using openui-lang, a declarative UI language. Your ENTIRE response must be valid openui-lang code — no markdown, no explanations, just openui-lang.";

fn syntax_rules(root: &str, supports_expressions: bool, bindings: bool) -> String {
    let mut lines = vec![
        "## Syntax Rules".to_owned(),
        String::new(),
        "1. Each statement is on its own line: `identifier = Expression`".to_owned(),
        format!("2. `root` is the entry point — every program must define `root = {root}(...)`"),
        "3. Expressions are: strings (\"...\"), numbers, booleans (true/false), null, arrays ([...]), objects ({...}), or component calls TypeName(arg1, arg2, ...)".to_owned(),
        "4. Use references for readability: define `name = ...` on one line, then use `name` later".to_owned(),
        "5. EVERY variable (except root) MUST be referenced by at least one other variable. Unreferenced variables are silently dropped and will NOT render. Always include defined variables in their parent's children/items array.".to_owned(),
        "6. Arguments are POSITIONAL (order matters, not names). Write `SomeComp([children], \"row\", \"l\")` NOT `SomeComp([children], direction: \"row\", gap: \"l\")` — colon syntax is NOT supported and silently breaks".to_owned(),
        "7. Optional arguments can be omitted from the end".to_owned(),
    ];
    let mut rule = 8;
    if bindings {
        lines.push(format!(
            "{rule}. Declare mutable state with `$varName = defaultValue`. Components marked with `$binding` can read/write these. Undeclared $variables are auto-created with null default."
        ));
        rule += 1;
    }
    if supports_expressions {
        for text in [
            "String concatenation: `\"text\" + $var + \"more\"`",
            "Dot member access: `query.field` reads a field; on arrays it extracts that field from every element",
            "Index access: `arr[0]`, `data[index]`",
            "Arithmetic operators: +, -, *, /, % (work on numbers; + is string concat when either side is a string)",
            "Comparison: ==, !=, >, <, >=, <=",
            "Logical: &&, ||, ! (prefix)",
            "Ternary: `condition ? valueIfTrue : valueIfFalse`",
            "Parentheses for grouping: `(a + b) * c`",
        ] {
            lines.push(format!("{rule}. {text}"));
            rule += 1;
        }
    }
    lines.push("- Strings use double quotes with backslash escaping".to_owned());
    lines.join("\n")
}

/// The builtin registry's prompt docs, in registry order, then the lazy `Each`.
const BUILTIN_DOCS: [(&str, &str); 14] = [
    ("Count(array) → number", "Returns array length"),
    ("First(array) → element", "Returns first element of array"),
    ("Last(array) → element", "Returns last element of array"),
    ("Sum(numbers[]) → number", "Sum of numeric array"),
    ("Avg(numbers[]) → number", "Average of numeric array"),
    ("Min(numbers[]) → number", "Minimum value in array"),
    ("Max(numbers[]) → number", "Maximum value in array"),
    (
        "Sort(array, field, direction?) → sorted array",
        "Sort array by field. Direction: \"asc\" (default) or \"desc\"",
    ),
    (
        "Filter(array, field, operator: \"==\" | \"!=\" | \">\" | \"<\" | \">=\" | \"<=\" | \"contains\", value) → filtered array",
        "Filter array by field value",
    ),
    (
        "Round(number, decimals?) → number",
        "Round to N decimal places (default 0)",
    ),
    ("Abs(number) → number", "Absolute value"),
    ("Floor(number) → number", "Round down to nearest integer"),
    ("Ceil(number) → number", "Round up to nearest integer"),
    (
        "Each(array, varName, template)",
        "Evaluate template for each element. varName is the loop variable — use it ONLY inside the template expression (inline). Do NOT create a separate statement for the template.",
    ),
];

fn builtin_functions_section() -> String {
    let lines: Vec<String> = BUILTIN_DOCS
        .iter()
        .map(|(signature, description)| format!("@{signature} — {description}"))
        .collect();
    format!(
        "{}\n{}\n{}",
        r##"## Built-in Functions

Data functions prefixed with `@` to distinguish from components. These are the ONLY functions available — do NOT invent new ones.
Use @-prefixed built-in functions (@Count, @Sum, @Avg, @Min, @Max, @Round) on Query results — do NOT hardcode computed values.
"##,
        lines.join("\n"),
        r##"
Builtins compose — output of one is input to the next:
`@Count(@Filter(data.rows, "field", "==", "val"))` for KPIs/chart values, `@Round(@Avg(data.rows.score), 1)`, `@Each(data.rows, "item", Comp(item.field))` for per-item rendering.
Array pluck: `data.rows.field` extracts a field from every row → use with @Sum, @Avg, charts, tables.

IMPORTANT @Each rule: The loop variable (e.g. "item") is ONLY available inside the @Each template expression. Always inline the template — do NOT extract it to a separate statement.
CORRECT: `Col("Actions", @Each(rows, "t", Button("Edit", Action([@Set($id, t.id)]))))`
WRONG: `myBtn = Button("Edit", Action([@Set($id, t.id)]))` then `Col("Actions", @Each(rows, "t", myBtn))` — t is undefined in myBtn."##
    )
}

const QUERY_SECTION: &str = r##"## Query — Live Data Fetching

Fetch data from available tools. Returns defaults instantly, swaps in real data when it arrives.

```
metrics = Query("tool_name", {arg1: value, arg2: $binding}, {defaultField: 0, defaultData: []}, refreshInterval?)
```

- First arg: tool name (string)
- Second arg: arguments object (may reference $bindings — re-fetches automatically on change)
- Third arg: default data (rendered immediately before fetch resolves)
- Fourth arg (optional): refresh interval in seconds (e.g. 30 for auto-refresh every 30s)
- Use dot access on results: metrics.totalEvents, metrics.data.day (array pluck)
- Query results must use regular identifiers: `metrics = Query(...)`, NOT `$metrics = Query(...)`
- Manual refresh: `Button("Refresh", Action([@Run(query1), @Run(query2)]), "secondary")` — re-fetches the listed queries
- Refresh all queries: create Action with @Run for each query"##;

const MUTATION_SECTION: &str = r##"## Mutation — Write Operations

Execute state-changing tool calls (create, update, delete). Unlike Query (auto-fetches on render), Mutation fires only on button click via Action.

```
result = Mutation("tool_name", {arg1: $binding, arg2: "value"})
```

- First arg: tool name (string)
- Second arg: arguments object (evaluated with current $binding values at click time)
- result.status: "idle" | "loading" | "success" | "error"
- result.data: tool response on success
- result.error: error message on failure
- Mutation results use regular identifiers: `result = Mutation(...)`, NOT `$result`
- Show loading state: `result.status == "loading" ? TextContent("Saving...") : null`"##;

fn action_section(tool_calls: bool, bindings: bool) -> String {
    let mut steps = vec![
        "- @ToAssistant(\"message\") — Send a message to the assistant (for conversational buttons like \"Tell me more\", \"Explain this\")",
        "- @OpenUrl(\"https://...\") — Navigate to a URL",
    ];
    if bindings {
        steps.push("- @Set($variable, value) — Set a $variable to a specific value");
        steps.push("- @Reset($var1, $var2, ...) — Reset $variables to their declared defaults (e.g. @Reset($title, $priority) restores $title=\"\" and $priority=\"medium\")");
    }
    if tool_calls {
        steps.insert(0, "- @Run(queryOrMutationRef) — Execute a Mutation or re-fetch a Query (ref must be a declared Query/Mutation)");
    }

    let mut examples = Vec::new();
    if tool_calls {
        examples.push(
            r##"Example — mutation + refresh + reset (PREFERRED pattern):
```
$binding = "default"
result = Mutation("tool_name", {field: $binding})
data = Query("tool_name", {}, {rows: []})
onSubmit = Action([@Run(result), @Run(data), @Reset($binding)])
```"##,
        );
    }
    examples.push(
        r##"Example — simple nav:
```
viewBtn = Button("View", Action([@OpenUrl("https://example.com")]))
```"##,
    );

    let mut rules = vec![
        "- Action can be assigned to a variable or inlined: Button(\"Go\", onSubmit) and Button(\"Go\", Action([...])) both work",
    ];
    if tool_calls {
        rules.push(
            "- If a @Run(mutation) step fails, remaining steps are skipped (halt on failure)",
        );
        rules.push("- @Run(queryRef) re-fetches the query (fire-and-forget, cannot fail)");
    }

    format!(
        "{}\n{}\n\n{}\n\n{}",
        r##"## Action — Button Behavior

Action([@steps...]) wires button clicks to operations. Steps are @-prefixed built-in actions. Steps execute in order.
Buttons without an explicit Action prop automatically send their label to the assistant (equivalent to Action([@ToAssistant(label)])).

Available steps:"##,
        steps.join("\n"),
        examples.join("\n\n"),
        rules.join("\n")
    )
}

const INTERACTIVE_FILTERS_SECTION: &str = r##"## Interactive Filters

To let the user filter data with a dropdown:
1. Declare a $variable with a default: `$dateRange = "14"`
2. Create a Select with name, items, and binding: `Select("dateRange", [SelectItem("7", "Last 7 days"), ...], null, null, $dateRange)`
3. Wrap in FormControl for a label: `FormControl("Date Range", Select(...))`
4. Pass $dateRange in Query args: `Query("tool", {dateRange: $dateRange}, {defaults})`
5. When the user changes the Select, $dateRange updates and the Query automatically re-fetches

FILTER WIRING RULE: If a $binding filter is visible in the UI, EVERY relevant Query MUST reference that $binding in its args. Never show a filter dropdown while hardcoding the query args.

Rules for $variables:
- $variables hold simple values (strings or numbers), NOT arrays or objects
- $variables must be bound to a Select/Input component via the value argument (last positional arg) to be interactive
- Queries must use regular identifiers (NOT $variables): `metrics = Query(...)` not `$metrics = Query(...)`
- **Auto-declare**: You do NOT need to explicitly declare $variables. If you use `$foo` without declaring it, the parser auto-creates `$foo = null`. You can still declare explicitly to set a default: `$days = "14"`

## Forms

Simple form — no $bindings needed. Field values are managed internally by the Form via the name prop:
```
contactForm = Form("contact", submitBtn, [nameField, emailField])
nameField = FormControl("Name", Input("name", "Your name", "text", {required: true}))
emailField = FormControl("Email", Input("email", "your@email.com", "email", {required: true, email: true}))
submitBtn = Button("Submit")
```

Use $bindings when you need to read field values elsewhere (in Action context, Query args, or conditionals). They are auto-declared:
```
$role = "engineer"
contactForm = Form("contact", submitBtn, [nameField, emailField, roleField])
nameField = FormControl("Name", Input("name", "Enter your name", "text", {required: true}, $name))
emailField = FormControl("Email", Input("email", "Enter your email", "email", {required: true, email: true}, $email))
roleField = FormControl("Role", Select("role", [SelectItem("engineer", "Engineer"), SelectItem("designer", "Designer"), SelectItem("pm", "PM")], null, {required: true}, $role))
submitBtn = Button("Submit")
```

For form + mutation patterns (create, refresh, reset), see the Action section example above.

IMPORTANT: Always add validation rules to form fields used with Mutations. Use OBJECT syntax: {required: true, email: true, minLength: 8}. The renderer shows error messages automatically and blocks submit when validation fails."##;

const EDIT_MODE_SECTION: &str = r##"## Edit Mode

The runtime merges by statement name: same name = replace, new name = append.
Output ONLY statements that changed or are new. Everything else is kept automatically.

### Delete
To remove a component, re-declare its parent without that component in the parent's children array. The removed component and any statements only it referenced are automatically garbage-collected.

### Patch size guide
- Changing a title or label: 1 statement
- Adding a component: 2-3 statements (the new component + parent update)
- Removing a component: 1 statement (re-declare parent without the removed child)
- Adding a filter + wiring to query: 3-5 statements
- Restructuring into tabs: 5-10 statements

### Rules
- Reuse existing statement names exactly — do not rename
- Do NOT re-emit unchanged statements — the runtime keeps them
- A typical edit patch is 1-10 statements, not 20+
- If the existing code already satisfies the request, output only the root statement
- NEVER output the entire program as a patch. Only output what actually changes
- If you are about to output more than 10 statements, reconsider — most edits need fewer"##;

fn streaming_rules(root: &str, supports_expressions: bool) -> String {
    let mut steps = vec![format!(
        "1. `root = {root}(...)` — UI shell appears immediately"
    )];
    if supports_expressions {
        steps.push("2. $variable declarations — state ready for bindings".to_owned());
        steps.push(
            "3. Query statements — defaults resolve immediately so components render with data"
                .to_owned(),
        );
        steps.push("4. Component definitions — fill in with data already available".to_owned());
        steps.push("5. Data values — leaf content last".to_owned());
    } else {
        steps.push("2. Component definitions — fill in as they stream".to_owned());
        steps.push("3. Data values — leaf content last".to_owned());
    }
    format!(
        "{}\n{}\n\nAlways write the root = {root}(...) statement first so the UI shell appears immediately, even before child data has streamed in.",
        r##"## Hoisting & Streaming (CRITICAL)

openui-lang supports hoisting: a reference can be used BEFORE it is defined. The parser resolves all references after the full input is parsed.

During streaming, the output is re-parsed on every chunk. Undefined references are temporarily unresolved and appear once their definitions stream in. This creates a progressive top-down reveal — structure first, then data fills in.

**Recommended statement order for optimal streaming:**"##,
        steps.join("\n")
    )
}

const INLINE_MODE_SECTION: &str = r##"## Inline Mode

You are in inline mode. You can respond in two ways:

### 1. Code response (when the user wants to CREATE or CHANGE the UI)
Wrap openui-lang code in triple-backtick fences. You can include explanatory text before/after:

Here's your dashboard:

```openui-lang
root = RootComp([header, content])
header = SomeHeader("Title")
content = SomeContent("Hello world")
```

I created a simple layout with a header.

### 2. Text-only response (when the user asks a QUESTION)
If the user asks "what is this?", "explain the chart", "how does this work", etc. — respond with plain text. Do NOT output any openui-lang code. The existing dashboard stays unchanged.

### Rules
- When the user asks for changes, output ONLY the changed/new statements in a fenced block
- When the user asks a question, respond with text only — NO code. The dashboard stays unchanged.
- The parser extracts code from fences automatically. Text outside fences is shown as chat."##;

const TOOL_WORKFLOW_SECTION: &str = r##"## Data Workflow

When tools are available, follow this workflow:
1. FIRST: Call the most relevant tool to inspect the real data shape before generating code
2. Use Query() for READ operations (data that should stay live) — NEVER hardcode tool results as literal arrays or objects
3. Use Mutation() for WRITE operations (create, update, delete) — triggered by button clicks via Action([@Run(mutationRef)])
4. Use the real data from step 1 as condensed Query defaults (3-5 rows) so the UI renders immediately
5. Use @-prefixed builtins (@Count, @Filter, @Sort, @Sum) on Query results for KPIs and aggregations — the runtime evaluates these live on every refresh
6. Hardcoded arrays are ONLY for static display data (labels, options) where no tool exists

WRONG — you called a tool and got data back, but you inlined the results:
```
openCount = 2
item1 = SomeComp("first item title")
item2 = SomeComp("second item title")
list = SomeList([item1, item2])
chart = SomeChart(["A", "B"], [12, 8])
```
This is static — it shows stale data and won't update. Creating item1, item2, item3... manually is ALWAYS wrong when a tool exists.

RIGHT — use Query() for live data, Mutation() for writes, @builtins to derive values:
```
data = Query("tool_name", {}, {rows: []})
openCount = @Count(@Filter(data.rows, "field", "==", "value"))
list = @Each(data.rows, "item", SomeComp(item.title, item.field))
createResult = Mutation("create_tool", {title: $title})
submitBtn = Button("Create", Action([@Run(createResult), @Run(data), @Reset($title)]))
```
Everything derives from the Query — when data refreshes, the entire dashboard updates automatically."##;

fn important_rules(root: &str, tool_calls: bool, bindings: bool) -> String {
    let mut verify = vec![
        format!("1. root = {root}(...) is the FIRST line (for optimal streaming)."),
        "2. Every referenced name is defined. Every defined name (other than root) is reachable from root.".to_owned(),
    ];
    if tool_calls {
        verify.push("3. Every Query result is referenced by at least one component.".to_owned());
    }
    if bindings {
        let n = if tool_calls { "4" } else { "3" };
        verify.push(format!(
            "{n}. Every $binding appears in at least one component or expression."
        ));
    }
    if tool_calls && bindings {
        verify.push(
            "5. Every visible filter $binding appears in at least one Query args object."
                .to_owned(),
        );
    }
    format!(
        "{}\n{}",
        r##"## Important Rules
- Choose components that best represent the content (tables for comparisons, charts for trends, forms for input, etc.)

## Final Verification
Before finishing, walk your output and verify:"##,
        verify.join("\n")
    )
}

// ─── Tools ──────────────────────────────────────────────────────────────────

fn render_tool_signature(tool: &Json) -> String {
    let mut args = String::new();
    if let Some(input) = field(tool, "inputSchema").filter(|v| truthy(Some(v))) {
        let props = entries(field(input, "properties"));
        if !props.is_empty() {
            args = props
                .into_iter()
                .map(|(k, v)| {
                    let opt = if requires(input, k) { "" } else { "?" };
                    format!("{k}{opt}: {}", json_schema_type_str(v))
                })
                .collect::<Vec<_>>()
                .join(", ");
        }
    }
    let mut return_type = String::new();
    if let Some(output) = field(tool, "outputSchema").filter(|v| truthy(Some(v))) {
        return_type = format!(" → {}", json_schema_type_str(output));
    }
    let name = field(tool, "name").map_or_else(|| "undefined".to_owned(), js_string);
    let mut line = format!("- {name}({args}){return_type}");
    if let Some(description) = field(tool, "description").filter(|v| truthy(Some(v))) {
        line.push_str("\n  ");
        line.push_str(&js_string(description));
    }
    line
}

fn render_tools_section(tools: &[Json]) -> String {
    let mut lines: Vec<String> = vec![
        "## Available Tools".to_owned(),
        String::new(),
        "Use these with Query() for read operations or Mutation() for write operations. The LLM decides which is appropriate based on the tool's purpose.".to_owned(),
        String::new(),
    ];
    let (names, specs): (Vec<&Json>, Vec<&Json>) = tools.iter().partition(|t| t.is_string());
    for name in &names {
        lines.push(format!("- {}", js_string(name)));
    }
    for tool in &specs {
        lines.push(render_tool_signature(tool));
    }
    let with_output: Vec<&Json> = specs
        .iter()
        .copied()
        .filter(|t| truthy(field(t, "outputSchema")))
        .collect();
    if !with_output.is_empty() {
        lines.push(String::new());
        lines.push("### Default values for Query results".to_owned());
        lines.push(String::new());
        lines.push("Use these shapes as minimal Query defaults:".to_owned());
        for tool in with_output {
            let name = field(tool, "name").map_or_else(|| "undefined".to_owned(), js_string);
            let defaults = default_for_schema(field(tool, "outputSchema").expect("truthy"));
            lines.push(format!("- {name}: `{}`", json_value_string(&defaults)));
        }
    }
    lines.push(String::new());
    lines.push("CRITICAL: Use ONLY the tools listed above in Query() and Mutation() calls. Do NOT invent or guess tool names. If the user asks for functionality that doesn't match any available tool, use realistic mock data instead of fabricating a tool call.".to_owned());
    lines.join("\n")
}

// ─── Component signatures ───────────────────────────────────────────────────

fn signature_contains(component: &Json, needle: &str) -> bool {
    field(component, "signature")
        .and_then(Json::as_str)
        .is_some_and(|s| s.contains(needle))
}

fn format_signature(component: &Json) -> String {
    let signature = field(component, "signature").map_or_else(|| "undefined".to_owned(), js_string);
    match field(component, "description").filter(|v| truthy(Some(v))) {
        Some(description) => format!("{signature} — {}", js_string(description)),
        None => signature,
    }
}

fn component_signatures(
    spec: &Json,
    tool_calls: bool,
    bindings: bool,
    uses_action_expression: bool,
) -> String {
    let components = entries(field(spec, "components"));
    let mut lines = vec![
        "## Component Signatures".to_owned(),
        String::new(),
        "Arguments marked with ? are optional. Sub-components can be inline or referenced; prefer references for better streaming.".to_owned(),
    ];
    if uses_action_expression {
        let steps: Vec<&str> = [
            tool_calls.then_some("@Run"),
            Some("@ToAssistant"),
            Some("@OpenUrl"),
            bindings.then_some("@Set"),
            bindings.then_some("@Reset"),
        ]
        .into_iter()
        .flatten()
        .collect();
        lines.push(format!(
            "Props typed `ActionExpression` accept an Action([@steps...]) expression. See the Action section for available steps ({}).",
            steps.join(", ")
        ));
    }
    let uses_bindings = bindings
        || components
            .iter()
            .any(|(_, c)| signature_contains(c, "$binding"));
    if uses_bindings {
        lines.push(
            "Props marked `$binding<type>` accept a `$variable` reference for two-way binding."
                .to_owned(),
        );
    }

    let groups = items(field(spec, "componentGroups"));
    if groups.is_empty() {
        lines.push(String::new());
        for (_, component) in &components {
            lines.push(format_signature(component));
        }
        return lines.join("\n");
    }

    // Names already listed, as the group wrote them (a group may name a
    // component by something that isn't a string; that never matches a key).
    let mut grouped: Vec<&Json> = Vec::new();
    for group in groups {
        lines.push(String::new());
        lines.push(format!(
            "### {}",
            field(group, "name").map_or_else(|| "undefined".to_owned(), js_string)
        ));
        for name in items(field(group, "components")) {
            if grouped.contains(&name) {
                continue;
            }
            let key = js_string(name);
            let Some(component) = components
                .iter()
                .find(|(k, _)| **k == key)
                .map(|(_, c)| *c)
                .filter(|c| truthy(Some(c)))
            else {
                continue;
            };
            grouped.push(name);
            lines.push(format_signature(component));
        }
        for note in items(field(group, "notes")) {
            lines.push(js_string(note));
        }
    }
    let ungrouped: Vec<&Json> = components
        .iter()
        .filter(|(k, _)| !grouped.contains(&&Json::String((*k).clone())))
        .map(|(_, c)| *c)
        .collect();
    if !ungrouped.is_empty() {
        lines.push(String::new());
        lines.push("### Other".to_owned());
        for component in ungrouped {
            lines.push(format_signature(component));
        }
    }
    lines.join("\n")
}

// ─── Assembly ───────────────────────────────────────────────────────────────

/// The reference's `generatePrompt`: a prompt spec (components, groups, tools,
/// examples, rules and mode flags) to the system prompt text.
pub fn generate_prompt(spec: &Json) -> String {
    let root = present(spec, "root").map_or_else(|| "Root".to_owned(), js_string);
    let tools = items(field(spec, "tools"));
    let has_tools = !tools.is_empty();
    let tool_calls = present(spec, "toolCalls").map_or(has_tools, |v| truthy(Some(v)));
    let bindings = present(spec, "bindings").map_or(tool_calls, |v| truthy(Some(v)));
    let supports_expressions = tool_calls || bindings;
    let uses_action_expression = entries(field(spec, "components"))
        .iter()
        .any(|(_, c)| signature_contains(c, "ActionExpression"));

    let mut parts: Vec<String> = vec![
        present(spec, "preamble").map_or_else(|| PREAMBLE.to_owned(), js_string),
        String::new(),
        syntax_rules(&root, supports_expressions, bindings),
        String::new(),
        component_signatures(spec, tool_calls, bindings, uses_action_expression),
    ];
    if supports_expressions {
        parts.push(String::new());
        parts.push(builtin_functions_section());
    }
    if tool_calls {
        parts.push(String::new());
        parts.push(QUERY_SECTION.to_owned());
        parts.push(String::new());
        parts.push(MUTATION_SECTION.to_owned());
    }
    if uses_action_expression {
        parts.push(String::new());
        parts.push(action_section(tool_calls, bindings));
    }
    if tool_calls && bindings {
        parts.push(String::new());
        parts.push(INTERACTIVE_FILTERS_SECTION.to_owned());
    }
    if tool_calls {
        parts.push(String::new());
        parts.push(TOOL_WORKFLOW_SECTION.to_owned());
    }
    if has_tools {
        parts.push(String::new());
        parts.push(render_tools_section(tools));
    }
    parts.push(String::new());
    parts.push(streaming_rules(&root, supports_expressions));

    let examples: Vec<&Json> = items(field(spec, "examples"))
        .iter()
        .chain(items(field(spec, "toolExamples")))
        .collect();
    if !examples.is_empty() {
        parts.push(String::new());
        parts.push("## Examples".to_owned());
        parts.push(String::new());
        for example in examples {
            parts.push(js_string(example));
            parts.push(String::new());
        }
    }
    if truthy(field(spec, "editMode")) {
        parts.push(String::new());
        parts.push(EDIT_MODE_SECTION.to_owned());
    }
    if truthy(field(spec, "inlineMode")) {
        parts.push(String::new());
        parts.push(INLINE_MODE_SECTION.to_owned());
    }
    parts.push(important_rules(&root, tool_calls, bindings));
    let rules = items(field(spec, "additionalRules"));
    if !rules.is_empty() {
        parts.push(String::new());
        for rule in rules {
            parts.push(format!("- {}", js_string(rule)));
        }
    }
    parts.join("\n")
}

// ─── Cloud config ───────────────────────────────────────────────────────────

const CLOUD_CONFIG_MARKER: &str = "]]>openui:config\n";
const CLOUD_CHAT_LIBRARY_VERSION: &str = "0.1.0";

/// The prompt options OpenUI Cloud accepts, in its key order; `None` if there
/// are none worth sending.
fn cloud_prompt_options(options: Option<&Json>) -> Option<Json> {
    let options = options.filter(|o| truthy(Some(o)))?;
    let mut picked = serde_json::Map::new();
    for key in ["examples", "preamble", "additionalRules"] {
        if let Some(value) = field(options, key).filter(|v| truthy(Some(v))) {
            picked.insert(key.to_owned(), value.clone());
        }
    }
    (!picked.is_empty()).then_some(Json::Object(picked))
}

fn cloud_config(spec: &Json) -> Result<String, PromptError> {
    let mut config = serde_json::Map::new();
    match field(spec, "library").filter(|l| truthy(Some(l))) {
        Some(library) => {
            let issues = validate_chat_library(library);
            if !issues.is_empty() {
                return Err(PromptError(format!(
                    "[generateSystemPrompt] Invalid library: {}",
                    issues.join(" ")
                )));
            }
            let mut chat_library = serde_json::Map::new();
            if let Some(entries) = library.as_object() {
                for (key, value) in entries {
                    if key != "components" {
                        chat_library.insert(key.clone(), value.clone());
                    }
                }
            }
            config.insert("chatLibrary".to_owned(), Json::Object(chat_library));
            if let Some(options) = cloud_prompt_options(field(spec, "promptOptions")) {
                config.insert("systemPromptOptions".to_owned(), options);
            }
        }
        None => {
            if cloud_prompt_options(field(spec, "promptOptions")).is_some() {
                return Err(PromptError(
                    "[generateSystemPrompt] promptOptions requires a library — the built-in library ignores it.".to_owned(),
                ));
            }
            config.insert(
                "libraryVersion".to_owned(),
                Json::String(CLOUD_CHAT_LIBRARY_VERSION.to_owned()),
            );
        }
    }
    let mut block = format!(
        "{CLOUD_CONFIG_MARKER}{}",
        json_value_string(&Json::Object(config))
    );
    if let Some(instructions) = field(spec, "instructions").filter(|v| truthy(Some(v))) {
        block.push('\n');
        block.push_str(&js_string(instructions));
    }
    Ok(block)
}

// ─── Library validation (cloud mode) ────────────────────────────────────────

fn is_plain_object(value: Option<&Json>) -> bool {
    matches!(value, Some(Json::Object(_)))
}

/// Every `$ref` under `node` that doesn't point at a component in `$defs`.
fn collect_ref_issues(node: &Json, path: &str, defs: &[&String], issues: &mut Vec<String>) {
    match node {
        Json::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                collect_ref_issues(item, &format!("{path}/{i}"), defs, issues);
            }
        }
        Json::Object(map) => {
            if let Some(Json::String(r)) = map.get("$ref") {
                // The reference's `^#\/\$defs\/(.+)$`: `.` stops at line terminators.
                let resolves = r
                    .strip_prefix("#/$defs/")
                    .filter(|name| {
                        !name.is_empty() && !name.contains(['\n', '\r', '\u{2028}', '\u{2029}'])
                    })
                    .is_some_and(|name| defs.iter().any(|d| *d == name));
                if !resolves {
                    issues.push(format!(
                        "Unresolvable $ref \"{r}\" at {path} — refs must be \"#/$defs/<name>\" pointing at a component in $defs."
                    ));
                }
            }
            for (key, value) in json_entries(map) {
                if key != "$ref" {
                    collect_ref_issues(value, &format!("{path}/{key}"), defs, issues);
                }
            }
        }
        _ => {}
    }
}

/// The reference's `validateChatLibrary`: every issue's message, in order.
fn validate_chat_library(library: &Json) -> Vec<String> {
    let mut issues = Vec::new();
    let Some(map) = library.as_object() else {
        return vec!["chatLibrary must be an object.".to_owned()];
    };
    let root = map.get("root");
    if let Some(root) = root
        && !matches!(root, Json::String(s) if !s.is_empty())
    {
        issues.push("chatLibrary.root, when present, must be a non-empty string.".to_owned());
    }
    let schema = map.get("schema");
    if !is_plain_object(schema) {
        issues.push(
            "chatLibrary.schema must be an object with a $defs map of component schemas."
                .to_owned(),
        );
        return issues;
    }
    let defs = field(schema.expect("object"), "$defs");
    let defs = match defs.and_then(Json::as_object) {
        Some(defs) if !defs.is_empty() => defs,
        _ => {
            issues.push(
                "chatLibrary.schema.$defs must be a non-empty object keyed by component name."
                    .to_owned(),
            );
            return issues;
        }
    };
    let def_entries = json_entries(defs);
    let def_names: Vec<&String> = def_entries.iter().map(|(k, _)| *k).collect();

    if let Some(Json::String(root)) = root
        && !root.is_empty()
        && !def_names.contains(&root)
    {
        issues.push(format!(
            "Root component \"{root}\" was not found in schema.$defs. Available components: {}.",
            def_names
                .iter()
                .map(|n| n.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    for (name, def) in &def_entries {
        let def_path = format!("$defs/{name}");
        let Some(def) = def.as_object() else {
            issues.push(format!("{def_path} must be an object component schema."));
            continue;
        };
        let properties = def.get("properties");
        if properties.is_some() && !is_plain_object(properties) {
            issues.push(format!("{def_path}/properties must be an object."));
        }
        if let Some(required) = def.get("required") {
            match required.as_array() {
                Some(names) if names.iter().all(Json::is_string) => {
                    let keys: Vec<&String> = properties
                        .and_then(Json::as_object)
                        .map(|p| p.keys().collect())
                        .unwrap_or_default();
                    for r in names {
                        let r = r.as_str().expect("string");
                        if !keys.iter().any(|k| *k == r) {
                            issues.push(format!(
                                "{def_path} lists required property \"{r}\" that is not in properties."
                            ));
                        }
                    }
                }
                _ => issues.push(format!(
                    "{def_path}/required must be an array of property names."
                )),
            }
        }
        if let Some(properties) = properties {
            collect_ref_issues(
                properties,
                &format!("{def_path}/properties"),
                &def_names,
                &mut issues,
            );
        }
    }

    for group in items(map.get("componentGroups")) {
        let name = field(group, "name").and_then(Json::as_str);
        let components = field(group, "components").and_then(Json::as_array);
        let (Some(name), Some(components)) = (name, components) else {
            issues.push("Each componentGroups entry must be {name: string, components: string[], notes?: string[]}.".to_owned());
            continue;
        };
        for component in components {
            let known = component
                .as_str()
                .is_some_and(|c| def_names.iter().any(|d| *d == c));
            if !known {
                issues.push(format!(
                    "Component group \"{name}\" references unknown component \"{}\".",
                    js_string(component)
                ));
            }
        }
    }
    issues
}
