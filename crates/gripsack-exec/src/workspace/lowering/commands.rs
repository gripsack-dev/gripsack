use super::{Compiler, Mount, Node, NodeIndex, Origin, failure};
use crate::{ExecError, workspace::inputs::InputKind};
use gripsack_ir::{
    Span,
    workspace_v6::{
        PackageLayoutV6, WorkspaceArg, WorkspaceCommand, WorkspaceOutput, WorkspacePath,
    },
};
use std::collections::{BTreeMap, BTreeSet};

const SOURCE: &str = "/gripsack-source";
const OUTPUT: &str = "/gripsack-output";
const IMAGE_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

struct Bindings {
    mounts: BTreeMap<String, Mount>,
    packages: BTreeSet<String>,
    binaries: BTreeSet<String>,
    libraries: BTreeSet<String>,
}
impl Bindings {
    fn readonly(
        &mut self,
        path: String,
        node: NodeIndex,
        span: &Span,
    ) -> Result<String, ExecError> {
        match self.mounts.get(&path) {
            Some(existing) if existing.source != Some(node) || !existing.readonly => {
                return Err(failure(
                    span,
                    format!("different production bindings occupy {path:?}"),
                ));
            }
            Some(_) => {}
            None => {
                self.mounts.insert(
                    path.clone(),
                    Mount {
                        source: Some(node),
                        destination: path.clone(),
                        readonly: true,
                    },
                );
            }
        }
        Ok(path)
    }
}
impl<'p, 'ir> Compiler<'p, 'ir> {
    pub(super) fn command(
        &mut self,
        command: &'ir WorkspaceCommand,
        root: NodeIndex,
        source: NodeIndex,
        output: Option<NodeIndex>,
        prerequisites: &[&str],
    ) -> Result<NodeIndex, ExecError> {
        self.origin = Origin {
            output: self.origin.output,
            span: command.span(),
            line_map: match command {
                WorkspaceCommand::RunBash { line_map, .. } => line_map,
                _ => &[],
            },
        };
        let mut bindings = Bindings {
            mounts: BTreeMap::new(),
            packages: BTreeSet::new(),
            binaries: BTreeSet::new(),
            libraries: BTreeSet::new(),
        };
        bindings.readonly(SOURCE.into(), source, command.span())?;
        bindings.mounts.insert(
            OUTPUT.into(),
            Mount {
                source: output,
                destination: OUTPUT.into(),
                readonly: false,
            },
        );
        for prerequisite in prerequisites {
            self.artifact_binding(prerequisite, &mut bindings, command.span())?;
        }
        let (argv, declared_env, cwd) = match command {
            WorkspaceCommand::Exec { argv, env, cwd, .. } => {
                let argv = argv
                    .iter()
                    .map(|argument| self.argument(argument, &mut bindings, command.span()))
                    .collect::<Result<_, _>>()?;
                (argv, env, cwd)
            }
            WorkspaceCommand::RunBash {
                interpreter,
                options,
                body,
                env,
                cwd,
                ..
            } => {
                let mut argv = Vec::with_capacity(options.len() + 4);
                argv.push(self.argument(interpreter, &mut bindings, command.span())?);
                argv.extend(options.iter().cloned());
                argv.push("-c".into());
                argv.push(body.clone());
                argv.push("gripsack-bash".into());
                (argv, env, cwd)
            }
        };
        let cwd = match cwd {
            None => OUTPUT.into(),
            Some(WorkspacePath::Literal { value }) => {
                if value.starts_with('/') {
                    value.clone()
                } else {
                    select(OUTPUT, value)
                }
            }
            Some(WorkspacePath::Source { selector }) => select(SOURCE, selector),
            Some(WorkspacePath::Output { selector }) => select(OUTPUT, selector),
            Some(WorkspacePath::Artifact { output, selector }) => select(
                &self.artifact_binding(output, &mut bindings, command.span())?,
                selector,
            ),
            Some(WorkspacePath::Host { .. }) => {
                return Err(failure(
                    command.span(),
                    "a live host directory cannot enter a production command",
                ));
            }
        };
        let mut env = BTreeMap::new();
        for (key, value) in declared_env {
            env.insert(
                key.as_str(),
                self.argument(value, &mut bindings, command.span())?,
            );
        }
        let mut path = bindings.binaries.into_iter().collect::<Vec<_>>().join(":");
        if !path.is_empty() {
            path.push(':');
        }
        path.push_str(IMAGE_PATH);
        env.entry("PATH").or_insert(path);
        env.entry("HOME").or_insert_with(|| OUTPUT.into());
        env.entry("TMPDIR").or_insert_with(|| OUTPUT.into());
        if !bindings.libraries.is_empty() {
            env.entry("LD_LIBRARY_PATH")
                .or_insert_with(|| bindings.libraries.into_iter().collect::<Vec<_>>().join(":"));
        }
        self.push(Node::Process {
            root,
            argv,
            env: env
                .into_iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect(),
            cwd,
            mounts: bindings.mounts.into_values().collect(),
            output: OUTPUT.into(),
        })
    }
    fn argument(
        &mut self,
        argument: &WorkspaceArg,
        bindings: &mut Bindings,
        span: &Span,
    ) -> Result<String, ExecError> {
        Ok(match argument {
            WorkspaceArg::Literal { value } => value.clone(),
            WorkspaceArg::Source { selector } => select(SOURCE, selector),
            WorkspaceArg::Output { selector } => select(OUTPUT, selector),
            WorkspaceArg::Artifact { output, selector } => {
                select(&self.artifact_binding(output, bindings, span)?, selector)
            }
            WorkspaceArg::Input { input } => {
                let captured = &self.prepared.inputs[input.as_str()];
                let path = format!("/gripsack-inputs/{}", captured.identity);
                let node = self.local(&captured.artifact)?;
                bindings.readonly(path.clone(), node, span)?;
                match captured.kind {
                    InputKind::Directory => path,
                    InputKind::File => select(&path, "value"),
                }
            }
            WorkspaceArg::PackageCommand {
                package, command, ..
            } => {
                let path = self.package_binding(package, bindings, span)?;
                let WorkspaceOutput::Package(package) =
                    self.prepared.selection.outputs[package.as_str()]
                else {
                    unreachable!("package command admitted");
                };
                select(&path, &package.commands[command])
            }
        })
    }
    fn artifact_binding(
        &mut self,
        name: &str,
        bindings: &mut Bindings,
        span: &Span,
    ) -> Result<String, ExecError> {
        if matches!(
            self.prepared.selection.outputs[name],
            WorkspaceOutput::Package(_)
        ) {
            return self.package_binding(name, bindings, span);
        }
        let node = self.artifact(name)?;
        bindings.readonly(
            format!(
                "/gripsack-artifacts/{}",
                self.prepared.command_pins.artifacts[name]
            ),
            node,
            span,
        )
    }
    fn package_binding(
        &mut self,
        name: &str,
        bindings: &mut Bindings,
        span: &Span,
    ) -> Result<String, ExecError> {
        let WorkspaceOutput::Package(package) = self.prepared.selection.outputs[name] else {
            return Err(failure(span, "command binding does not name a package"));
        };
        let path = match &package.layout {
            PackageLayoutV6::Relocatable => format!(
                "/gripsack-packages/{}",
                self.prepared.package_productions[name]
            ),
            PackageLayoutV6::FixedPrefix { prefix } => prefix.as_str().to_owned(),
            PackageLayoutV6::PrefixMaterialized => {
                return Err(failure(
                    span,
                    "prefix-materialized packages bind their durable prefix at admission, not into an isolated build",
                ));
            }
        };
        if !bindings.packages.insert(name.to_owned()) {
            return Ok(path);
        }
        let node = self.artifact(name)?;
        bindings.readonly(path.clone(), node, span)?;
        for selector in package.commands.values() {
            let parent = selector.rsplit_once('/').map_or(".", |(parent, _)| parent);
            bindings.binaries.insert(select(&path, parent));
        }
        bindings.libraries.insert(select(&path, "lib"));
        bindings.libraries.insert(select(&path, "lib64"));
        for runtime in &package.runtime {
            self.package_binding(runtime, bindings, span)?;
        }
        Ok(path)
    }
}
/// Selectors have already passed the shared normalized-relative-path admission.
fn select(root: &str, selector: &str) -> String {
    if selector == "." {
        root.to_owned()
    } else {
        format!("{root}/{selector}")
    }
}
