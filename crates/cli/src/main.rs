mod cmd;
mod util;

use std::{env, ffi::OsStr, fmt::Display, path::PathBuf, process::exit, str::FromStr};

use anyhow::Error;
use argp::{FromArgValue, FromArgs, HelpStyle};
use tracing::level_filters::LevelFilter;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Eq, PartialEq, Copy, Clone)]
enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl FromStr for LogLevel {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "error" => Self::Error,
            "warn" => Self::Warn,
            "info" => Self::Info,
            "debug" => Self::Debug,
            "trace" => Self::Trace,
            _ => return Err(()),
        })
    }
}

impl Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            LogLevel::Error => "error",
            LogLevel::Warn => "warn",
            LogLevel::Info => "info",
            LogLevel::Debug => "debug",
            LogLevel::Trace => "trace",
        })
    }
}

impl FromArgValue for LogLevel {
    fn from_arg_value(value: &OsStr) -> Result<Self, String> {
        String::from_arg_value(value)
            .and_then(|s| Self::from_str(&s).map_err(|_| "Invalid log level".to_string()))
    }
}

#[derive(FromArgs, Debug)]
/// Yet another GameCube/Wii decompilation toolkit.
struct TopLevel {
    #[argp(subcommand)]
    command: SubCommand,
    #[argp(option, short = 'C')]
    /// Change working directory.
    chdir: Option<PathBuf>,
    #[argp(option, short = 'L')]
    /// Minimum logging level. (Default: info)
    /// Possible values: error, warn, info, debug, trace
    log_level: Option<LogLevel>,
    /// Print version information and exit.
    #[argp(switch, short = 'V')]
    #[allow(dead_code)]
    version: bool,
    /// Disable color output. (env: NO_COLOR)
    #[argp(switch)]
    no_color: bool,
}

#[derive(FromArgs, Debug)]
#[argp(subcommand)]
enum SubCommand {
    Changes(cmd::changes::Args),
    // Import(cmd::import::Args),
}

fn main() {
    let args: TopLevel = argp::parse_args_or_exit(&HelpStyle::default());

    let use_colors = !args.no_color;
    let format =
        tracing_subscriber::fmt::format().with_ansi(use_colors).with_target(false).without_time();
    let builder = tracing_subscriber::fmt().event_format(format);
    if let Some(level) = args.log_level {
        builder
            .with_max_level(match level {
                LogLevel::Error => LevelFilter::ERROR,
                LogLevel::Warn => LevelFilter::WARN,
                LogLevel::Info => LevelFilter::INFO,
                LogLevel::Debug => LevelFilter::DEBUG,
                LogLevel::Trace => LevelFilter::TRACE,
            })
            .init();
    } else {
        builder
            .with_env_filter(
                EnvFilter::builder()
                    .with_default_directive(LevelFilter::INFO.into())
                    .from_env_lossy(),
            )
            .init();
    }

    let mut result = Ok(());
    if let Some(dir) = &args.chdir {
        result = env::set_current_dir(dir).map_err(|e| {
            Error::new(e)
                .context(format!("Failed to change working directory to '{}'", dir.display()))
        });
    }
    result = result.and_then(|_| match args.command {
        SubCommand::Changes(c_args) => cmd::changes::run(c_args),
        // SubCommand::Import(c_args) => cmd::import::run(c_args),
    });
    if let Err(e) = result {
        eprintln!("Failed: {e:?}");
        exit(1);
    }
}
