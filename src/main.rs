mod tui;

use nocur::{dsl, editor::Editor, MAX_BUFFER_BYTES};
use std::{
    fs::File,
    io::{self, Read},
    path::PathBuf,
};

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let input = match args.next() {
        Some(s) if s != "--help" && s != "-h" => PathBuf::from(s),
        _ => {
            println!("Usage: nocur FILE [--prompt TEXT] [--eval EXPRESSION]\nF2 commit | Ctrl+Z undo | Ctrl+Y redo | Ctrl+S save | Ctrl+E export | Ctrl+Q quit\nEnter newline | Ctrl+U clear expression | PageUp/PageDown scroll preview");
            return Ok(());
        }
    };
    let output = PathBuf::from(format!("{}.edited", input.display()));
    let mut prompt = String::from(":)");
    let mut expression = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--prompt" => prompt = args.next().ok_or("Missing prompt text")?,
            "--eval" => expression = Some(args.next().ok_or("Missing expression")?),
            _ => return Err(format!("Unknown argument: {arg}").into()),
        }
    }
    let mut buffer = String::new();
    File::open(input)?
        .take((MAX_BUFFER_BYTES + 1) as u64)
        .read_to_string(&mut buffer)?;
    if buffer.len() > MAX_BUFFER_BYTES {
        return Err("Input exceeds 8 MiB".into());
    }
    if let Some(source) = expression {
        let expr = dsl::parse(&source).map_err(io::Error::other)?;
        let result = dsl::eval(&expr, &buffer).map_err(io::Error::other)?;
        use std::io::Write;
        io::stdout().write_all(result.as_bytes())?;
        return Ok(());
    }
    tui::run(Editor::new(buffer), output, prompt)
}
