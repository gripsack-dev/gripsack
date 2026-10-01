use gripsack_ir::{
    check,
    workspace_v6::{
        WorkspaceArg, WorkspaceCommand, WorkspaceOutput, WorkspaceSourceV6, WorkspaceStep,
        identity::{self, ArtifactDigest, CommandPins, DefinitionDigest, PinGap, RecipePins},
        lock::{DefinitionPins, ResolvedPinFields},
    },
};
use std::collections::{BTreeMap, BTreeSet};

struct OwnedPins {
    definitions: DefinitionPins,
    source: ResolvedPinFields,
    commands: CommandPins,
    checks: BTreeMap<String, identity::CheckDigest>,
    transitive_checks: BTreeSet<identity::CheckDigest>,
}
impl OwnedPins {
    fn view(&self) -> RecipePins<'_> {
        RecipePins {
            definitions: &self.definitions,
            source: &self.source,
            commands: &self.commands,
            checks: &self.checks,
            transitive_checks: &self.transitive_checks,
        }
    }
}

fn definitions() -> DefinitionPins {
    DefinitionPins {
        frontend: DefinitionDigest::parse(&"f".repeat(64)).unwrap(),
        imports: BTreeMap::new(),
    }
}
fn document() -> serde_json::Value {
    serde_json::json!({"ir_version":6,"host":{"os":"linux","arch":"x86_64"},"workspace":{
    "span":{"file":"gripsack.ts","line":1},"outputs":[
        {"kind":"recipe","name":"build","span":{"file":"gripsack.ts","line":2},
         "source":{"kind":"fetch","fetch":{"kind":"file","path":"src"},"span":{"file":"gripsack.ts","line":2}},
         "execution":{"kind":"host","access":"unconfined"},"output_kind":"tree",
         "target":{"os":"linux","arch":"x86_64"},"checks":["verify"],
         "steps":[{"command":{"kind":"exec","span":{"file":"gripsack.ts","line":3},"argv":[{"kind":"literal","value":"cc"}]}}]},
        {"kind":"check","name":"verify","span":{"file":"gripsack.ts","line":5},"subject":"build",
         "run":{"kind":"exec","span":{"file":"gripsack.ts","line":5},"argv":[{"kind":"literal","value":"/bin/true"}]}}
    ]}})
}
#[test]
fn recipe_identity_binds_required_check_policy_but_not_diagnostic_locations() {
    let ir = check(&document().to_string()).unwrap();
    let workspace = ir.workspace_v6.unwrap();
    let WorkspaceOutput::Recipe(recipe) = &workspace.outputs[0] else {
        panic!("recipe fixture");
    };
    let WorkspaceOutput::Check(required) = &workspace.outputs[1] else {
        panic!("check fixture");
    };
    let mut pins = OwnedPins {
        definitions: definitions(),
        source: ResolvedPinFields {
            tree256: Some("a".repeat(64)),
            ..Default::default()
        },
        commands: CommandPins::default(),
        checks: BTreeMap::new(),
        transitive_checks: BTreeSet::new(),
    };
    let production = identity::production_digest(recipe, &pins.view()).unwrap();
    let subject = CommandPins {
        artifacts: BTreeMap::from([("build".into(), ArtifactDigest::recipe_output(production))]),
        ..Default::default()
    };
    pins.checks.insert(
        "verify".into(),
        identity::check_digest(required, &subject, &recipe.execution).unwrap(),
    );
    let original = identity::recipe_digest(recipe, &pins.view()).unwrap();
    let mut moved = recipe.clone();
    moved.name = "different-diagnostic-name".into();
    moved.span.file = "factory.ts".into();
    moved.span.line = 100;
    if let WorkspaceSourceV6::Fetch(fetch) = &mut moved.source {
        fetch.span.file = "factory.ts".into();
    }
    if let WorkspaceStep::Command(WorkspaceCommand::Exec { span, .. }) = &mut moved.steps[0] {
        span.line = 101;
    }
    assert_eq!(
        identity::recipe_digest(&moved, &pins.view()).unwrap(),
        original
    );
    let mut changed = required.clone();
    if let WorkspaceCommand::Exec { argv, .. } = &mut changed.run {
        argv[0] = WorkspaceArg::Literal {
            value: "/bin/false".into(),
        };
    }
    pins.checks.insert(
        "verify".into(),
        identity::check_digest(&changed, &subject, &recipe.execution).unwrap(),
    );
    assert_ne!(
        identity::recipe_digest(recipe, &pins.view()).unwrap(),
        original,
        "required check policy was omitted from identity"
    );
    assert_eq!(
        identity::production_digest(recipe, &pins.view()).unwrap(),
        production,
        "validation policy was confused with production bytes"
    );
}
#[test]
fn unresolved_inputs_and_checks_cannot_acquire_recipe_identity() {
    let ir = check(&document().to_string()).unwrap();
    let WorkspaceOutput::Recipe(mut recipe) = ir.workspace_v6.unwrap().outputs.remove(0) else {
        panic!("recipe fixture");
    };
    let pins = OwnedPins {
        definitions: definitions(),
        source: ResolvedPinFields {
            tree256: Some("a".repeat(64)),
            ..Default::default()
        },
        commands: CommandPins::default(),
        checks: BTreeMap::new(),
        transitive_checks: BTreeSet::new(),
    };
    assert!(matches!(
        identity::recipe_digest(&recipe, &pins.view()),
        Err(PinGap::Check(_))
    ));
    if let WorkspaceStep::Command(WorkspaceCommand::Exec { argv, .. }) = &mut recipe.steps[0] {
        argv.push(WorkspaceArg::Input {
            input: "configuration".into(),
        });
    }
    assert!(matches!(
        identity::recipe_digest(&recipe, &pins.view()),
        Err(PinGap::Input(_))
    ));
}
#[test]
fn unknown_authority_fields_remain_rejected_by_each_versioned_reader() {
    let mut input = document();
    input["workspace"]["outputs"][0]["steps"][0]["command"]["extra_authority"] = true.into();
    assert!(
        check(&input.to_string())
            .unwrap_err()
            .iter()
            .any(|diagnostic| diagnostic.code == gripsack_ir::codes::MALFORMED)
    );
    let historical = serde_json::json!({"ir_version":5,"host":{"os":"linux","arch":"x86_64"},"workspace":{
        "span":{"file":"old.ts","line":1},"outputs":[{"kind":"recipe","name":"old","span":{"file":"old.ts","line":2},
        "source":{"kind":"fetch","fetch":{"kind":"file","path":"src"},"span":{"file":"old.ts","line":2}},
        "execution":{"kind":"isolated_linux","worker":"buildkit","platform":{"os":"linux","arch":"x86_64"}},
        "output_kind":"tree","target":{"os":"linux","arch":"x86_64"}}]}});
    assert!(
        check(&historical.to_string())
            .unwrap_err()
            .iter()
            .any(|diagnostic| diagnostic.code == gripsack_ir::codes::MALFORMED)
    );
}

#[test]
fn ordered_commands_and_resolved_definition_pins_invalidate_production() {
    let ir = check(&document().to_string()).unwrap();
    let workspace = ir.workspace_v6.unwrap();
    let WorkspaceOutput::Recipe(mut recipe) = workspace.outputs[0].clone() else {
        panic!("recipe fixture");
    };
    recipe.checks.clear();
    let mut second = recipe.steps[0].clone();
    if let WorkspaceStep::Command(WorkspaceCommand::Exec { argv, .. }) = &mut second {
        argv.push(WorkspaceArg::Literal {
            value: "--second-stage".into(),
        });
    }
    recipe.steps.push(second);
    let mut pins = OwnedPins {
        definitions: definitions(),
        source: ResolvedPinFields {
            tree256: Some("a".repeat(64)),
            ..Default::default()
        },
        commands: CommandPins::default(),
        checks: BTreeMap::new(),
        transitive_checks: BTreeSet::new(),
    };
    let original = identity::recipe_digest(&recipe, &pins.view()).unwrap();
    recipe.steps.swap(0, 1);
    assert_ne!(
        identity::recipe_digest(&recipe, &pins.view()).unwrap(),
        original
    );
    recipe.steps.swap(0, 1);
    pins.definitions.frontend = DefinitionDigest::parse(&"e".repeat(64)).unwrap();
    assert_ne!(
        identity::recipe_digest(&recipe, &pins.view()).unwrap(),
        original
    );
    pins.definitions = definitions();
    pins.definitions.imports.insert(
        "recipe-library@1".into(),
        DefinitionDigest::parse(&"d".repeat(64)).unwrap(),
    );
    let imported = identity::recipe_digest(&recipe, &pins.view()).unwrap();
    assert_ne!(imported, original);
    pins.definitions.imports.insert(
        "recipe-library@1".into(),
        DefinitionDigest::parse(&"c".repeat(64)).unwrap(),
    );
    assert_ne!(
        identity::recipe_digest(&recipe, &pins.view()).unwrap(),
        imported
    );
}

#[test]
fn inherited_check_execution_and_transitive_policy_changes_invalidate_reuse() {
    let ir = check(&document().to_string()).unwrap();
    let workspace = ir.workspace_v6.unwrap();
    let WorkspaceOutput::Recipe(recipe) = &workspace.outputs[0] else {
        unreachable!();
    };
    let WorkspaceOutput::Check(check) = &workspace.outputs[1] else {
        unreachable!();
    };
    let mut pins = OwnedPins {
        definitions: definitions(),
        source: ResolvedPinFields {
            tree256: Some("a".repeat(64)),
            ..Default::default()
        },
        commands: CommandPins::default(),
        checks: BTreeMap::new(),
        transitive_checks: BTreeSet::new(),
    };
    let production = identity::production_digest(recipe, &pins.view()).unwrap();
    let commands = CommandPins {
        artifacts: BTreeMap::from([("build".into(), ArtifactDigest::recipe_output(production))]),
        ..Default::default()
    };
    let host = identity::check_digest(check, &commands, &recipe.execution).unwrap();
    let mut isolated: gripsack_ir::workspace_v6::RecipeExecution = serde_json::from_value(serde_json::json!({
        "kind":"isolated_linux","worker":"buildkit","platform":{"os":"linux","arch":"x86_64"},
        "toolchain":{"reference":format!("docker.io/library/toolchain@sha256:{}", "b".repeat(64))}
    })).unwrap();
    let linux = identity::check_digest(check, &commands, &isolated).unwrap();
    assert_ne!(
        host, linux,
        "one check was reused across execution authorities"
    );
    if let gripsack_ir::workspace_v6::RecipeExecution::IsolatedLinux { toolchain, .. } =
        &mut isolated
    {
        toolchain.reference = format!("docker.io/library/toolchain@sha256:{}", "c".repeat(64));
    }
    assert_ne!(
        identity::check_digest(check, &commands, &isolated).unwrap(),
        linux
    );
    pins.checks.insert("verify".into(), host);
    let original = identity::recipe_digest(recipe, &pins.view()).unwrap();
    pins.transitive_checks.insert(linux);
    assert_ne!(
        identity::recipe_digest(recipe, &pins.view()).unwrap(),
        original
    );
    assert_eq!(
        identity::production_digest(recipe, &pins.view()).unwrap(),
        production
    );
}
