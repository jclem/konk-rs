use anyhow::{Context, Result};
use std::{
    io::{BufRead, BufReader},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    thread::{spawn, JoinHandle},
};

#[derive(Debug)]
pub struct Runnable {
    pub command: String,
    pub label: String,
}

impl Runnable {
    pub fn run(&self) -> Result<RunHandle> {
        let child = Command::new("/bin/sh")
            .args(["-c", &self.command])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("process spawned");

        Ok(RunHandle {
            child,
            label: self.label.clone(),
        })
    }
}

#[derive(Debug)]
pub struct RunHandle {
    child: std::process::Child,
    label: String,
}

impl RunHandle {
    pub fn wait(mut self, aggregate_output: bool) -> Result<()> {
        let stdout = self.child.stdout.take().context("get child stdout")?;
        let stderr = self.child.stderr.take().context("get child stderr")?;

        let lines: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

        let out_lines = lines.clone();
        let label = self.label.clone();

        let out: JoinHandle<Result<()>> = spawn(move || {
            let reader = BufReader::new(stdout);

            for line in reader.lines() {
                let line = line.context("read line")?;

                if aggregate_output {
                    out_lines.lock().unwrap().push(line.clone());
                } else {
                    println!("{} {}", label, line);
                }
            }

            Ok(())
        });

        let err_lines = lines.clone();
        let label = self.label.clone();

        let err: JoinHandle<Result<()>> = spawn(move || {
            let reader = BufReader::new(stderr);

            for line in reader.lines() {
                let line = line.context("read line")?;

                if aggregate_output {
                    err_lines.lock().unwrap().push(line.clone());
                } else {
                    println!("{} {}", label, line);
                }
            }

            Ok(())
        });

        out.join()
            .map_err(|err| anyhow::anyhow!("join stdout: panicked: {:?}", err))
            .context("join stdout thread")??;

        err.join()
            .map_err(|err| anyhow::anyhow!("join stderr: panicked: {:?}", err))
            .context("join stderr thread")??;

        if aggregate_output {
            for line in lines.lock().unwrap().iter() {
                println!("{} {}", self.label, line);
            }
        }

        let status = self.child.wait().context("wait for child")?;
        if !status.success() {
            anyhow::bail!("{} command exited with status: {}", self.label, status);
        }

        Ok(())
    }
}
