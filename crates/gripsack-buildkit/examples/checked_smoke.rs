use gripsack_buildkit::{
    identity::{AttemptId, AttemptIdentity, FenceEpoch, SessionId},
    plan::{
        Architecture, BuildPlan, ExporterPlan, LinuxOs, Node, NodeIndex, Platform,
        ValidatedBuildPlan,
    },
    transport::{Bridge, ExportPaths},
    worker::{CleanupConfirmation, OwnedWorker, WorkerError, WorkerOptions, WorkerProfile},
};
use gripsack_process::{OperatorEnvironment, SelectedProgram};
use std::{
    io::{BufRead, Read, Write},
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<_> = std::env::args_os().collect();
    let environment = OperatorEnvironment::capture()?;
    let deadline = Instant::now() + Duration::from_secs(180);
    if arguments
        .get(1)
        .is_some_and(|argument| argument == "cleanup")
    {
        let worker = OwnedWorker::open(
            Path::new(&arguments[2]),
            WorkerProfile::parse("smoke")?,
            &environment,
            WorkerOptions::default(),
            deadline,
        )?;
        worker.stop()?;
        worker.cache_cleanup()?;
        return Ok(());
    }
    if arguments.get(1).is_some_and(|argument| argument == "hold") {
        let worker = OwnedWorker::open(
            Path::new(&arguments[2]),
            WorkerProfile::parse("smoke")?,
            &environment,
            WorkerOptions::default(),
            deadline,
        )?;
        let lease = worker.acquire()?;
        println!("READY {}", lease.instance_id());
        std::io::stdout().flush()?;
        let mut release = [0; 1];
        std::io::stdin().read_exact(&mut release)?;
        worker.release(lease, CleanupConfirmation::Confirmed)?;
        return Ok(());
    }
    let temporary = tempfile::tempdir()?.keep();
    let home = temporary.join("home");
    println!("SMOKE_HOME={}", home.display());
    std::fs::create_dir(&home)?;
    let inputs = temporary.join("inputs");
    let output = temporary.join("output");
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new().mode(0o700).create(&inputs)?;
    std::fs::DirBuilder::new().mode(0o700).create(&output)?;
    let selected = SelectedProgram::select(&environment, Path::new(&arguments[1]), None, deadline)?;
    let bridge = Bridge::new(&environment, &selected, &temporary, deadline);
    let plan = ValidatedBuildPlan::admit(BuildPlan {
        platform: Platform {
            os: LinuxOs::Linux,
            architecture: Architecture::Amd64,
        },
        nodes: vec![
            Node::Directory {
                input: None,
                path: "/payload".into(),
                mode: 0o755,
            },
            Node::File {
                input: Some(NodeIndex::new(0)?),
                path: "/payload/value".into(),
                data: b"checked through Rust and Go\n".to_vec(),
                mode: 0o444,
            },
        ],
        root: NodeIndex::new(1)?,
        exporter: ExporterPlan::Local,
    })?;
    let identity = AttemptIdentity {
        session: SessionId::new("checked-smoke")?,
        attempt: AttemptId::new(1)?,
        epoch: FenceEpoch::new(1)?,
    };
    let checked = bridge.lower(&plan, &identity, None)?;
    println!("CHECKED_DEFINITION={}", checked.digest());
    let worker = OwnedWorker::open(
        &home,
        WorkerProfile::parse("smoke")?,
        &environment,
        WorkerOptions::default(),
        deadline,
    )?;
    let lease = worker.acquire()?;
    let runtime = lease.address().socket_path().parent().unwrap().to_owned();
    let mut other = Command::new(std::env::current_exe()?)
        .arg("hold")
        .arg(&home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    let mut ready = String::new();
    std::io::BufReader::new(other.stdout.take().unwrap()).read_line(&mut ready)?;
    assert_eq!(ready.trim(), format!("READY {}", lease.instance_id()));
    assert!(matches!(worker.stop(), Err(WorkerError::LiveLeases(2))));
    let result = bridge.execute(
        checked,
        &identity,
        ExportPaths {
            worker: &lease,
            retention: None,
            inputs: &inputs,
            destination: &output,
        },
        |_nodes, vertex, bytes, _| {
            eprintln!("{vertex}: {:?}", String::from_utf8_lossy(bytes));
        },
    )?;
    assert_eq!(
        std::fs::read(result.destination().join("payload/value"))?,
        b"checked through Rust and Go\n"
    );
    worker.release(lease, CleanupConfirmation::Confirmed)?;
    let status = worker.status()?;
    assert!(status.running && status.live_leases == 1);
    other.stdin.take().unwrap().write_all(b"x")?;
    assert!(other.wait()?.success());
    let status = worker.status()?;
    assert!(!status.running && status.live_leases == 0);
    worker.cache_cleanup()?;
    assert!(
        !runtime.exists(),
        "owned runtime directory survived explicit cache cleanup"
    );
    worker.cache_cleanup()?;
    assert_eq!(
        std::fs::read(output.join("payload/value"))?,
        b"checked through Rust and Go\n"
    );
    println!("CHECKED_EXPORT_AND_TWO_REAL_CLIENTS=passed");
    std::fs::remove_dir_all(&temporary)?;
    Ok(())
}
