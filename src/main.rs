mod runnable;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use runnable::{RunHandle, Runnable};
use serde::Deserialize;
use signal_hook::{
    consts::{SIGINT, SIGTERM},
    iterator::Signals,
};
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

        #[arg(
            short = 'S',
            long = "no-subshell",
            help = "Do not run commands in a subshell",
            global = true
        )]
        no_subshell: bool,

        #[arg(
            short = 'B',
            long,
            help = "Do not attach label to output",
            global = true
        )]
        no_label: bool,

        #[arg(long, help = "Include command PID in label", global = true)]
        show_pid: bool,

        #[arg(global = true)]
        commands: Vec<String>,

        #[command(subcommand)]
        command: RunCommands,

        #[arg(
            short = 'w',
            long,
            help = "Working directory for commands",
            global = true
        )]
        working_directory: Option<String>,
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
            show_pid,
            no_color,
            no_label,
            no_subshell,
            mut commands,
            command,
            working_directory,
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

            let labels;

            if no_label {
                labels = vec!["".to_string(); commands.len()];
            } else {
                labels = collect_labels(&commands, &provided_labels, command_as_label, !no_color);
            }

            let mut runnables: Vec<Runnable> = commands
                .iter()
                .zip(labels.into_iter())
                .map(|(command, label)| Runnable {
                    command: command.clone(),
                    working_dir: working_directory.clone(),
                    use_subshell: !no_subshell,
                    label: label.clone(),
                    show_pid,
                })
                .collect();

            match command {
                RunCommands::Serially {} => {
                    run_serially(&mut runnables, SerialOpts { continue_on_error })
                }

                RunCommands::Concurrently { aggregate_output } => run_concurrently(
                    &mut runnables,
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

fn run_serially(runnables: &mut [Runnable], opts: SerialOpts) -> Result<()> {
    let mut command_failed = false;

    for runnable in runnables.into_iter() {
        let handle = runnable.run(false).context("run command")?;
        let handles = vec![handle];

        install_signal_handlers(&handles)?;

        for handle in handles.into_iter() {
            if let Err(err) = handle.wait() {
                eprintln!("{err}");
                command_failed = true;
                if !opts.continue_on_error {
                    break;
                }
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

fn run_concurrently(runnables: &mut [Runnable], opts: ConcurrentOpts) -> Result<()> {
    let mut handles: Vec<RunHandle> = vec![];
    let mut command_failed = false;

    for runnable in runnables {
        let handle = runnable.run(opts.aggregate_output).context("run command")?;
        handles.push(handle);
    }

    install_signal_handlers(&handles)?;

    let (tx, rx) = mpsc::channel::<Result<()>>();

    for handle in handles {
        let tx = tx.clone();

        thread::spawn(move || {
            let res = handle.wait();
            let _ = tx.send(res).or_else(|err| -> Result<()> {
                eprintln!("Failed to send result to main thread: {}", err);
                Ok(())
            });
        });
    }

    drop(tx);

    for result in rx {
        if let Err(err) = result {
            eprintln!("{err}");
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
            format!("\x1b[0;{}m[{}{}]\x1b[0m ", color, label, padding)
        })
        .collect()
}

fn install_signal_handlers(handles: &Vec<RunHandle>) -> Result<()> {
    let pids: Vec<u32> = handles.iter().map(|h| h.get_pid()).collect();
    let mut signals = Signals::new([SIGINT, SIGTERM]).context("register signals")?;
    let mut recv_once = false;

    thread::spawn(move || {
        for signal in signals.forever() {
            match signal {
                SIGINT | SIGTERM => {
                    if recv_once {
                        for pid in pids.iter() {
                            // https://github.com/nix-rust/nix/issues/656#issuecomment-2056684715
                            let pid = nix::unistd::Pid::from_raw(*pid as i32);

                            let _ = nix::sys::signal::kill(pid, nix::sys::signal::SIGKILL)
                                .context("send signal to child")
                                .or_else(|err| -> Result<()> {
                                    eprintln!("Failed to send signal to child process: {}", err);
                                    Ok(())
                                });
                        }

                        // Exit: This ensures we don't continue running the main
                        // thread and potentially spawning more serial commands.
                        std::process::exit(130);
                    } else {
                        recv_once = true;
                    }
                }

                _ => {}
            }
        }
    });

    Ok(())
}
