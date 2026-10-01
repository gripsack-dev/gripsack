use gripsack_buildkit::worker::{OwnedWorker, WorkerOptions, WorkerProfile};
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
