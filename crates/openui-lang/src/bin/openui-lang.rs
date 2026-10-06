//! Command-line tool for the OpenUI Lang parser.
//!
//!   openui-lang parse <schema.json> <program|->    one-shot parse, prints the result JSON
//!   openui-lang stream <schema.json> <program|-> [chunk]
//!                                                   streaming parse in chunks of `chunk` chars
//!                                                   (default 4), one result JSON per line
//!   openui-lang tokens <program|->                  prints the tokens as JSON

use std::io::Read;
use std::process::ExitCode;

use std::sync::Arc;

use openui_lang::{Library, StreamParser, parse, tokenize, tokens_to_json};

const USAGE: &str =
    "usage: openui-lang parse <schema.json> <program|->\n       openui-lang tokens <program|->";

fn read_input(path: &str) -> Result<String, String> {
    if path == "-" {
        let mut s = String::new();
        std::io::stdin()
            .read_to_string(&mut s)
            .map_err(|e| format!("read stdin: {e}"))?;
        Ok(s)
    } else {
        std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))
    }
}

fn load_library(path: &str) -> Result<Library, String> {
    let schema: serde_json::Value =
        serde_json::from_str(&read_input(path)?).map_err(|e| format!("{path}: {e}"))?;
    Ok(Library::from_json_schema(&schema))
}

fn run(args: &[String]) -> Result<(), String> {
    match args {
        [command, schema, program] if command == "parse" => {
            let library = load_library(schema)?;
            println!("{}", parse(&read_input(program)?, &library, None).to_json());
            Ok(())
        }
        [command, schema, program, rest @ ..] if command == "stream" && rest.len() <= 1 => {
            let size: usize = match rest.first() {
                Some(n) => n
                    .parse()
                    .ok()
                    .filter(|&n| n > 0)
                    .ok_or("chunk must be a positive number")?,
                None => 4,
            };
            let library = Arc::new(load_library(schema)?);
            let text = read_input(program)?;
            let mut parser = StreamParser::new(library, None);
            let chars: Vec<(usize, char)> = text.char_indices().collect();
            for group in chars.chunks(size) {
                let start = group[0].0;
                let end = group.last().map_or(start, |&(i, c)| i + c.len_utf8());
                println!("{}", parser.push(&text[start..end]).to_json());
            }
            Ok(())
        }
        [command, program] if command == "tokens" => {
            println!("{}", tokens_to_json(&tokenize(&read_input(program)?)));
            Ok(())
        }
        _ => Err(USAGE.to_owned()),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}
