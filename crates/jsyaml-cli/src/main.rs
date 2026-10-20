//! `jsyaml [FILE]`: loads YAML (stdin if no file) and prints it as JSON, or prints the
//! parser events with `--events`. Handy for debugging the port against js-yaml's CLI.

use std::io::Read as _;
use std::path::PathBuf;

use anyhow::{Context as _, Result, bail};
use clap::{Parser, ValueEnum};
use jsyaml::{ConstructorOptions, LoadOptions, Schema};

#[derive(Debug, Clone, Copy, ValueEnum)]
enum SchemaName {
    Failsafe,
    Json,
    Core,
    Yaml11,
}

#[derive(Debug, Parser)]
#[command(version, about)]
struct Args {
    /// YAML file to read; stdin if omitted.
    file: Option<PathBuf>,
    /// Schema to resolve tags with.
    #[arg(long, value_enum, default_value = "core")]
    schema: SchemaName,
    /// Allow duplicate keys, the last one winning (`json: true`).
    #[arg(long)]
    json: bool,
    /// Print parser events instead of values.
    #[arg(long)]
    events: bool,
    /// Pretty-print the JSON output.
    #[arg(long, short)]
    pretty: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let (name, input) = if let Some(path) = &args.file {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        (path.display().to_string(), text)
    } else {
        let mut text = String::new();
        std::io::stdin()
            .read_to_string(&mut text)
            .context("reading stdin")?;
        (String::new(), text)
    };
    let options = LoadOptions {
        constructor: ConstructorOptions {
            schema: match args.schema {
                SchemaName::Failsafe => Schema::failsafe(),
                SchemaName::Json => Schema::json(),
                SchemaName::Core => Schema::core(),
                SchemaName::Yaml11 => Schema::yaml11(),
            },
            json: args.json,
            ..ConstructorOptions::default()
        },
        ..LoadOptions::default()
    };

    if args.events {
        match jsyaml::parse_events(&input, &options.parser) {
            Ok(events) => {
                for event in events {
                    println!("{event:?}");
                }
                return Ok(());
            }
            Err(e) => bail!("{}", e.format(&input, &name)),
        }
    }

    let documents = jsyaml::load_all(&input, &options)
        .map_err(|e| anyhow::anyhow!("{}", e.format(&input, &name)))?;
    for doc in &documents {
        let json = if args.pretty {
            serde_json::to_string_pretty(doc)?
        } else {
            serde_json::to_string(doc)?
        };
        println!("{json}");
    }
    Ok(())
}
