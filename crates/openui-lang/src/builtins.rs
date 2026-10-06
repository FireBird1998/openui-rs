//! Names the reference parser treats as builtins rather than components.

/// Builtin functions, the lazy `Each`, and action steps (`BUILTIN_NAMES`).
const BUILTIN_NAMES: [&str; 20] = [
    "Count",
    "First",
    "Last",
    "Sum",
    "Avg",
    "Min",
    "Max",
    "Sort",
    "Filter",
    "Round",
    "Abs",
    "Floor",
    "Ceil",
    "Each",
    "Action",
    "Run",
    "ToAssistant",
    "OpenUrl",
    "Set",
    "Reset",
];

/// Whether `name` is a builtin function or action, not a component.
pub fn is_builtin(name: &str) -> bool {
    BUILTIN_NAMES.contains(&name)
}

/// Statement-level calls that are neither builtins nor components.
pub fn is_reserved_call(name: &str) -> bool {
    matches!(name, "Query" | "Mutation")
}
