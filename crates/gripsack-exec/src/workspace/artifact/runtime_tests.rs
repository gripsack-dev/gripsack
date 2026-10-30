use super::*;
use gripsack_ir::{
    workspace::{PlatformAbi, PlatformArch, PlatformOs},
    workspace_model::HostLibraryDirectory,
};

#[derive(Clone, Copy)]
enum Evidence {
    Independent,
    HostPolicy,
    PortableRoot,
}

fn package(identity: u8, runtime: Vec<Arc<Package>>, evidence: Evidence) -> Arc<Package> {
    let producer_root = PathBuf::from(format!("/fixture/store/{identity:064x}-workspace-source"));
    let mut retention = BTreeSet::from([producer_root.clone()]);
    if matches!(evidence, Evidence::PortableRoot) {
        retention.insert(PathBuf::from(format!(
            "/fixture/store/{identity:064x}-package-import"
        )));
    }
    Arc::new(Package {
        identity: PackageDigest::from_bytes([identity; 32]),
        root: PathBuf::from(format!("/fixture/store/{identity:064x}-workspace-package")),
        producer: Arc::new(Artifact {
            root: producer_root.clone(),
            payload: producer_root,
            tree: serde_json::from_value(serde_json::json!("0".repeat(64))).unwrap(),
            retention,
        }),
        archive_root: None,
        selected_root: None,
        selected_payload: None,
        commands: BTreeMap::new(),
        runtime,
        target: WorkspacePlatform {
            os: PlatformOs::Linux,
            arch: PlatformArch::X86_64,
            abi: Some(PlatformAbi::Gnu),
            minimum_os: None,
        },
        layout: CatalogPackageLayout::Relocatable,
        host_runtime: matches!(evidence, Evidence::HostPolicy).then(|| HostRuntimeRequirements {
            library_directories: vec![
                HostLibraryDirectory::new("/declared/runtime/lib".into()).unwrap(),
            ],
        }),
        conda: None,
    })
}

/// Two distinct views at every level share the preceding level. Each package
/// has a unique identity and no duplicate direct edge, as in real publication.
fn repeated_dag(leaf: Evidence) -> (Arc<Package>, usize) {
    let mut previous = vec![package(0, Vec::new(), leaf)];
    let mut identity = 1;
    for _ in 0..26 {
        let left = package(identity, previous.clone(), Evidence::Independent);
        identity += 1;
        let right = package(identity, previous, Evidence::Independent);
        identity += 1;
        previous = vec![left, right];
    }
    (
        package(identity, previous, Evidence::Independent),
        usize::from(identity) + 1,
    )
}

#[test]
fn negative_runtime_queries_visit_shared_views_once_with_or_without_commands() {
    for with_command in [false, true] {
        let (mut root, unique_views) = repeated_dag(Evidence::Independent);
        if with_command {
            Arc::get_mut(&mut root).unwrap().commands.insert(
                "probe".into(),
                ProvidedCommand {
                    selector: "bin/probe".into(),
                    executable: ExecutableDigest::from_bytes([7; 32]),
                },
            );
        }
        let mut visits = 0;
        // Bound the regression itself: a missing visited guard returns true at
        // the first excess visit rather than expanding the exponential tree.
        let exceeded = root.any_in_runtime_closure(|_| {
            visits += 1;
            visits > unique_views
        });
        assert!(!exceeded);
        assert_eq!(visits, unique_views);
        assert!(!root.host_dependent());
        assert!(!root.portable());
    }
}

#[test]
fn transitive_runtime_authority_is_preserved_through_shared_descendants() {
    let (host, _) = repeated_dag(Evidence::HostPolicy);
    assert!(host.host_dependent());
    assert!(!host.portable());
    let (portable, _) = repeated_dag(Evidence::PortableRoot);
    assert!(portable.portable());
    assert!(!portable.host_dependent());
}

#[test]
fn distinct_policy_views_with_equal_identities_are_not_collapsed() {
    let plain = package(1, Vec::new(), Evidence::Independent);
    let host = package(1, Vec::new(), Evidence::HostPolicy);
    assert_eq!(plain.identity, host.identity);
    assert_eq!(plain.producer.root, host.producer.root);
    // The plain view is deliberately visited first by the LIFO traversal.
    let root = package(2, vec![host, Arc::clone(&plain)], Evidence::Independent);
    assert!(root.host_dependent());
    assert!(!root.portable());

    let portable = package(1, Vec::new(), Evidence::PortableRoot);
    assert_eq!(plain.identity, portable.identity);
    assert_eq!(plain.producer.root, portable.producer.root);
    let root = package(2, vec![portable, plain], Evidence::Independent);
    assert!(root.portable());
    assert!(!root.host_dependent());
}

#[test]
fn local_and_leaf_queries_need_only_the_local_observation() {
    let leaf = package(0, Vec::new(), Evidence::Independent);
    let mut visits = 0;
    assert!(!leaf.any_in_runtime_closure(|_| {
        visits += 1;
        false
    }));
    assert_eq!(visits, 1);
    let (root, _) = repeated_dag(Evidence::Independent);
    visits = 0;
    assert!(root.any_in_runtime_closure(|_| {
        visits += 1;
        true
    }));
    assert_eq!(visits, 1);
}
