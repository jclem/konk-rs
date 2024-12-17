use anyhow::{Context, Result};
use std::{
    fmt,
    fs::canonicalize,
    io::{BufRead, BufReader},
    process::{Command, ExitStatus, Stdio},
    sync::{Arc, Mutex},
    thread::{spawn, JoinHandle},
};

/// A representation of a command to be run and streamed to stdout
#[derive(Debug)]
pub struct Runnable {
    /// The label to prepend to each line of output
    pub label: String,

    /// Whether to show the PID of the command after the label
    pub show_pid: bool,

    /// The command and its arguments to run as a single string
    ///
    /// If [use_subshell](Runnable::use_subshell) is true, this will be run in a subshell (/bin/sh).
    /// Otherwise, the command will be split into a command and its arguments.
    pub command: String,

    /// The working directory to run the command in
    pub working_dir: Option<String>,

    /// Whether to run the command in a subshell (/bin/sh)
    pub use_subshell: bool,
}

impl Runnable {
    /// Runs the runnable command
    ///
    /// If `aggregate_output` is true, the output of the command will be
    /// collected and printed at the end. Otherwise, the output will be
    /// streamed to stdout as it is produced.
    ///
    /// Returns a [RunHandle] that can be used to wait for the
    /// command to finish.
    pub fn run(&mut self, aggregate_output: bool) -> Result<RunHandle> {
        let mut child;

        let working_dir;
        if let Some(dir) = &self.working_dir {
            working_dir = Some(canonicalize(dir).context("canonicalize working directory")?);
        } else {
            working_dir = None;
        }

        if self.use_subshell {
            let mut cmd = Command::new("/bin/sh");
            cmd.args(["-c", &self.command]);

            if let Some(working_dir) = working_dir {
                cmd.current_dir(working_dir);
            }

            child = cmd
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .context(format!("spawn process: {}", self.command))?;
        } else {
            let parts = shell_words::split(&self.command).context("split command")?;
            let command = parts.get(0).context("get command")?;
            let rest = parts.get(1..).context("get arguments")?;

            let mut cmd = Command::new(command);
            cmd.args(rest);

            if let Some(working_dir) = working_dir {
                cmd.current_dir(working_dir);
            }

            child = cmd
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .context(format!("spawn process: {command}"))?;
        }

        if self.show_pid {
            self.label = format!("{}(PID: {}) ", self.label, child.id());
        }

        let stdout = child.stdout.take().context("get child stdout")?;
        let stderr = child.stderr.take().context("get child stderr")?;

        let lines: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

        let out_lines = lines.clone();
        let out_label = self.label.clone();

        let out_handle: JoinHandle<Result<()>> = spawn(move || {
            let reader = BufReader::new(stdout);

            for line in reader.lines() {
                let line = line.context("read line")?;

                if aggregate_output {
                    out_lines.lock().unwrap().push(line.clone());
                } else {
                    println!("{out_label}{line}");
                }
            }

            Ok(())
        });

        let err_lines = lines.clone();
        let err_label = self.label.clone();

        let err_handle: JoinHandle<Result<()>> = spawn(move || {
            let reader = BufReader::new(stderr);

            for line in reader.lines() {
                let line = line.context("read line")?;

                if aggregate_output {
                    err_lines.lock().unwrap().push(line.clone());
                } else {
                    println!("{err_label}{line}");
                }
            }

            Ok(())
        });

        Ok(RunHandle {
            child,
            label: self.label.clone(),
            err_handle,
            out_handle,
            output: lines,
        })
    }
}

/// A handle to a running command, which can be used to wait for the command to
/// finish
#[derive(Debug)]
pub struct RunHandle {
    child: std::process::Child,
    label: String,
    out_handle: JoinHandle<Result<()>>,
    err_handle: JoinHandle<Result<()>>,
    output: Arc<Mutex<Vec<String>>>,
}

/// An error indicating that a command exited with a non-zero status
#[derive(Debug)]
pub struct ExitStatusError {
    /// The exit status of the command
    pub status: ExitStatus,
    label: String,
}

impl fmt::Display for ExitStatusError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{} command exited with non-zero status: {}",
            self.label, self.status
        )
    }
}

impl RunHandle {
    /// Waits for the command to finish
    ///
    /// If the command ran with output aggregation (see [Runnable::run]), the output will be printed
    /// to stdout after the command has finished.
    ///
    /// Returns an error if the command exited with a non-zero status.
    pub fn wait(mut self) -> Result<()> {
        self.out_handle
            .join()
            .map_err(|err| anyhow::anyhow!("join stdout: panicked: {:?}", err))
            .context("join stdout thread")??;

        self.err_handle
            .join()
            .map_err(|err| anyhow::anyhow!("join stderr: panicked: {:?}", err))
            .context("join stderr thread")??;

        let status = self.child.wait().context("wait for child")?;

        // Will be empty if aggregate_output is false
        for line in self.output.lock().unwrap().iter() {
            println!("{}{}", self.label, line);
        }

        if !status.success() {
            let label = self.label.clone();
            let err = ExitStatusError { label, status };
            anyhow::bail!(err);
        }

        Ok(())
    }
}
