//! Random programs: generated from a component library's grammar, or real
//! corpus programs mangled the ways model output gets mangled.

use openui_lang::Library;

use crate::rng::Rng;

const STATEMENT_IDS: [&str; 12] = [
    "a", "b", "c", "items", "data", "header", "Main", "x1", "$count", "$flag", "$q", "save",
];

const STRING_PIECES: [&str; 23] = [
    "hi",
    "a b",
    "it's",
    "\\n",
    "\\\"",
    "\\\\",
    "\\u00e9",
    "\\uD83D\\uDE00",
    "\\uD83D",
    "\\uDE00x",
    "😀",
    "é",
    "// not",
    "# not",
    "```",
    "'",
    "$x",
    "@y",
    "\\t",
    "x\\q",
    "\\u12",
    " ",
    // 1.1: a fence line inside a multi-line string.
    "\n```\n",
];

const NUMBERS: [&str; 17] = [
    "0",
    "1",
    "-1",
    "3.14",
    "1e3",
    "1e-7",
    "1e21",
    "-0",
    "007",
    ".5",
    "1.",
    "1e",
    "9007199254740993",
    "1e400",
    "0.1",
    "2.5e-3",
    "123456789012345678901234567890",
];

const OBJECT_KEYS: [&str; 14] = [
    "a",
    "\"b c\"",
    "3",
    "$k",
    "Type",
    "0",
    "10",
    "constructor",
    "__proto__",
    "toString",
    "k",
    "type",
    "props",
    "id",
];

const AST_KIND_STRINGS: [&str; 5] = ["\"Str\"", "\"Comp\"", "\"Ref\"", "\"element\"", "\"Null\""];

const BUILTIN_CALLS: [&str; 23] = [
    "@Count(items)",
    "@Each(items, \"item\", Text(item.name))",
    "@Each(items, item, Text(item))",
    "Action([@Set($count, $count + 1), @Run(save), @ToAssistant(\"hi\")])",
    "Action([@OpenUrl(\"https://x.dev\"), @Reset($q)])",
    "@Filter(items, \"a\", \"==\", 1)",
    "@Sum(data.rows)",
    "@toString(1)",
    "Count(items)",
    "@First($items)",
    // 1.0: a step on its own, a step's context, bad and missing args, an
    // `@` name nothing registers, and Action without its list.
    "@ToAssistant(\"hi\", {orderId: 42})",
    "@OpenUrl(5)",
    "@OpenUrl()",
    "@Round(1, 2, 3)",
    "@Nope(items)",
    "Action(@OpenUrl(\"https://x.dev\"))",
    "Action([])",
    "@Set($q)",
    // 1.1: rounding over lists, and calls state defaults evaluate.
    "@Round([1.25, [2.5, null]], 1)",
    "@Abs(data.rows)",
    "@Sort(items, \"name\", \"desc\")",
    "@Each([1, 2], \"n\", n * 2)",
    "@Floor(\"2.7\")",
];

/// Characters and fragments the lexer and preprocessing treat specially.
const NOISE: [&str; 30] = [
    "\"", "'", "\\", "(", ")", "[", "]", "{", "}", ",", ":", "=", "==", "!", "?", "&", "|", "-",
    ".", "$", "@", "#", "//", "```", "\n", "\t", "\r\n", "😀", "\\u", "e",
];

pub struct Generator<'a> {
    rng: &'a mut Rng,
    components: Vec<(String, usize)>,
    /// The library's own functions and actions, with their param counts.
    calls: Vec<(String, usize)>,
    ids: Vec<&'static str>,
}

impl<'a> Generator<'a> {
    pub fn new(rng: &'a mut Rng, library: &Library) -> Generator<'a> {
        let components = library
            .component_names()
            .map(|n| {
                (
                    n.to_owned(),
                    library.component(n).map_or(0, |c| c.params.len()),
                )
            })
            .collect();
        let calls = library
            .calls()
            .filter(|(_, call)| !call.builtin)
            .map(|(n, call)| (n.to_owned(), call.params.len()))
            .collect();
        Generator {
            rng,
            components,
            calls,
            ids: Vec::new(),
        }
    }

    /// A whole program: `root` (usually) plus a few more statements, maybe
    /// wrapped in prose, fences or comments.
    pub fn program(&mut self) -> String {
        let mut ids: Vec<&'static str> = Vec::new();
        if self.rng.chance(90) {
            ids.push("root");
        }
        for _ in 0..self.rng.below(6) {
            let id = *self.rng.pick(&STATEMENT_IDS);
            if !ids.contains(&id) || self.rng.chance(15) {
                ids.push(id);
            }
        }
        if ids.is_empty() {
            ids.push("root");
        }
        self.ids = ids.clone();
        let mut lines = Vec::new();
        for id in &ids {
            let expr = match *id {
                "data" | "items" if self.rng.chance(50) => self.query(),
                // 1.1 evaluates these while parsing.
                id if id.starts_with('$') && self.rng.chance(40) => self.computed_default(),
                "save" if self.rng.chance(70) => format!("Mutation(\"save\", {})", self.object(1)),
                _ => self.expr(0),
            };
            let comment = if self.rng.chance(10) {
                " // note"
            } else if self.rng.chance(5) {
                " # note"
            } else {
                ""
            };
            // 1.1: whole-line comments, which the lexer and merging skip.
            if self.rng.chance(6) {
                lines.push(
                    self.rng
                        .pick(&["// a note", "# a note", "  // indented"])
                        .to_string(),
                );
            }
            lines.push(format!("{id} = {expr}{comment}"));
            if self.rng.chance(8) {
                lines.push(String::new());
            }
        }
        let mut text = lines.join("\n");
        if self.rng.chance(30) {
            text.push('\n');
        }
        self.wrap(text)
    }

    /// A state default that computes: arithmetic over numbers, text and the
    /// other states, or a built-in function over a literal list.
    fn computed_default(&mut self) -> String {
        let operand = |g: &mut Self| -> String {
            match g.rng.below(4) {
                0 => g.rng.pick(&NUMBERS).to_string(),
                1 => g.string(),
                2 => g.rng.pick(&["$count", "$flag", "$q", "$later"]).to_string(),
                _ => g
                    .rng
                    .pick(&["[1, \"2\", null]", "{n: 2}", "true", "null"])
                    .to_string(),
            }
        };
        match self.rng.below(3) {
            0 => {
                let op = *self
                    .rng
                    .pick(&["+", "-", "*", "/", "%", "==", "!=", ">", "&&", "||"]);
                format!("{} {op} {}", operand(self), operand(self))
            }
            1 => {
                let f = *self.rng.pick(&[
                    "@Count", "@Sum", "@Avg", "@Min", "@Max", "@First", "@Last", "@Round", "@Abs",
                    "@Floor", "@Ceil",
                ]);
                format!(
                    "{f}([{}, {}, {}])",
                    operand(self),
                    operand(self),
                    operand(self)
                )
            }
            _ => format!(
                "@Sort([{}, {}, {}, {}], \"\"{})",
                operand(self),
                operand(self),
                operand(self),
                operand(self),
                if self.rng.chance(50) {
                    ", \"desc\""
                } else {
                    ""
                }
            ),
        }
    }

    fn query(&mut self) -> String {
        // 1.1: a tool name read from another statement, and args reading
        // other queries' results, so queries wait for each other or loop.
        let tool = if self.rng.chance(20) {
            self.reference()
        } else {
            format!("\"tool_{}\"", self.rng.below(3))
        };
        let args = if self.rng.chance(30) {
            let read = *self
                .rng
                .pick(&["data.rows", "items.total", "data", "x1.id"]);
            format!("{{x: {read}}}")
        } else {
            self.object(1)
        };
        let mut args = vec![tool, args];
        if self.rng.chance(60) {
            args.push("{rows: []}".to_owned());
        }
        if self.rng.chance(30) {
            args.push(self.rng.pick(&NUMBERS).to_string());
        }
        format!("Query({})", args.join(", "))
    }

    fn expr(&mut self, depth: usize) -> String {
        if depth >= 3 {
            return self.leaf();
        }
        match self.rng.below(100) {
            0..=34 => self.component_call(depth),
            35..=44 => self.string(),
            45..=51 => self.rng.pick(&NUMBERS).to_string(),
            52..=55 => self.rng.pick(&["true", "false", "null"]).to_string(),
            56..=65 => {
                let n = self.rng.below(5);
                let items: Vec<String> = (0..n).map(|_| self.expr(depth + 1)).collect();
                let hole = if self.rng.chance(5) { ", ," } else { "" };
                format!("[{}{hole}]", items.join(", "))
            }
            66..=72 => self.object(depth + 1),
            73..=79 => self.reference(),
            80..=83 => self
                .rng
                .pick(&["$count", "$flag", "$q", "$items"])
                .to_string(),
            84..=88 => {
                let op = *self
                    .rng
                    .pick(&["+", "-", "*", "/", "%", "==", "!=", ">", "<=", "&&", "||"]);
                format!("{} {op} {}", self.expr(depth + 1), self.expr(depth + 1))
            }
            89..=91 => format!(
                "{} ? {} : {}",
                self.expr(depth + 1),
                self.expr(depth + 1),
                self.expr(depth + 1)
            ),
            92..=95 => {
                let base = self.expr(depth + 1);
                if self.rng.chance(50) {
                    format!(
                        "{base}.{}",
                        self.rng.pick(&["name", "rows", "$x", "1", "Type"])
                    )
                } else {
                    format!("{base}[{}]", self.expr(depth + 1))
                }
            }
            96..=97 => format!("{}{}", self.rng.pick(&["!", "-"]), self.expr(depth + 1)),
            _ => self.rng.pick(&BUILTIN_CALLS).to_string(),
        }
    }

    fn leaf(&mut self) -> String {
        match self.rng.below(4) {
            0 => self.string(),
            1 => self.rng.pick(&NUMBERS).to_string(),
            2 => self.reference(),
            _ => "null".to_owned(),
        }
    }

    fn component_call(&mut self, depth: usize) -> String {
        let (name, params) = if self.components.is_empty() || self.rng.chance(8) {
            (
                self.rng
                    .pick(&["Ghost", "Query", "Mutation", "Action"])
                    .to_string(),
                2,
            )
        } else {
            let (n, p) = self.rng.pick(&self.components).clone();
            (n, p)
        };
        if self.rng.chance(10) {
            return self.rng.pick(&BUILTIN_CALLS).to_string();
        }
        if !self.calls.is_empty() && self.rng.chance(20) {
            return self.library_call(depth);
        }
        let n = self.rng.below(params + 2).min(5);
        let args: Vec<String> = (0..n).map(|_| self.expr(depth + 1)).collect();
        format!("{name}({})", args.join(", "))
    }

    /// A call to one of the library's functions or actions: usually with `@`,
    /// sometimes bare, sometimes as one of several steps; with too few, the
    /// right number or too many args.
    fn library_call(&mut self, depth: usize) -> String {
        let (name, params) = self.rng.pick(&self.calls).clone();
        let n = self.rng.below(params + 2).min(5);
        let args: Vec<String> = (0..n).map(|_| self.expr(depth + 1)).collect();
        let at = if self.rng.chance(85) { "@" } else { "" };
        let call = format!("{at}{name}({})", args.join(", "));
        if self.rng.chance(15) {
            format!("Action([{call}, @ToAssistant(\"done\")])")
        } else {
            call
        }
    }

    fn reference(&mut self) -> String {
        if !self.ids.is_empty() && self.rng.chance(80) {
            let id = *self.rng.pick(&self.ids);
            return id.to_owned();
        }
        self.rng.pick(&["missing", "items", "Other"]).to_string()
    }

    fn string(&mut self) -> String {
        let n = self.rng.below(4);
        let body: String = (0..n).map(|_| *self.rng.pick(&STRING_PIECES)).collect();
        if self.rng.chance(15) {
            format!("'{}'", body.replace('\'', "\\'"))
        } else {
            format!("\"{body}\"")
        }
    }

    fn object(&mut self, depth: usize) -> String {
        let n = self.rng.below(4);
        let entries: Vec<String> = (0..n)
            .map(|_| {
                let key = *self.rng.pick(&OBJECT_KEYS);
                let value = if key == "k" || key == "type" {
                    self.rng.pick(&AST_KIND_STRINGS).to_string()
                } else if key == "__proto__" {
                    // Only plain values: an object here becomes the prototype,
                    // a documented limitation (see main.rs).
                    self.rng.pick(&NUMBERS).to_string()
                } else {
                    self.expr(depth + 1)
                };
                format!("{key}: {value}")
            })
            .collect();
        format!("{{{}}}", entries.join(", "))
    }

    fn wrap(&mut self, text: String) -> String {
        let (first, rest) = text.split_once('\n').unwrap_or((&text, ""));
        match self.rng.below(100) {
            0..=9 => format!("```openui\n{text}\n```"),
            10..=14 => format!("Here's the UI:\n```openui\n{text}\n```\nHope that helps!"),
            15..=19 => format!("Here is the UI:\n{text}"),
            20..=22 => format!("Here's the UI:\n{text}"),
            23..=26 => format!("{text}\n\nLet me know if you'd like changes."),
            // 1.1's fence rules: a script block, code on the opener line, an
            // inline mention, a stray closing fence, helpers after the program.
            27..=28 => format!("```openui-js\nconst x = 1\n```\n```openui\n{text}\n```"),
            29..=30 => format!("```openui {first}\n{rest}\n```"),
            31..=32 => format!("Use ```openui``` blocks:\n```openui\n{text}\n```"),
            33..=34 => format!("{text}\n```\nDone."),
            35..=36 => format!("{text}\n```js\nhelper()\n```"),
            _ => text,
        }
    }
}

/// Mangle a program 1–3 ways, as truncated or garbled model output would be.
pub fn mutate(rng: &mut Rng, text: &str) -> String {
    let mut chars: Vec<char> = text.chars().collect();
    for _ in 0..1 + rng.below(3) {
        let n = chars.len();
        match rng.below(8) {
            0 if n > 0 => chars.truncate(rng.below(n)),
            1 if n > 0 => {
                let start = rng.below(n);
                let end = (start + 1 + rng.below(10)).min(n);
                chars.drain(start..end);
            }
            2 => {
                let at = rng.below(n + 1);
                let noise: Vec<char> = rng.pick(&NOISE).chars().collect();
                chars.splice(at..at, noise);
            }
            3 | 4 => {
                let text: String = chars.iter().collect();
                let mut lines: Vec<&str> = text.split('\n').collect();
                if lines.len() > 1 {
                    let (i, j) = (rng.below(lines.len()), rng.below(lines.len()));
                    if rng.chance(50) {
                        lines.swap(i, j);
                    } else {
                        let line = lines[i];
                        lines.insert(j, line);
                    }
                }
                chars = lines.join("\n").chars().collect();
            }
            5 => {
                let text: String = chars.iter().collect();
                let wrapped = match rng.below(4) {
                    0 => format!("```openui\n{text}\n```"),
                    1 => format!("Here's what I made:\n{text}"),
                    2 => format!("{text}\nThat's it."),
                    _ => format!("```\n{text}"),
                };
                chars = wrapped.chars().collect();
            }
            6 => {
                let text: String = chars.iter().collect();
                chars = text.replace('\n', "\r\n").chars().collect();
            }
            _ => {
                let at = rng.below(n + 1);
                chars.insert(at, *rng.pick(&['"', '\'', '(', '[', '{', '\n', '\\', '`']));
            }
        }
    }
    chars.into_iter().collect()
}
