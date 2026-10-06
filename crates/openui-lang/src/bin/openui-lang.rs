//! Command-line tool for the OpenUI Lang parser.
//!
//!   openui-lang parse <schema.json> <program|->    one-shot parse, prints the result JSON

use std::io::Read;
use std::process::ExitCode;

use openui_lang::{Library, parse};

const USAGE: &str = "usage: openui-lang parse <schema.json> <program|->";

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
