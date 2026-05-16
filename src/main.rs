use std::io::{self, Write};
use std::process;
use std::sync::Arc;
use std::time::Instant;

use clap::Parser;
use tracing_subscriber::EnvFilter;

mod cli;

use cli::args::{Cli, OutputMode};
use cli::output::{StatsSnapshot, emit_json, emit_json_error, emit_json_summary};
use cli::render::{RenderOptions, collect_summary, render_dupes_only, render_text, summary_line};

use fifi::{ScanOptions, TracingProgress, scan};

const EXIT_DUPS_FOUND: i32 = 0;
const EXIT_NO_DUPS: i32 = 1;
const EXIT_ERROR: i32 = 2;

fn main() {
    install_sigpipe_default();

    let cli = Cli::parse();
    let mode = cli.output_mode();
    init_tracing(cli.verbose, cli.quiet);

    match run(&cli, mode) {
        Ok(found) => process::exit(if found { EXIT_DUPS_FOUND } else { EXIT_NO_DUPS }),
        Err(e) => {
            match mode {
                OutputMode::Json => emit_json_error(&format!("{e:#}")),
                _ => eprintln!("error: {e:#}"),
            }
            process::exit(EXIT_ERROR);
        }
    }
}

fn install_sigpipe_default() {
    // Restore default SIGPIPE so `fifi /huge | head` exits cleanly instead
    // of panicking on a broken pipe in the middle of println!.
    unsafe {
        let _ = nix::sys::signal::signal(
            nix::sys::signal::Signal::SIGPIPE,
            nix::sys::signal::SigHandler::SigDfl,
        );
    }
}

fn init_tracing(verbose: u8, quiet: bool) {
    let level = if quiet {
        "error"
    } else {
        match verbose {
            0 => "warn",
            1 => "info",
            2 => "debug",
            _ => "trace",
        }
    };
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_writer(std::io::stderr)
        .try_init();
}

fn run(cli: &Cli, mode: OutputMode) -> anyhow::Result<bool> {
    let algo = cli.algo.into_strategy();
    let algo_name = algo.name();

    let mut opts = ScanOptions::new(algo);
    opts.follow = cli.follow;
    opts.include_hidden = cli.hidden;
    opts.one_file_system = cli.one_file_system;
    opts.per_path = cli.per_path;
    if cli.verbose >= 1 {
        opts.progress = Some(Arc::new(TracingProgress::new()));
    }

    let started = Instant::now();
    let result = scan(&cli.path, &opts)?;
    let elapsed = started.elapsed().as_secs_f64();

    let stdout = io::stdout();
    let mut stdout = stdout.lock();

    match mode {
        OutputMode::Text => {
            render_text(
                &mut stdout,
                &result,
                &RenderOptions {
                    include_unique: cli.unique,
                },
            )?;
            if cli.verbose >= 1 {
                let stats = collect_summary(&result, elapsed);
                eprintln!("{}", summary_line(&stats));
            }
        }
        OutputMode::Json => {
            emit_json(
                &mut stdout,
                &cli.path,
                &result,
                cli.unique,
                algo_name,
                StatsSnapshot {
                    elapsed_seconds: elapsed,
                },
            )?;
        }
        OutputMode::SummaryJson => {
            emit_json_summary(
                &mut stdout,
                &result,
                StatsSnapshot {
                    elapsed_seconds: elapsed,
                },
            )?;
        }
        OutputMode::DupesOnly => {
            render_dupes_only(&mut stdout, &result)?;
        }
        OutputMode::Summary => {
            let stats = collect_summary(&result, elapsed);
            // --summary as the sole output mode goes to stdout so it's
            // pipeable. (When --summary is implied by -v, we route it to
            // stderr alongside the rest of the verbose output.)
            writeln!(stdout, "{}", summary_line(&stats))?;
        }
    }

    Ok(result.has_duplicates())
}
