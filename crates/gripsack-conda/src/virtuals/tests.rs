use super::*;
use serde_json::json;

fn package(depends: &[&str], constrains: &[&str]) -> LockedCondaPackage {
    serde_json::from_value(json!({
        "name":"python","version":"3.12.7","build":"fixture_0","build_number":0,
        "subdir":"linux-64","channel":"https://conda.anaconda.org/conda-forge",
        "url":"https://conda.anaconda.org/conda-forge/linux-64/python-3.12.7-fixture_0.conda",
        "sha256":"0".repeat(64),"depends":depends,"constrains":constrains,
    }))
    .unwrap()
}
fn fact(name: &str, version: &str, build: &str) -> LockedVirtualPackage {
    LockedVirtualPackage {
        name: name.into(),
        version: version.into(),
        build: build.into(),
    }
}

#[test]
fn glibc_constraints_keep_both_floor_and_ceiling() {
    let packages = [package(&["__glibc >=2.17,<3.0.a0"], &[])];
    assert!(evaluate_requirements(&packages, &[fact("__glibc", "2.36", "0")]).is_ok());
    assert!(evaluate_requirements(&packages, &[fact("__glibc", "2.16", "0")]).is_err());
    assert!(evaluate_requirements(&packages, &[fact("__glibc", "3.0", "0")]).is_err());
    let packages = [package(&["__glibc <2.32"], &[])];
    assert!(evaluate_requirements(&packages, &[fact("__glibc", "2.31", "0")]).is_ok());
    assert!(evaluate_requirements(&packages, &[fact("__glibc", "2.36", "0")]).is_err());
}
#[test]
fn channel_qualified_virtual_dependencies_cannot_bypass_admission() {
    let packages = [package(&["conda-forge::__glibc <2"], &[])];
    assert!(evaluate_requirements(&packages, &[fact("__glibc", "2.36", "0")]).is_err());
}
#[test]
fn canonical_bare_version_and_build_specs_are_admitted() {
    let packages = [package(&["__glibc 2.36", "__archspec 1 x86_64_v3"], &[])];
    assert!(
        evaluate_requirements(
            &packages,
            &[
                fact("__glibc", "2.36", "0"),
                fact("__archspec", "1", "x86_64_v3")
            ]
        )
        .is_ok()
    );
    assert!(
        evaluate_requirements(
            &packages,
            &[
                fact("__glibc", "2.36", "0"),
                fact("__archspec", "1", "aarch64")
            ]
        )
        .is_err()
    );
}
#[test]
fn optional_constraints_do_not_invent_required_virtual_packages() {
    let packages = [package(&[], &["__cuda >=12"])];
    assert!(evaluate_requirements(&packages, &[]).is_ok());
    assert!(evaluate_requirements(&packages, &[fact("__cuda", "11.8", "0")]).is_err());
    let packages = [package(&["__cuda >=12"], &[])];
    assert!(matches!(
        evaluate_requirements(&packages, &[]),
        Err(VirtualConstraintError::Missing { .. })
    ));
}
#[test]
fn invalid_and_duplicate_measured_facts_refuse_admission() {
    let packages = [package(&["__glibc >=2.17"], &[])];
    assert!(evaluate_requirements(&packages, &[fact("__glibc", "!", "0")]).is_err());
    assert!(
        evaluate_requirements(
            &packages,
            &[fact("__glibc", "2.36", "0"), fact("__GLIBC", "2.40", "0")]
        )
        .is_err()
    );
    assert!(
        evaluate_requirements(
            &[package(&["__glibc >="], &[])],
            &[fact("__glibc", "2.36", "0")]
        )
        .is_err()
    );
}

#[test]
fn conditional_virtual_requirements_follow_the_frozen_package_selection() {
    let mut selected = package(&[r#"__cuda >=12[when="python >=3.13"]"#], &[]);
    assert!(evaluate_requirements(&[selected.clone()], &[]).is_ok());
    selected.version = "3.13.0".into();
    selected.url = selected.url.replace("3.12.7", "3.13.0");
    assert!(matches!(
        evaluate_requirements(&[selected], &[]),
        Err(VirtualConstraintError::Missing { .. })
    ));
}

#[test]
fn declared_system_floors_do_not_reuse_solve_assumptions() {
    use gripsack_ir::workspace_v6::lock::LockedVirtualPackageRequirement;
    let requirements = LockedCondaSystemRequirements {
        virtual_packages: vec![LockedVirtualPackageRequirement {
            name: "__glibc".into(),
            minimum_version: "2.28".into(),
            build: Some("0".into()),
        }],
        archspec: None,
    };
    assert!(evaluate_system_requirements(&requirements, &[fact("__glibc", "2.36", "0")]).is_ok());
    assert!(evaluate_system_requirements(&requirements, &[fact("__glibc", "2.27", "0")]).is_err());
    assert!(
        evaluate_system_requirements(&requirements, &[fact("__glibc", "2.36", "vendor")]).is_err()
    );
    assert!(evaluate_system_requirements(&requirements, &[]).is_err());
}

#[test]
fn declared_architecture_uses_canonical_capability_ancestry() {
    let requirements = LockedCondaSystemRequirements {
        virtual_packages: vec![],
        archspec: Some("x86_64_v3".into()),
    };
    assert!(
        evaluate_system_requirements(&requirements, &[fact("__archspec", "1", "x86_64_v4")])
            .is_ok()
    );
    assert!(
        evaluate_system_requirements(&requirements, &[fact("__archspec", "1", "x86_64_v2")])
            .is_err()
    );
    assert!(
        evaluate_system_requirements(&requirements, &[fact("__archspec", "1", "aarch64")]).is_err()
    );
    assert!(
        evaluate_system_requirements(&requirements, &[fact("__archspec", "1", "invented")])
            .is_err()
    );
    assert!(evaluate_system_requirements(&requirements, &[]).is_err());
}

#[test]
fn image_userspace_rejects_incompatible_libc_and_reports_external_requirements() {
    let packages = [package(
        &["__glibc >=2.28", "__linux >=5.10", "__cuda >=12"],
        &["__archspec 1 x86_64_v3"],
    )];
    let pending = evaluate_image_constraints(
        &packages,
        &[fact("__glibc", "2.39", "0"), fact("__unix", "0", "0")],
    )
    .unwrap();
    assert_eq!(
        pending,
        [
            "python: depends __linux >=5.10",
            "python: depends __cuda >=12",
            "python: constrains __archspec 1 x86_64_v3",
        ]
    );
    assert!(matches!(
        evaluate_image_constraints(&packages, &[fact("__glibc", "2.27", "0")]),
        Err(VirtualConstraintError::Unsatisfied { .. })
    ));
    assert!(matches!(
        evaluate_image_constraints(&packages, &[]),
        Err(VirtualConstraintError::Missing { .. })
    ));
    assert!(matches!(
        evaluate_image_constraints(&packages, &[fact("__cuda", "12", "0")]),
        Err(VirtualConstraintError::Fact { .. })
    ));
}

#[test]
fn image_conditionals_distinguish_unknown_runtime_from_known_false_selection() {
    let external = package(&[r#"__glibc >=99[when="__cuda >=12"]"#], &[]);
    let facts = [fact("__glibc", "2.39", "0")];
    assert_eq!(
        evaluate_image_constraints(&[external], &facts).unwrap(),
        [r#"python: depends __glibc >=99[when="__cuda >=12"]"#,]
    );
    let inactive = package(&[r#"__glibc >=99[when="python >=3.13"]"#], &[]);
    assert_eq!(
        evaluate_image_constraints(&[inactive], &facts).unwrap(),
        Vec::<String>::new()
    );
    let active = package(&[r#"__glibc >=99[when="python >=3.12"]"#], &[]);
    assert!(matches!(
        evaluate_image_constraints(&[active], &facts),
        Err(VirtualConstraintError::Unsatisfied { .. })
    ));
}

fn named(name: &str, depends: &[&str], constrains: &[&str]) -> LockedCondaPackage {
    let mut record = package(depends, constrains);
    record.name = name.into();
    record.url = record.url.replace("/python-", &format!("/{name}-"));
    record
}
fn environment(packages: Vec<LockedCondaPackage>) -> LockedCondaEnvironment {
    serde_json::from_value(json!({
        "platform":"linux-64","channels":["https://conda.anaconda.org/conda-forge"],
        "channel_priority":"strict","system_requirements":{},"virtual_packages":[],
        "packages":packages,"materializer":{"bytecode":"suppress","receipt":"normalized_conda_meta"}
    }))
    .unwrap()
}

#[test]
fn ordinary_transitive_requirements_match_version_build_and_channel() {
    let numpy = named("numpy", &["python >=3.12,<3.13 fixture_*"], &[]);
    assert!(matches!(
        evaluate_requirements(std::slice::from_ref(&numpy), &[]),
        Err(VirtualConstraintError::Missing { .. })
    ));
    let python = package(&[], &[]);
    assert!(evaluate_requirements(&[numpy.clone(), python.clone()], &[]).is_ok());
    for bad in [
        "python >=3.13",
        "python * wrong_*",
        "other-channel::python >=3.12",
        "python[subdir=osx-64]",
        "python[fn=wrong.conda]",
    ] {
        let mut numpy = numpy.clone();
        numpy.depends = vec![bad.into()];
        assert!(matches!(
            evaluate_requirements(&[numpy, python.clone()], &[]),
            Err(VirtualConstraintError::Unsatisfied { .. })
        ));
    }
}

#[test]
fn ordinary_constraints_are_optional_but_bind_selected_records() {
    let numpy = named("numpy", &[], &["python <3.12"]);
    assert!(evaluate_requirements(std::slice::from_ref(&numpy), &[]).is_ok());
    assert!(matches!(
        evaluate_requirements(&[numpy, package(&[], &[])], &[]),
        Err(VirtualConstraintError::Unsatisfied { .. })
    ));
}

#[test]
fn ordinary_conditional_closure_uses_actual_native_facts() {
    let numpy = named("numpy", &[r#"python >=3.12[when="__cuda >=12"]"#], &[]);
    assert!(evaluate_requirements(std::slice::from_ref(&numpy), &[]).is_ok());
    assert!(matches!(
        evaluate_requirements(std::slice::from_ref(&numpy), &[fact("__cuda", "12", "0")]),
        Err(VirtualConstraintError::Missing { .. })
    ));
    assert!(
        evaluate_requirements(&[numpy, package(&[], &[])], &[fact("__cuda", "12", "0")]).is_ok()
    );
}

#[test]
fn imported_missing_condition_facts_are_not_known_absence_or_image_facts() {
    let record = package(&[r#"numpy >=1[when="__cuda >=12"]"#], &[]);
    assert!(matches!(
        validate_frozen_environment(&environment(vec![record.clone()]), None),
        Err(VirtualConstraintError::Spec { .. })
    ));
    assert_eq!(
        evaluate_image_constraints(&[record], &[]).unwrap(),
        [r#"python: depends numpy >=1[when="__cuda >=12"]"#]
    );
    let ordinary_virtual = environment(vec![package(&["__glibc >=2.17"], &[])]);
    assert!(validate_frozen_environment(&ordinary_virtual, None).is_ok());
    assert!(evaluate_requirements(&ordinary_virtual.packages, &[]).is_err());
}

#[test]
fn linux_images_know_foreign_os_is_absent_even_when_optional() {
    for name in ["__osx", "__win"] {
        assert!(matches!(
            evaluate_image_constraints(&[package(&[name], &[])], &[]),
            Err(VirtualConstraintError::Missing { .. })
        ));
        assert_eq!(
            evaluate_image_constraints(&[package(&[], &[name])], &[]).unwrap(),
            Vec::<String>::new()
        );
        let conditional = format!(r#"missing[when="{name}"]"#);
        assert_eq!(
            evaluate_image_constraints(&[package(&[&conditional], &[])], &[]).unwrap(),
            Vec::<String>::new()
        );
    }
}

#[test]
fn canonical_duplicate_package_names_cannot_hide_records() {
    assert!(matches!(
        evaluate_requirements(&[package(&[], &[]), named("Python", &[], &[])], &[]),
        Err(VirtualConstraintError::Closure(_))
    ));
}

#[test]
fn declared_roots_exclude_unrequested_packages_and_bind_channel_policy() {
    let mut locked = environment(vec![
        named("numpy", &["python >=3.12"], &[]),
        package(&[], &[]),
    ]);
    let roots = vec!["numpy".into()];
    let channels = vec!["conda-forge".into()];
    assert!(validate_solved_environment(&locked, &roots, &channels, &[]).is_ok());
    for root in ["python", "numpy >=4", "other::numpy"] {
        assert!(validate_solved_environment(&locked, &[root.into()], &channels, &[]).is_err());
    }
    assert!(validate_solved_environment(&locked, &roots, &["other".into()], &[]).is_err());
    locked
        .channels
        .push("https://conda.anaconda.org/other".into());
    assert!(
        validate_solved_environment(
            &locked,
            &roots,
            &["other".into(), "conda-forge".into()],
            &[]
        )
        .is_err()
    );
    locked.channel_priority = ChannelPriority::Flexible;
    assert!(
        validate_solved_environment(
            &locked,
            &roots,
            &["conda-forge".into(), "other".into()],
            &[]
        )
        .is_err()
    );
}

#[test]
fn selected_extras_expand_closure_without_activating_unselected_extras() {
    let mut python = package(&[], &[]);
    python
        .extra_depends
        .insert("science".into(), vec!["numpy >=3".into()]);
    let locked = environment(vec![python, named("numpy", &[], &[])]);
    assert!(
        validate_frozen_environment(&locked, Some(&[r#"python[extras=["science"]]"#.into()]))
            .is_ok()
    );
    assert!(validate_frozen_environment(&locked, Some(&["python".into()])).is_err());
    assert!(
        validate_frozen_environment(
            &environment(vec![locked.packages[0].clone()]),
            Some(&[r#"python[extras=["science"]]"#.into()])
        )
        .is_err()
    );
}

#[test]
fn foreign_records_and_undeclared_artifact_channels_are_refused() {
    for (subdir, channel) in [
        ("osx-64", "https://conda.anaconda.org/conda-forge"),
        ("linux-64", "https://conda.anaconda.org/other"),
    ] {
        let mut record = package(&[], &[]);
        record.subdir = subdir.into();
        record.channel = channel.into();
        record.url = format!("{channel}/{subdir}/python-3.12.7-fixture_0.conda");
        assert!(matches!(
            validate_frozen_environment(&environment(vec![record]), None),
            Err(VirtualConstraintError::Closure(_))
        ));
    }
}
