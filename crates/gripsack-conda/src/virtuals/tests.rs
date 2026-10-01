use super::*;
use serde_json::json;

fn package(depends: &[&str],constrains: &[&str]) -> LockedCondaPackage {
    serde_json::from_value(json!({
        "name":"python","version":"3.12.7","build":"fixture_0","build_number":0,
        "subdir":"linux-64","channel":"https://conda.anaconda.org/conda-forge",
        "url":"https://conda.anaconda.org/conda-forge/linux-64/python-3.12.7-fixture_0.conda",
        "sha256":"0".repeat(64),"depends":depends,"constrains":constrains,
    })).unwrap()
}
fn fact(name: &str,version: &str,build: &str) -> LockedVirtualPackage {
    LockedVirtualPackage { name:name.into(),version:version.into(),build:build.into() }
}

#[test]
fn glibc_constraints_keep_both_floor_and_ceiling() {
    let packages=[package(&["__glibc >=2.17,<3.0.a0"],&[])];
    assert!(evaluate_virtual_constraints(&packages,&[fact("__glibc","2.36","0")]).is_ok());
    assert!(evaluate_virtual_constraints(&packages,&[fact("__glibc","2.16","0")]).is_err());
    assert!(evaluate_virtual_constraints(&packages,&[fact("__glibc","3.0","0")]).is_err());
    let packages=[package(&["__glibc <2.32"],&[])];
    assert!(evaluate_virtual_constraints(&packages,&[fact("__glibc","2.31","0")]).is_ok());
    assert!(evaluate_virtual_constraints(&packages,&[fact("__glibc","2.36","0")]).is_err());
}
#[test]
fn channel_qualified_virtual_dependencies_cannot_bypass_admission() {
    let packages=[package(&["conda-forge::__glibc <2"],&[])];
    assert!(evaluate_virtual_constraints(&packages,&[fact("__glibc","2.36","0")]).is_err());
}
#[test]
fn canonical_bare_version_and_build_specs_are_admitted() {
    let packages=[package(&["__glibc 2.36","__archspec 1 x86_64_v3"],&[])];
    assert!(evaluate_virtual_constraints(&packages,&[fact("__glibc","2.36","0"),fact("__archspec","1","x86_64_v3")]).is_ok());
    assert!(evaluate_virtual_constraints(&packages,&[fact("__glibc","2.36","0"),fact("__archspec","1","aarch64")]).is_err());
}
#[test]
fn optional_constraints_do_not_invent_required_virtual_packages() {
    let packages=[package(&[],&["__cuda >=12"])];
    assert!(evaluate_virtual_constraints(&packages,&[]).is_ok());
    assert!(evaluate_virtual_constraints(&packages,&[fact("__cuda","11.8","0")]).is_err());
    let packages=[package(&["__cuda >=12"],&[])];
    assert!(matches!(evaluate_virtual_constraints(&packages,&[]),Err(VirtualConstraintError::Missing { .. })));
}
#[test]
fn invalid_and_duplicate_measured_facts_refuse_admission() {
    let packages=[package(&["__glibc >=2.17"],&[])];
    assert!(evaluate_virtual_constraints(&packages,&[fact("__glibc","!","0")]).is_err());
    assert!(evaluate_virtual_constraints(&packages,&[fact("__glibc","2.36","0"),fact("__glibc","2.40","0")]).is_err());
    assert!(evaluate_virtual_constraints(&[package(&["__glibc >="],&[])],&[fact("__glibc","2.36","0")]).is_err());
}

#[test]
fn conditional_virtual_requirements_follow_the_frozen_package_selection() {
    let mut selected=package(&[r#"__cuda >=12[when="python >=3.13"]"#],&[]);
    assert!(evaluate_virtual_constraints(&[selected.clone()],&[]).is_ok());
    selected.version="3.13.0".into();
    selected.url=selected.url.replace("3.12.7","3.13.0");
    assert!(matches!(evaluate_virtual_constraints(&[selected],&[]),Err(VirtualConstraintError::Missing { .. })));
}
