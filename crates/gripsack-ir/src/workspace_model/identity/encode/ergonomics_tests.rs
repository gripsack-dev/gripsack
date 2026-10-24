use super::*;
use serde_json::json;

#[test]
fn absent_archive_policy_preserves_retained_package_bytes() {
    let mut package: PackageOutput = serde_json::from_value(json!({
        "name":"tool","span":{"file":"old.ts","line":1},
        "producer":{"kind":"recipe","recipe":"build"},
        "commands":{"tool":"bin/tool"},"runtime":[],
        "target":{"os":"linux","arch":"x86_64"},"layout":{"kind":"relocatable"}
    }))
    .unwrap();
    let mut retained = Encoder::new(b"retained-package");
    retained.platform(&package.target);
    retained.layout(&package.layout);
    retained.number(1);
    retained.text("tool");
    retained.text("bin/tool");
    retained.number(0); // Runtime closure; retained encoding ends here.
    let expected = retained.finish();
    let digest = |package: &PackageOutput| {
        let mut writer = Encoder::new(b"retained-package");
        super::super::package(
            &mut writer,
            package,
            &BTreeMap::<String, PackageDigest>::new(),
        )
        .unwrap();
        writer.finish()
    };
    assert_eq!(digest(&package), expected);
    package.archive_root = Some(ArchiveRootPolicy::SingleDirectory);
    assert_ne!(digest(&package), expected);
    package.archive_root = None;
    assert_eq!(digest(&package), expected);
}

#[test]
fn absent_launchers_preserve_retained_profile_declaration_bytes() {
    let mut profile: ProfileOutput = serde_json::from_value(json!({
        "name":"home","span":{"file":"old.ts","line":1},"environment":"tools"
    }))
    .unwrap();
    let mut retained = Encoder::new(b"retained-profile");
    retained.text("home");
    retained.text("profile");
    retained.field(b"some");
    retained.text("tools");
    retained.number(0); // Schedules.
    retained.number(0); // Hooks.
    retained.number(0); // Files; retained encoding ends here.
    let expected = retained.finish();
    let digest = |profile: ProfileOutput| {
        let mut writer = Encoder::new(b"retained-profile");
        output(&mut writer, &WorkspaceOutput::Profile(profile));
        writer.finish()
    };
    assert_eq!(digest(profile.clone()), expected);
    profile.launchers = Some(ProfileLaunchers {
        directory: "~/.local/bin".into(),
        commands: vec!["tool".into()],
    });
    assert_ne!(digest(profile.clone()), expected);
    profile.launchers = None;
    assert_eq!(digest(profile), expected);
}
