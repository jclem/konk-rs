use std::{
    env,
    io::BufRead,
    process::{Command, Stdio},
    thread::JoinHandle,
};

fn main() -> std::io::Result<()> {
    let mut args = env::args();

    match args.nth(1).as_deref() {
        Some("run") => handle_run(args),
        Some(command) => {
            eprintln!("unknown command: {}", command);
            std::process::exit(1);
        }
        None => {
            eprintln!("no args");
            std::process::exit(1);
        }
    }
}

fn handle_run(mut args: env::Args) -> std::io::Result<()> {
    // Command can be "s" or "serial"
    match args.nth(0).as_deref() {
        Some("s") | Some("serial") => handle_serial(args),
        Some("c") | Some("concurrent") => handle_concurrent(args),
        Some(command) => {
            eprintln!("unknown command: {}", command);
            std::process::exit(1);
        }
        _ => {
            eprintln!("no command");
            std::process::exit(1);
        }
    }
}

fn handle_serial(args: env::Args) -> std::io::Result<()> {
    // iterate with index
    for (i, command) in args.enumerate() {
        let mut child = Command::new("/bin/sh")
            .args(["-c", &command])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("process spawned");

        let stdout = child.stdout.take().expect("child should have stdout");
        let stderr = child.stderr.take().expect("child should have stderr");

        let stdout_thread = std::thread::spawn(move || {
            let reader = std::io::BufReader::new(stdout);

            for line in reader.lines() {
                let line = line.unwrap();
                println!("{}: {}", i, line);
            }
        });

        let stderr_thread = std::thread::spawn(move || {
            let reader = std::io::BufReader::new(stderr);

            for line in reader.lines() {
                let line = line.unwrap();
                println!("{}: {}", i, line);
            }
        });

        stdout_thread.join().expect("thread should finish");
        stderr_thread.join().expect("thread should finish");

        let status = child.wait()?;
        if !status.success() {
            eprintln!("command exited with status: {}", status);
            std::process::exit(1);
        }
    }

    Ok(())
}

fn handle_concurrent(args: env::Args) -> std::io::Result<()> {
    // Create a vector to store the threads
    let mut threads: Vec<JoinHandle<()>> = vec![];

    // iterate with index
    for (i, command) in args.enumerate() {
        let child_thread = std::thread::spawn(move || {
            let mut child = Command::new("/bin/sh")
                .args(["-c", &command])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("process spawned");

            let stdout = child.stdout.take().expect("child should have stdout");
            let stderr = child.stderr.take().expect("child should have stderr");

            let stdout_thread = std::thread::spawn(move || {
                let reader = std::io::BufReader::new(stdout);

                for line in reader.lines() {
                    let line = line.unwrap();
                    println!("{}: {}", i, line);
                }
            });

            let stderr_thread = std::thread::spawn(move || {
                let reader = std::io::BufReader::new(stderr);

                for line in reader.lines() {
                    let line = line.unwrap();
                    println!("{}: {}", i, line);
                }
            });

            stdout_thread.join().expect("thread should finish");
            stderr_thread.join().expect("thread should finish");

            let status = child.wait();
            match status {
                Ok(status) => {
                    if !status.success() {
                        eprintln!("command exited with status: {}", status);
                        std::process::exit(1);
                    }
                }
                Err(e) => {
                    eprintln!("error: {}", e);
                    std::process::exit(1);
                }
            }
        });

        threads.push(child_thread);
    }

    // Await all threads
    for thread in threads {
        thread.join().expect("thread should finish");
    }

    Ok(())
}
