//! Random programs: generated from a component library's grammar, or real
//! corpus programs mangled the ways model output gets mangled.

use openui_lang::Library;

use crate::rng::Rng;

const STATEMENT_IDS: [&str; 12] = [
    "a", "b", "c", "items", "data", "header", "Main", "x1", "$count", "$flag", "$q", "save",
];

const STRING_PIECES: [&str; 22] = [
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

const BUILTIN_CALLS: [&str; 10] = [
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
];

/// Characters and fragments the lexer and preprocessing treat specially.
const NOISE: [&str; 30] = [
    "\"", "'", "\\", "(", ")", "[", "]", "{", "}", ",", ":", "=", "==", "!", "?", "&", "|", "-",
    ".", "$", "@", "#", "//", "```", "\n", "\t", "\r\n", "😀", "\\u", "e",
];

pub struct Generator<'a> {
    rng: &'a mut Rng,
    components: Vec<(String, usize)>,
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
        Generator {
            rng,
            components,
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

    fn query(&mut self) -> String {
        let mut args = vec![format!("\"tool_{}\"", self.rng.below(3)), self.object(1)];
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
        let n = self.rng.below(params + 2).min(5);
        let args: Vec<String> = (0..n).map(|_| self.expr(depth + 1)).collect();
        format!("{name}({})", args.join(", "))
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
        match self.rng.below(100) {
            0..=9 => format!("```openui\n{text}\n```"),
            10..=14 => format!("Here's the UI:\n```openui\n{text}\n```\nHope that helps!"),
            15..=19 => format!("Here is the UI:\n{text}"),
            20..=22 => format!("Here's the UI:\n{text}"),
            23..=26 => format!("{text}\n\nLet me know if you'd like changes."),
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
