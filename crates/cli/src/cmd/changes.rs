use anyhow::{Context, Result};
use argp::FromArgs;
use decomp_dev_github::changes::{generate_changes, generate_comment};
use objdiff_core::bindings::report::Report;
use typed_path::Utf8NativePathBuf;

use crate::util::native_path;

#[derive(FromArgs, PartialEq, Eq, Debug)]
/// Calculate changes between two reports and print a markdown summary.
#[argp(subcommand, name = "changes")]
pub struct Args {
    #[argp(option, short = '1', from_str_fn(native_path))]
    /// previous report file
    previous: Utf8NativePathBuf,
    #[argp(option, short = '2', from_str_fn(native_path))]
    /// current report file
    current: Utf8NativePathBuf,
    #[argp(option, short = 'o', from_str_fn(native_path))]
    /// write markdown changes to output file
    output: Option<Utf8NativePathBuf>,
}

pub fn run(args: Args) -> Result<()> {
    let previous_report_data = std::fs::read(args.previous.with_platform_encoding())
        .with_context(|| format!("Failed to read {}", args.previous))?;
    let current_report_data = std::fs::read(args.current.with_platform_encoding())
        .with_context(|| format!("Failed to read {}", args.current))?;
    let previous_report = Report::parse(previous_report_data.as_slice())
        .with_context(|| format!("Failed to parse {}", args.previous))?;
    let current_report = Report::parse(current_report_data.as_slice())
        .with_context(|| format!("Failed to parse {}", args.current))?;
    let changes = generate_changes(&previous_report, &current_report)
        .context("Failed to generate changes")?;
    let comment = generate_comment(&previous_report, &current_report, None, None, None, changes);
    if let Some(out_path) = &args.output {
        std::fs::write(out_path.with_platform_encoding(), comment)
            .with_context(|| format!("Failed to write output file '{}'", out_path))?;
    } else {
        println!("{}", comment);
    }
    Ok(())
}
