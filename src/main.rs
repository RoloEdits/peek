// PID,NAME,CPU,MEM,VIRT_MEM

use anyhow::Context;
use clap::{Parser, ValueEnum};
use std::fs::File;
use std::io::Write;
use std::sync::mpsc::channel;
use std::{path::PathBuf, process::Command};
use sysinfo::{CpuExt, Pid, ProcessExt, System, SystemExt};
use uuid::Uuid;

// TODO: Need to figure out the message passing to control when to break out of loop and write data out.
// Need to handle when there is an error with the running program if spawned from the positional argument.
// NOTE: When the interput is sent, if the program was running from the positional argument, when it should be killed too.

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let (program_tx, program_rx) = channel();

    let mut peep_spawned = false;

    let pid = cli.pid.unwrap_or_else(|| {
        let mut process = Command::new(format!(
            "./{}",
            cli.program.expect("no program was passed in to run")
        ))
        .spawn()
        .expect("Failed to run program");

        peep_spawned = true;

        let pid = process.id() as usize;

        std::thread::spawn(move || {
            let status = process.wait().expect("program failed to run");
            program_tx
                .send(status)
                .expect("failed to send finsihed programs status back to peep");
        });

        pid
    });

    let (ctl_c_tx, ctrl_c_rx) = channel();

    ctrlc::set_handler(move || {
        ctl_c_tx
            .send(())
            .expect("failed to send interupt signal through channel")
    })
    .expect("error setting Ctrl-C handler");

    let mut data: Vec<Data> = Vec::with_capacity(1024);
    let uuid = Uuid::new_v4();
    let mut sample = 0;

    let mut sys = System::new_all();
    let threads = sys.cpus().len();
    // let cores = sys.physical_core_count().expect("failed to get core count");

    loop {
        if program_rx.try_recv().is_ok() {
            // program finished running
            break;
        }

        sys.refresh_all();

        let process = sys
            .process(Pid::from(pid))
            .with_context(|| "no such proces is running")?;

        // Was manually interupted
        if ctrl_c_rx.try_recv().is_ok() {
            // If was manually interupted and the process was started from peep
            // then kill the process
            if peep_spawned {
                process.kill();
            }
            break;
        }

        let name = process.name();
        let cpu = process.cpu_usage() / threads as f32;
        let mem = process.memory();
        let virt_mem = process.virtual_memory();
        let cpu_freq = sys
            .cpus()
            .iter()
            .fold(0, |freq, cpu| cpu.frequency() + freq)
            / threads as u64;
        let disk_read = process.disk_usage().total_read_bytes;
        let disk_write = process.disk_usage().total_written_bytes;

        data.push(Data {
            uuid,
            sample,
            pid,
            name: name.to_string(),
            cpu,
            cpu_freq,
            mem,
            virt_mem,
            disk_read,
            disk_write,
        });

        sample += 1;

        // 200ms
        std::thread::sleep(System::MINIMUM_CPU_UPDATE_INTERVAL);
    }

    match cli.format {
        Format::Csv => to_csv(data),
        Format::Json => to_json(cli.output, data),
    }

    Ok(())
}

fn to_csv(data: Vec<Data>) {
    todo!()
}

fn to_json(path: PathBuf, data: Vec<Data>) {
    let json = serde_json::json!(data);

    let mut cwd = std::env::current_dir().expect("couldn't get cwd");

    cwd.push(path);

    let mut file = File::create(cwd).expect("failed to get file handle");
    file.write_all(json.to_string().as_bytes())
        .expect("failed to write data to file");
}

#[derive(Debug, serde::Serialize)]
struct Data {
    uuid: Uuid,
    sample: u64,
    pid: usize,
    name: String,
    cpu: f32,
    cpu_freq: u64,
    mem: u64,
    virt_mem: u64,
    disk_read: u64,
    disk_write: u64,
}

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    program: Option<String>,
    #[arg(long, short)]
    pid: Option<usize>,
    #[arg(long, short)]
    output: PathBuf,
    #[arg(value_enum, long, short)]
    format: Format,
}

enum Output {
    File,
    Stdout,
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
enum Format {
    Csv,
    Json,
}
