mod runnable;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use runnable::{RunHandle, Runnable};
use serde::Deserialize;
use std::{sync::mpsc, thread};

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Deserialize, Debug)]
struct PackageJSON {
    scripts: std::collections::HashMap<String, String>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    #[command(
        alias = "r",
        about = "Run commands serially or concurrently (alias: r)"
    )]
    Run {
        #[arg(
            short = 'n',
            long,
            help = "Run script from package.json",
            global = true
        )]
        npm: Vec<String>,

        #[arg(
            short = 'b',
            long,
            help = "Run package.json scripts with Bun",
            global = true
        )]
        bun: bool,

        #[arg(
            short = 'L',
            long,
            help = "Use command as its own label",
            global = true
        )]
        command_as_label: bool,

        #[arg(
            short = 'c',
            long,
            help = "Continue running commands after a failure",
            global = true
        )]
        continue_on_error: bool,

        #[arg(
            short = 'l',
            long = "label",
            help = "Label prefix for a command",
            global = true
        )]
        labels: Vec<String>,

        #[arg(
            short = 'C',
            long = "no-color",
            help = "Do not colorize label output",
            global = true
        )]
        no_color: bool,

        #[arg(global = true)]
        commands: Vec<String>,

        #[command(subcommand)]
        command: RunCommands,
    },
}

#[derive(Subcommand, Debug)]
enum RunCommands {
    #[command(alias = "s", about = "Run commands serially (alias: s)")]
    Serially {},

    #[command(alias = "c", about = "Run commands concurrently (alias: c)")]
    Concurrently {
        #[arg(short = 'g', long, help = "Aggregate command output", global = true)]
        aggregate_output: bool,
    },
}

fn main() -> Result<()> {
    let args = Args::parse();

    match args.command {
        Commands::Run {
            npm,
            bun,
            command_as_label,
            continue_on_error,
            labels: provided_labels,
            no_color,
            mut commands,
            command,
        } => {
            if let Err(err) = add_npm_commands(&mut commands, &npm, bun) {
                anyhow::bail!("adding npm commands: {}", err);
            }

            if provided_labels.len() > 0 && provided_labels.len() != commands.len() {
                anyhow::bail!("Number of provided_labels must match number of commands");
            }

            if provided_labels.len() > 0 && command_as_label {
                anyhow::bail!("Cannot use -L and -l together");
            }

            let labels = collect_labels(&commands, &provided_labels, command_as_label, !no_color);

            let runnables: Vec<Runnable> = commands
                .iter()
                .zip(labels.into_iter())
                .map(|(command, label)| Runnable {
                    command: command.clone(),
                    label: label.clone(),
                })
                .collect();

            match command {
                RunCommands::Serially {} => {
                    run_serially(&runnables, SerialOpts { continue_on_error })
                }

                RunCommands::Concurrently { aggregate_output } => run_concurrently(
                    &runnables,
                    ConcurrentOpts {
                        continue_on_error,
                        aggregate_output,
                    },
                ),
            }
        }
    }
}

struct SerialOpts {
    continue_on_error: bool,
}

fn run_serially(runnables: &[Runnable], opts: SerialOpts) -> Result<()> {
    let mut command_failed = false;

    for runnable in runnables.into_iter() {
        if let Err(_) = runnable.run(false).context("run command")?.wait() {
            command_failed = true;
            if !opts.continue_on_error {
                break;
            }
        }
    }

    if command_failed {
        anyhow::bail!("One or more commands failed");
    }

    Ok(())
}

struct ConcurrentOpts {
    continue_on_error: bool,
    aggregate_output: bool,
}

fn run_concurrently(runnables: &[Runnable], opts: ConcurrentOpts) -> Result<()> {
    let mut handles: Vec<RunHandle> = vec![];
    let mut command_failed = false;

    for runnable in runnables {
        let handle = runnable.run(opts.aggregate_output).context("run command")?;
        handles.push(handle);
    }

    let (tx, rx) = mpsc::channel::<Result<()>>();

    for handle in handles {
        let tx = tx.clone();

        thread::spawn(move || {
            let res = handle.wait();
            let _ = tx.send(res); // Ignore send error.
        });
    }

    drop(tx);

    for result in rx {
        if let Err(_) = result {
            command_failed = true;
            if !opts.continue_on_error {
                break;
            }
        }
    }

    if command_failed {
        anyhow::bail!("One or more commands failed");
    }

    Ok(())
}

fn add_npm_commands(commands: &mut Vec<String>, npm: &[String], use_bun: bool) -> Result<()> {
    if npm.len() == 0 {
        return Ok(());
    }

    let package_json = std::fs::read_to_string("package.json").context("read package.json")?;

    let package_json: PackageJSON =
        serde_json::from_str(&package_json).context("parse package.json")?;

    let run_with = if use_bun { "bun" } else { "npm" };

    for script in npm {
        if script.ends_with("*") {
            let prefix = script.strip_suffix("*").unwrap(); // Already checked suffix.

            package_json.scripts.keys().for_each(|key| {
                key.starts_with(prefix).then(|| {
                    commands.push(format!("{} run {}", run_with, key));
                });
            });
        } else {
            if !package_json.scripts.contains_key(script) {
                anyhow::bail!(r#"Script "{}" does not exist in package.json"#, script);
            }

            commands.push(format!("{} run {}", run_with, script));
        }
    }

    Ok(())
}

fn collect_labels(
    commands: &[String],
    provided_labels: &[String],
    command_as_label: bool,
    use_color: bool,
) -> Vec<String> {
    let labels: Vec<String> = if command_as_label {
        commands.iter().cloned().collect()
    } else {
        commands
            .iter()
            .enumerate()
            .map(|(i, _)| {
                provided_labels
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| i.to_string())
            })
            .collect()
    };

    let max_label_len = labels.iter().map(|l| l.len()).max().unwrap_or_default();

    labels
        .into_iter()
        .enumerate()
        .map(|(i, label)| {
            let color = if use_color { 31 + (i % 9) } else { 0 };
            let padding = " ".repeat(max_label_len - label.len());
            format!("\x1b[0;{}m[{}{}]\x1b[0m", color, label, padding)
        })
        .collect()
}
