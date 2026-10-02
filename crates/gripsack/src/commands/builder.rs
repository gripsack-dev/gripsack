use gripsack_buildkit::worker::{
    OwnedWorker, WorkerFact, WorkerInspection, WorkerObservation, WorkerOptions, WorkerProfile,
};
use gripsack_process::{OperatorEnvironment, terminal::tame};
use std::{
    process::ExitCode,
    time::{Duration, Instant},
};

#[derive(Debug, clap::Subcommand)]
pub enum BuilderCommand {
    /// Observe the recorded owned worker without starting it.
    Status,
    /// Stop the owned worker; refuse active clients.
    Stop,
    /// Stop the owned worker and remove only its disposable cache.
    CacheClean,
}

pub fn builder(command: BuilderCommand) -> ExitCode {
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let environment = OperatorEnvironment::capture()?;
        let deadline = Instant::now() + Duration::from_secs(120);
        let home = gripsack_store::gripsack_home();
        let worker = OwnedWorker::open(
            &home,
            WorkerProfile::parse("production")?,
            &environment,
            WorkerOptions::default(),
            deadline,
        )?;
        match command {
            BuilderCommand::Status => {
                let status = worker.status()?;
                println!(
                    "phase: {:?}\nrunning: {}\nlive clients: {}\nuncertain clients: {}",
                    status.phase, status.running, status.live_leases, status.uncertain_leases
                );
                if let Some(identity) = status.instance_id {
                    println!("instance: {}", tame(identity));
                }
                print_inspection(status.inspection);
            }
            BuilderCommand::Stop | BuilderCommand::CacheClean => {
                let quiescence = worker.stop()?;
                let session = gripsack_exec::LifecycleSession::acquire(&home)?;
                let recovered = gripsack_exec::recover_builder_roots(&session, &quiescence)?;
                if matches!(command, BuilderCommand::CacheClean) {
                    worker.cache_cleanup()?;
                    println!("owned builder cache removed; native artifacts retained");
                } else {
                    println!("owned builder stopped");
                }
                println!(
                    "abandoned builds: {} retired, {} live, {} outside the stop fence",
                    recovered.retired, recovered.live, recovered.outside_fence
                );
            }
        }
        Ok(())
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("grip: {}", tame(error.to_string()));
            ExitCode::FAILURE
        }
    }
}

fn print_inspection(details: WorkerInspection) {
    println!("configuration: {}", details.configuration_source);
    print_fact("worker image/binding", details.image);
    print_observation("immutable image ID", details.image_id);
    print_fact("VM image", details.vm_image);
    print_fact("helper version", details.helper_version);
    print_fact("daemon version", details.daemon_version);
    print_fact("Linux target", details.target);
    print_fact("CPU limit", details.cpus);
    print_fact("memory limit (bytes)", details.memory_bytes);
    println!("cache scope: {}", tame(details.cache_scope));
    print_observation("cache present", details.cache_present);
    if details.disk.is_empty() {
        println!("disk use: absent (no owned instance or cache observed)");
    }
    for disk in details.disk {
        println!("disk scope: {} ({})", tame(disk.scope), disk.accounting);
        print_observation("  disk use (bytes)", disk.bytes);
    }
}

fn print_fact<T: std::fmt::Display>(name: &str, fact: WorkerFact<T>) {
    match fact.configured {
        Some(value) => println!("{name} configured: {}", tame(value.to_string())),
        None => println!("{name} configured: unspecified / not applicable"),
    }
    print_observation(format_args!("{name} observed"), fact.observed);
}

fn print_observation<T: std::fmt::Display>(
    name: impl std::fmt::Display,
    observation: WorkerObservation<T>,
) {
    match observation {
        WorkerObservation::Absent => println!("{name}: absent"),
        WorkerObservation::Unavailable(reason) => {
            println!("{name}: unavailable ({})", tame(reason))
        }
        WorkerObservation::Observed { value, source } => {
            println!("{name}: {} [{source}]", tame(value.to_string()))
        }
    }
}
