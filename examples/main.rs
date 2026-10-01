mod color;
mod render;

use std::{
    fs,
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
    process::ExitCode,
};

use clap::{Parser, ValueEnum};
use render::{Data, Style};

const DEFAULT_STYLE: &str = include_str!("../assets/default-style.json");

#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    /// Path to the JSON data file
    #[arg(long, value_name = "FILE")]
    data: PathBuf,

    /// Path to a JSON style file (uses the built-in style when omitted)
    #[arg(long, value_name = "FILE")]
    style: Option<PathBuf>,

    /// When to emit ANSI colors
    #[arg(long, value_enum, default_value_t = ColorChoice::Auto)]
    color: ColorChoice,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ColorChoice {
    Auto,
    Always,
    Never,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("yourfetch: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let cli = Cli::parse();
    let data: Data = read_json(&cli.data, "data")?;
    let style: Style = match &cli.style {
        Some(path) => read_json(path, "style")?,
        None => serde_json::from_str(DEFAULT_STYLE)
            .map_err(|error| format!("built-in style is invalid: {error}"))?,
    };
    let colors = match cli.color {
        ColorChoice::Auto => io::stdout().is_terminal(),
        ColorChoice::Always => true,
        ColorChoice::Never => false,
    };

    let output = render::render(&data, &style, colors)?;
    let mut stdout = io::stdout().lock();
    if let Err(error) = stdout.write_all(output.as_bytes())
        && error.kind() != io::ErrorKind::BrokenPipe
    {
        return Err(format!("could not write output: {error}"));
    }
    Ok(())
}

fn read_json<T>(path: &Path, kind: &str) -> Result<T, String>
where
    T: serde::de::DeserializeOwned,
{
    let source = fs::read_to_string(path)
        .map_err(|error| format!("could not read {kind} file {}: {error}", path.display()))?;
    serde_json::from_str(&source)
        .map_err(|error| format!("invalid {kind} JSON in {}: {error}", path.display()))
}
