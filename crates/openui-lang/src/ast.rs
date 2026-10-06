//! Expression trees, mirroring the reference parser's `ASTNode`.

/// One expression node.
#[derive(Clone, Debug, PartialEq)]
pub enum Ast {
    /// A component or builtin call: `Header("Hello")`.
    Comp {
        name: String,
        args: Vec<Ast>,
    },
    Str(String),
    Null,
    /// A reference to another statement: `header`.
    Ref(String),
    /// A placeholder for a reference that couldn't be resolved.
    Ph(String),
}
