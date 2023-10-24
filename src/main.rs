// PID,NAME,CPU,MEM,VIRT_MEM

use anyhow::Context;
use anyhow::Result;
use clap::{Parser, ValueEnum};
use std::fmt::Display;
use std::fs::File;
use std::io::Write;
use std::process::ExitStatus;
use std::sync::mpsc::channel;
use std::sync::mpsc::Receiver;
use std::{path::PathBuf, process::Command};
use sysinfo::{ProcessExt, System, SystemExt};
use uuid::Uuid;

fn main() -> Result<()> {
    let cli = Cli::parse();

    let mut peek = Peek::new(cli)?;

    peek.run()?;

    peek.output()?;

    Ok(())
}

struct Peek {
    system: System,
    program: Program,
    output_path: PathBuf,
    format: Format,
    output: Output,
    samples: Vec<Samples>,
    crtl_c_interupt: Receiver<()>,
}

impl Peek {
    fn new(cli: Cli) -> Result<Self> {
        let (tx, rx) = channel();
        ctrlc::set_handler(move || tx.send(()).unwrap())?;

        let program = Program::new(&cli)?;

        Ok(Self {
            system: System::new(),
            program,
            output_path: cli.path.unwrap_or_else(|| {
                let mut output = std::env::current_dir().expect("couldn't get cwd");
                output.push(format!("peek.{}", cli.format));
                output
            }),
            crtl_c_interupt: rx,
            format: cli.format,
            samples: Vec::with_capacity(1024),
            output: cli.output,
        })
    }

    fn run(&mut self) -> Result<()> {
        let uuid = Uuid::new_v4();

        let program = self.program.run()?;

        self.system.refresh_processes();
        self.system.refresh_cpu();
        let threads = self.system.cpus().len();

        let mut sample = 0;

        loop {
            if program.finished_running.try_recv().is_ok()
                || self.crtl_c_interupt.try_recv().is_ok()
            {
                break;
            }

            self.system.refresh_processes();

            let process = self
                .system
                .process(sysinfo::Pid::from(program.pid))
                .with_context(|| "no such process is running")?;

            self.samples.push(Samples {
                uuid,
                sample,
                pid: program.pid,
                name: process.name().to_string(),
                cpu: process.cpu_usage() / threads as f32,
                mem: process.memory(),
                virt_mem: process.virtual_memory(),
                disk_read: process.disk_usage().total_read_bytes,
                disk_write: process.disk_usage().total_written_bytes,
            });

            sample += 1;

            // 200ms
            std::thread::sleep(System::MINIMUM_CPU_UPDATE_INTERVAL);
        }
        Ok(())
    }

    fn output(self) -> Result<()> {
        fn to_json(data: Vec<Samples>) -> String {
            serde_json::json!(data).to_string()
        }

        fn to_csv(_data: Vec<Samples>) -> String {
            todo!()
        }

        let data = match self.format {
            Format::Csv => to_csv(self.samples),
            Format::Json => to_json(self.samples),
        };

        match self.output {
            Output::File => {
                let mut file = File::create(self.output_path)?;

                file.write_all(data.as_bytes())?;
            }
            Output::Stdout => {
                println!("{data}");
            }
        }

        Ok(())
    }
}

#[derive(Debug)]
struct Program {
    command: String,
    args: Vec<String>,
}

impl Program {
    pub fn new(cli: &Cli) -> Result<Self> {
        let command: Vec<String> = cli
            .program
            .split_whitespace()
            .map(|param| param.to_owned())
            .collect();

        Ok(Self {
            command: command[0].clone(),
            args: command[1..].to_vec(),
        })
    }

    pub fn run(&self) -> Result<RunningProgram> {
        let (status_tx, status_rx) = channel();
        let (pid_tx, pid_rx) = channel();

        let command = self.command.clone();
        let args = self.args.clone();

        std::thread::spawn(move || {
            let command = command;
            let args = args;

            if let Ok(mut child) = Command::new(&command).args(&args).spawn() {
                pid_tx.send(child.id()).unwrap();

                status_tx
                    .send(child.wait().unwrap())
                    .expect("failed to send finsihed programs status back to peep");
            } else {
                let cwd = std::env::current_dir().unwrap();
                let command = cwd.join(&command);

                let mut child = Command::new(command).args(&args).spawn().unwrap();

                pid_tx.send(child.id()).unwrap();

                status_tx
                    .send(child.wait().unwrap())
                    .expect("failed to send finsihed programs status back to peep");
            }
        });

        let pid = pid_rx.recv().unwrap() as usize;

        Ok(RunningProgram {
            pid,
            finished_running: status_rx,
        })
    }
}

struct RunningProgram {
    pid: usize,
    finished_running: Receiver<ExitStatus>,
}

// TODO: Time the samples and store in field.
#[derive(Debug, serde::Serialize)]
struct Samples {
    uuid: Uuid,
    sample: u64,
    pid: usize,
    name: String,
    cpu: f32,
    mem: u64,
    virt_mem: u64,
    disk_read: u64,
    disk_write: u64,
}

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    program: String,
    // #[arg(long, short)]
    // pid: Option<usize>,
    path: Option<PathBuf>,
    #[arg(long, short, default_value = "stdout")]
    output: Output,
    #[arg(value_enum, long, short, default_value = "json")]
    format: Format,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
enum Output {
    File,
    Stdout,
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
enum Format {
    Csv,
    Json,
}

impl Display for Format {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let str = match self {
            Self::Csv => "csv",
            Self::Json => "json",
        };

        write!(f, "{}", str)
    }
}
