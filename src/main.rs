use std::{
    collections::HashMap,
    env, fs,
    io::{prelude::*, BufReader},
    process,
    sync::mpsc,
    thread,
    time::Duration,
};

use anyhow::{anyhow, bail, ensure, Result};
use clap::{command, Parser, Subcommand};
use nix::{sys::signal, unistd};
use signal_hook::{
    consts::{SIGINT, SIGTERM},
    iterator::Signals,
};

#[derive(Parser)]
#[command(version, about, long_about = None, bin_name = "konk")]
struct CLI {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    #[command(
        alias = "r",
        about = "Run commands serially or concurrently (alias: r)"
    )]
    Run {
        #[command(subcommand)]
        command: RunCommand,

        #[arg(global = true)]
        commands: Vec<String>,

        #[arg(
            short = 'b',
            long,
            help = "Run package.json scripts with Bun",
            global = true
        )]
        bun: bool,

        #[arg(long, help = "Enable color output", global = true)]
        color: Option<bool>,

        #[arg(
            short = 'L',
            long,
            help = "Use each command as its own label",
            global = true
        )]
        command_as_label: bool,

        #[arg(
            short = 'c',
            long,
            help = "Continue running commands after any failures",
            global = true
        )]
        continue_on_failure: bool,

        #[arg(
            long,
            help = "Time (in seconds) for commands to exit after receiving a SIGINT/SIGTERM before a SIGKILL is sent to them",
            default_value = "10",
            global = true
        )]
        kill_timeout: u16,

        #[arg(
            short = 'l',
            long = "label",
            help = "Label prefix for each command (must match given number of commands)",
            global = true
        )]
        labels: Vec<String>,

        #[arg(long, help = "Do not attach label to output", global = true)]
        no_label: bool,

        #[arg(long, help = "Do not run commands with a subshell", global = true)]
        no_subshell: bool,

        #[arg(
            short = 'n',
            long,
            help = "Run script defined in package.json by name",
            global = true
        )]
        npm: Vec<String>,

        #[arg(long, help = "Include command PID in output", global = true)]
        show_pid: bool,
    },
}

#[derive(Subcommand)]
enum RunCommand {
    #[command(alias = "s", about = "Run commands serially (alias: s)")]
    Serially {},

    #[command(alias = "c", about = "Run commands concurrently (alias: c)")]
    Concurrently {
        #[arg(short = 'g', long, help = "Aggregate command output")]
        aggregate_output: bool,
    },
}

fn main() -> Result<()> {
    let args = CLI::parse();

    match args.command {
        crate::Command::Run {
            command,
            mut commands,
            bun,
            color,
            command_as_label,
            continue_on_failure,
            kill_timeout,
            labels: provided_labels,
            no_label,
            no_subshell,
            npm,
            show_pid,
        } => {
            if let Err(err) =
                collect_npm_commands(&mut commands, &npm, if bun { "bun" } else { "npm" })
            {
                bail!("collecting npm commands: {}", err);
            }

            ensure!(
                !(no_label && command_as_label),
                "Cannot use both --no-label and --command-as-label"
            );

            ensure!(
                provided_labels.len() == 0 || provided_labels.len() == commands.len(),
                "Number of labels must match number of commands"
            );

            ensure!(
                !(provided_labels.len() > 0 && no_label),
                "Cannot use --no-label with --label"
            );

            ensure!(
                !(provided_labels.len() > 0 && command_as_label),
                "Cannot use --command-as-label with --label"
            );

            let labels = if no_label {
                vec!["".to_string(); commands.len()]
            } else {
                let env_no_color = env::var("NO_COLOR").unwrap_or("0".to_string()) != "0";
                let color = color.unwrap_or(!env_no_color);

                collect_labels(
                    &commands,
                    LabelOpts {
                        command_as_label,
                        color,
                        provided_labels,
                    },
                )
            };

            let runnables = commands
                .into_iter()
                .zip(labels)
                .map(|(command, label)| Runnable {
                    label,
                    command,
                    with_pid: show_pid,
                })
                .collect();

            match command {
                RunCommand::Serially {} => {
                    run_serially(
                        runnables,
                        SeriallyOpts {
                            continue_on_failure,
                            kill_timeout,
                            no_subshell,
                        },
                    )?;
                }
                RunCommand::Concurrently { aggregate_output } => {
                    run_concurrently(
                        runnables,
                        ConcurrentlyOpts {
                            aggregate_output,
                            continue_on_failure,
                            kill_timeout,
                            no_subshell,
                        },
                    )?;
                }
            }
        }
    }
    Ok(())
}

struct LabelOpts {
    command_as_label: bool,
    color: bool,
    provided_labels: Vec<String>,
}

fn collect_labels(commands: &[String], opts: LabelOpts) -> Vec<String> {
    let labels: Vec<String> = commands
        .iter()
        .enumerate()
        .map(|(i, command)| {
            if opts.command_as_label {
                command.to_owned()
            } else {
                opts.provided_labels
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| i.to_string())
            }
        })
        .collect();

    let max_len = labels.iter().map(|label| label.len()).max().unwrap_or(0);

    labels
        .into_iter()
        .enumerate()
        .map(|(i, label)| {
            let padding = " ".repeat(max_len - label.len());

            if !opts.color {
                return format!("[{}{}] ", label, padding);
            }

            let color = 31 + (i % 6);
            format!("\x1b[0;{}m[{}{}]\x1b[0m ", color, label, padding)
        })
        .collect()
}

#[derive(serde::Deserialize)]
struct PackageJSON {
    scripts: HashMap<String, String>,
}

fn collect_npm_commands(commands: &mut Vec<String>, npm: &[String], run_with: &str) -> Result<()> {
    if npm.len() == 0 {
        return Ok(());
    }

    let package_json = fs::read_to_string("package.json")?;
    let package_json = serde_json::from_str::<PackageJSON>(&package_json)?;

    for script in npm {
        if let Some(prefix) = script.strip_suffix("*") {
            package_json.scripts.keys().for_each(|key| {
                if key.starts_with(prefix) {
                    commands.push(format!("{} run {}", run_with, key));
                }
            })
        } else {
            if !package_json.scripts.contains_key(script) {
                bail!(r#"Script "{}" does not exist in package.json"#, script)
            }

            commands.push(format!("{} run {}", run_with, script));
        }
    }

    Ok(())
}

struct Runnable {
    label: String,
    with_pid: bool,
    command: String,
}

struct SeriallyOpts {
    continue_on_failure: bool,
    kill_timeout: u16,
    no_subshell: bool,
}

fn run_serially(runnables: Vec<Runnable>, opts: SeriallyOpts) -> Result<()> {
    let mut command_failed = false;

    for runnable in runnables {
        let (pid, handle) = start_command(
            runnable,
            CommandOpts {
                aggregate_output: false,
                no_subshell: opts.no_subshell,
            },
        )?;

        install_signal_handlers(vec![pid], opts.kill_timeout)?;

        let exit_status = handle
            .join()
            .map_err(|e| anyhow!("thread panicked: {:?}", e))??;

        if exit_status.success() {
            continue;
        }

        command_failed = true;

        if !opts.continue_on_failure {
            break;
        }
    }

    if command_failed {
        bail!("One or more commands failed.");
    }

    Ok(())
}

struct ConcurrentlyOpts {
    aggregate_output: bool,
    continue_on_failure: bool,
    kill_timeout: u16,
    no_subshell: bool,
}

fn run_concurrently(runnables: Vec<Runnable>, opts: ConcurrentlyOpts) -> Result<()> {
    let (tx, rx) = mpsc::channel::<Result<process::ExitStatus>>();
    let mut pids: Vec<u32> = Vec::new();

    for runnable in runnables {
        let (pid, handle) = start_command(
            runnable,
            CommandOpts {
                aggregate_output: opts.aggregate_output,
                no_subshell: opts.no_subshell,
            },
        )?;

        pids.push(pid);

        let tx = tx.clone();
        thread::spawn(move || {
            match handle
                .join()
                .map_err(|e| anyhow!("thread panicked: {:?}", e))
            {
                Ok(r) => tx.send(r).unwrap(),
                Err(e) => tx.send(Err(e)).unwrap(),
            };
        });
    }

    install_signal_handlers(pids, opts.kill_timeout)?;

    drop(tx);

    let mut command_failed = false;

    for result in rx {
        match result {
            Ok(exit_status) => {
                if exit_status.success() {
                    continue;
                }

                command_failed = true;

                if !opts.continue_on_failure {
                    break;
                }
            }

            Err(e) => return Err(e),
        }
    }

    if command_failed {
        bail!("One or more commands failed.");
    }

    Ok(())
}

struct CommandOpts {
    aggregate_output: bool,
    no_subshell: bool,
}

fn start_command(
    runnable: Runnable,
    opts: CommandOpts,
) -> Result<(u32, thread::JoinHandle<Result<process::ExitStatus>>)> {
    let mut cmd;
    if opts.no_subshell {
        let parts = shell_words::split(&runnable.command)?;
        let (command, args) = parts.split_first().ok_or_else(|| anyhow!("no command"))?;
        cmd = process::Command::new(command);
        cmd.args(args);
    } else {
        cmd = process::Command::new("/bin/sh");
        cmd.args(["-c", &runnable.command]);
    }

    let mut child = cmd
        .stdout(process::Stdio::piped())
        .stderr(process::Stdio::piped())
        .spawn()?;

    let (tx, rx) = mpsc::channel::<String>();

    let stdout = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;
    let stdout_handle = read_stream(stdout, tx.clone());

    let stderr = child.stderr.take().ok_or_else(|| anyhow!("no stderr"))?;
    let stderr_handle = read_stream(stderr, tx.clone());

    let pid = child.id();

    Ok((
        pid,
        thread::spawn(move || -> Result<process::ExitStatus> {
            drop(tx);

            let mut lines = Vec::<String>::new();

            let label = if runnable.with_pid {
                format!("{}(PID: {}) ", runnable.label, pid)
            } else {
                runnable.label
            };

            for mut line in rx {
                line = format!("{}{}", label, line);

                if opts.aggregate_output {
                    lines.push(line);
                } else {
                    println!("{}", line);
                }
            }

            stdout_handle
                .join()
                .map_err(|e| anyhow!("thread panicked: {:?}", e))??;

            stderr_handle
                .join()
                .map_err(|e| anyhow!("thread panicked: {:?}", e))??;

            let exit_status = child.wait()?;

            for line in lines.iter() {
                println!("{}", line);
            }

            eprintln!("{}{}", label, exit_status);

            Ok(exit_status)
        }),
    ))
}

fn read_stream<R>(stream: R, into: mpsc::Sender<String>) -> thread::JoinHandle<Result<()>>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || -> Result<()> {
        let reader = BufReader::new(stream);

        for line in reader.lines() {
            into.send(line?)?;
        }

        Ok(())
    })
}

fn install_signal_handlers(pids: Vec<u32>, timeout: u16) -> Result<()> {
    let mut signals = Signals::new([SIGINT, SIGTERM])?;

    thread::spawn(move || {
        let mut received_signal = false;

        for signal in signals.forever() {
            if let SIGINT | SIGTERM = signal {
                if received_signal {
                    eprintln!("Received signal again. Killing processes.");
                    kill_all_and_exit(&pids);
                } else {
                    received_signal = true;

                    let timeout = timeout.clone();
                    let pids = pids.clone();

                    thread::spawn(move || {
                        thread::sleep(Duration::from_secs(timeout.to_owned().into()));
                        eprintln!("Timeout. Killing child processes.");
                        kill_all_and_exit(&pids);
                    });

                    eprintln!("Received signal. Waiting for child processes to exit.");
                }
            }
        }
    });

    Ok(())
}

fn kill_all_and_exit(pids: &[u32]) {
    pids.iter().for_each(kill_process);
    process::exit(130);
}

fn kill_process(pid: &u32) {
    // https://github.com/nix-rust/nix/issues/656#issuecomment-2056684715
    let pid = unistd::Pid::from_raw(pid.to_owned() as i32);

    eprintln!("Sending SIGKILL to process {}.", pid);

    match signal::kill(pid, signal::SIGKILL) {
        Err(e) => eprintln!("Failed to send SIGKILL to process {}: {:?}", pid, e),
        Ok(_) => {}
    };
}
