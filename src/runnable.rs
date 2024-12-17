use anyhow::{Context, Result};
use std::{
    fmt,
    io::{BufRead, BufReader},
    process::{Command, ExitStatus, Stdio},
    sync::{Arc, Mutex},
    thread::{spawn, JoinHandle},
};

#[derive(Debug)]
pub struct Runnable {
    pub command: String,
    pub label: String,
}

impl Runnable {
    pub fn run(&self, aggregate_output: bool) -> Result<RunHandle> {
        let mut child = Command::new("/bin/sh")
            .args(["-c", &self.command])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("process spawned");

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
                    println!("{} {}", out_label, line);
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
                    println!("{} {}", err_label, line);
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

#[derive(Debug)]
pub struct RunHandle {
    child: std::process::Child,
    label: String,
    out_handle: JoinHandle<Result<()>>,
    err_handle: JoinHandle<Result<()>>,
    output: Arc<Mutex<Vec<String>>>,
}

#[derive(Debug)]
struct ExitStatusError {
    label: String,
    status: ExitStatus,
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
    pub fn wait(mut self) -> Result<()> {
        self.out_handle
            .join()
            .map_err(|err| anyhow::anyhow!("join stdout: panicked: {:?}", err))
            .context("join stdout thread")??;

        self.err_handle
            .join()
            .map_err(|err| anyhow::anyhow!("join stderr: panicked: {:?}", err))
            .context("join stderr thread")??;

        // Will be empty if aggregate_output is false
        for line in self.output.lock().unwrap().iter() {
            println!("{} {}", self.label, line);
        }

        let status = self.child.wait().context("wait for child")?;

        if !status.success() {
            let label = self.label.clone();
            let err = ExitStatusError { label, status };
            anyhow::bail!(err);
        }

        Ok(())
    }
}
