use std::io::{self, Write};
use std::process;
use std::sync::Arc;
use std::time::Instant;

use clap::Parser;
use tracing_subscriber::EnvFilter;

mod cli;

use cli::args::{Cli, OutputMode};
use cli::output::{StatsSnapshot, emit_json, emit_json_error, emit_json_summary};
use cli::render::{
    collect_summary, render_print0, render_text, render_unique, render_unique_print0, summary_line,
};

use fifi::{ScanOptions, TracingProgress, scan};

// Linter-style convention: 0 = nothing to act on, 1 = findings, 2 = error.
// What counts as a finding follows the listing's subject — duplicate groups
// by default, unique files under --unique.
const EXIT_CLEAN: i32 = 0;
const EXIT_FOUND: i32 = 1;
const EXIT_ERROR: i32 = 2;

fn main() {
    install_sigpipe_default();

    let cli = Cli::parse();
    let mode = cli.output_mode();
    init_tracing(cli.verbose, cli.quiet);

    match run(&cli, mode) {
        Ok(code) => process::exit(code),
        Err(e) => {
            match mode {
                OutputMode::Json | OutputMode::SummaryJson => emit_json_error(&format!("{e:#}")),
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

fn run(cli: &Cli, mode: OutputMode) -> anyhow::Result<i32> {
    if let (Some(min), Some(max)) = (cli.min_size, cli.max_size) {
        anyhow::ensure!(
            min <= max,
            "--min-size ({min}) is greater than --max-size ({max}); no file can match"
        );
    }

    let algo = cli.algo.into_strategy();
    let algo_name = algo.name();

    let mut opts = ScanOptions::new(algo);
    opts.follow = cli.follow;
    opts.include_hidden = cli.hidden;
    opts.one_file_system = cli.one_file_system;
    opts.per_path = cli.per_path;
    opts.depth = cli.depth;
    opts.min_size = cli.min_size;
    opts.max_size = cli.max_size;
    opts.order_by = cli.order_by.into_order_by();
    opts.sort_groups = cli.sort_groups.into_sort_groups();
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
            if cli.unique {
                render_unique(&mut stdout, &result)?;
            } else {
                render_text(&mut stdout, &result, cli.dupes_only)?;
            }
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
                &StatsSnapshot {
                    elapsed_seconds: elapsed,
                },
            )?;
        }
        OutputMode::SummaryJson => {
            emit_json_summary(
                &mut stdout,
                &result,
                &StatsSnapshot {
                    elapsed_seconds: elapsed,
                },
            )?;
        }
        OutputMode::Print0 => {
            if cli.unique {
                render_unique_print0(&mut stdout, &result)?;
            } else {
                render_print0(&mut stdout, &result, cli.dupes_only)?;
            }
        }
        OutputMode::Summary => {
            let stats = collect_summary(&result, elapsed);
            // --summary as the sole output mode goes to stdout so it's
            // pipeable. (When --summary is implied by -v, we route it to
            // stderr alongside the rest of the verbose output.)
            writeln!(stdout, "{}", summary_line(&stats))?;
        }
    }

    // A scan root the user named but we couldn't access means the results
    // above cover less than was asked for — that's an error, not a clean
    // "no duplicates". The results are still rendered (everything reachable
    // was scanned); only the exit code and stderr carry the failure.
    if !result.missing_roots.is_empty() {
        for root in &result.missing_roots {
            eprintln!(
                "error: cannot scan '{}': path is missing or not accessible",
                root.display()
            );
        }
        return Ok(EXIT_ERROR);
    }

    // The exit code follows whatever the run *lists*: unique files under
    // `--unique`, duplicate groups otherwise. The summary modes are
    // subject-independent (they report duplicate stats either way), so
    // `--unique` is inert there and the code reflects duplicates.
    let lists_unique = cli.unique && !matches!(mode, OutputMode::Summary | OutputMode::SummaryJson);
    let found = if lists_unique {
        !result.unique.is_empty()
    } else {
        result.has_duplicates()
    };
    Ok(if found { EXIT_FOUND } else { EXIT_CLEAN })
}
