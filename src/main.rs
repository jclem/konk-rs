use clap::{Parser, Subcommand};
use serde::Deserialize;
use std::{
    io::BufRead,
    process::{Command, ExitStatus, Stdio},
    thread::JoinHandle,
};

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
    Concurrently {},
}

fn main() -> std::io::Result<()> {
    let args = Args::parse();

    match args.command {
        Commands::Run {
            npm,
            bun,
            command_as_label,
            continue_on_error,
            labels,
            no_color,
            mut commands,
            command,
        } => {
            if npm.len() > 0 {
                // Read the package.json file
                let package_json = std::fs::read_to_string("package.json")?;
                let package_json: PackageJSON = serde_json::from_str(&package_json)?;
                let run_with = if bun { "bun" } else { "npm" };

                for script in npm {
                    if script.ends_with("*") {
                        let prefix = script.strip_suffix("*").unwrap();

                        for (key, _) in package_json.scripts.iter() {
                            if key.starts_with(prefix) {
                                commands.push(format!("{} run {}", run_with, key));
                            }
                        }
                    } else {
                        assert!(
                            package_json.scripts.contains_key(&script),
                            "Script does not exist in package.json"
                        );

                        commands.push(format!("{} run {}", run_with, script));
                    }
                }
            }

            if labels.len() > 0 && labels.len() != commands.len() {
                eprintln!("Number of labels must match number of commands");
                std::process::exit(1);
            }

            if labels.len() > 0 && command_as_label {
                eprintln!("Cannot use -L and -l together");
                std::process::exit(1);
            }

            let labels: Vec<String> = commands
                .iter()
                .enumerate()
                .map(|(i, command)| {
                    if command_as_label {
                        command.clone()
                    } else {
                        format!("{i}")
                    }
                })
                .collect();

            let max_label_len = labels
                .iter()
                .max_by_key(|label| label.len())
                .unwrap_or(&String::from(""))
                .len();

            let labels: Vec<String> = labels
                .iter()
                .enumerate()
                .map(|(i, label)| {
                    let color = 31 + (i % 9);
                    let color = if no_color { 0 } else { color };
                    let padding = max_label_len - label.len();

                    format!("\x1b[0;{}m[{}{}]\x1b[0m", color, label, " ".repeat(padding))
                })
                .collect();

            match command {
                RunCommands::Serially {} => run_serially(
                    commands,
                    SerialOpts {
                        continue_on_error,
                        labels,
                    },
                ),
                RunCommands::Concurrently {} => run_concurrently(
                    commands,
                    ConcurrentOpts {
                        continue_on_error,
                        labels,
                    },
                ),
            }
        }
    }
}

struct SerialOpts {
    continue_on_error: bool,
    labels: Vec<String>,
}

fn run_serially(commands: Vec<String>, opts: SerialOpts) -> std::io::Result<()> {
    let mut command_failed = false;

    for (i, command) in commands.into_iter().enumerate() {
        let mut child = Command::new("/bin/sh")
            .args(["-c", &command])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("process spawned");

        let label = opts.labels.get(i).expect("label should exist");
        let stdout = child.stdout.take().expect("child should have stdout");
        let stderr = child.stderr.take().expect("child should have stderr");

        let stdout_label = label.clone();

        let stdout_thread = std::thread::spawn(move || {
            let reader = std::io::BufReader::new(stdout);

            for line in reader.lines() {
                let line = line.unwrap();
                println!("{} {}", stdout_label, line);
            }
        });

        let stderr_label = label.clone();

        let stderr_thread = std::thread::spawn(move || {
            let reader = std::io::BufReader::new(stderr);

            for line in reader.lines() {
                let line = line.unwrap();
                println!("{}: {}", stderr_label, line);
            }
        });

        stdout_thread.join().expect("thread should finish");
        stderr_thread.join().expect("thread should finish");

        let status = child.wait()?;
        if !status.success() {
            eprintln!("{} command exited with status: {}", label, status);

            command_failed = true;

            if !opts.continue_on_error {
                std::process::exit(1);
            }
        }
    }

    if command_failed {
        std::process::exit(1);
    }

    Ok(())
}

struct ConcurrentOpts {
    continue_on_error: bool,
    labels: Vec<String>,
}

fn run_concurrently(commands: Vec<String>, opts: ConcurrentOpts) -> std::io::Result<()> {
    let mut threads: Vec<JoinHandle<ExitStatus>> = vec![];
    let mut command_failed = false;

    for (i, command) in commands.into_iter().enumerate() {
        command_failed = true;

        let label = opts.labels.get(i).expect("label should exist").clone();

        threads.push(std::thread::spawn(move || {
            let mut child = Command::new("/bin/sh")
                .args(["-c", &command])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("process spawned");

            let stdout = child.stdout.take().expect("child should have stdout");
            let stderr = child.stderr.take().expect("child should have stderr");

            let stdout_label = label.clone();

            let stdout_thread = std::thread::spawn(move || {
                let reader = std::io::BufReader::new(stdout);

                for line in reader.lines() {
                    let line = line.unwrap();
                    println!("{} {}", stdout_label, line);
                }
            });

            let stderr_label = label.clone();

            let stderr_thread = std::thread::spawn(move || {
                let reader = std::io::BufReader::new(stderr);

                for line in reader.lines() {
                    let line = line.unwrap();
                    println!("{} {}", stderr_label, line);
                }
            });

            stdout_thread.join().expect("thread should finish");
            stderr_thread.join().expect("thread should finish");

            let status = child.wait();
            match status {
                Ok(status) => {
                    if !status.success() {
                        eprintln!("{} command exited with status: {}", label, status);

                        if !opts.continue_on_error {
                            std::process::exit(1);
                        }
                    }

                    status
                }
                Err(e) => {
                    eprintln!("{} error: {}", label, e);
                    std::process::exit(1);
                }
            }
        }))
    }

    for thread in threads {
        thread.join().expect("thread should finish");
    }

    if command_failed {
        std::process::exit(1);
    }

    Ok(())
}
