//! Context admission for the current workspace command grammar. The decoded
//! variants, not filenames or cache flags, determine which references may cross
//! each boundary. Effects and executable-byte acquisition remain in adapters.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionContext {
    Host,
    IsolatedLinux,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectoryOrigin {
    Literal,
    Artifact,
    LiveHost,
    ProductionSource,
    StagingOutput,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgumentPosition {
    Argument,
    Environment,
    Interpreter,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgumentOrigin {
    Literal,
    Artifact,
    CapturedInput,
    PackageCommand,
    ProductionSource,
    StagingOutput,
}
/// Production owns staging; artifact and explicitly attached file checks
/// receive only their subject binding. A deployed file subject is observed
/// host state, not an immutable package or writable production output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandOwner {
    Production,
    ImmutableSubject,
    FileSubject,
    Invocation,
}

/// Only production may bind its own staging output; subjects may read a
/// production source; anyone may bind an origin that is neither.
pub fn admit_production_binding(owner: CommandOwner, origin: ArgumentOrigin) -> bool {
    matches!(owner, CommandOwner::Production)
        || (matches!(
            owner,
            CommandOwner::ImmutableSubject | CommandOwner::FileSubject
        ) && matches!(origin, ArgumentOrigin::ProductionSource))
        || !matches!(
            origin,
            ArgumentOrigin::ProductionSource | ArgumentOrigin::StagingOutput
        )
}

/// Isolated execution never sees a live host directory.
pub fn admit_directory(context: ExecutionContext, origin: DirectoryOrigin) -> bool {
    !matches!(
        (context, origin),
        (ExecutionContext::IsolatedLinux, DirectoryOrigin::LiveHost)
    )
}

/// Interpreters come only from package commands; package commands never
/// enter the environment.
pub fn admit_argument(position: ArgumentPosition, origin: ArgumentOrigin) -> bool {
    match position {
        ArgumentPosition::Argument => true,
        ArgumentPosition::Environment => !matches!(origin, ArgumentOrigin::PackageCommand),
        ArgumentPosition::Interpreter => matches!(origin, ArgumentOrigin::PackageCommand),
    }
}

/// Admit the literal, ordered strict option set. Executable acquisition and
/// propagation of these options into the launcher are separate adapter duties.
pub fn admit_bash_options(options: &[String]) -> bool {
    options == ["-e", "-u", "-o", "pipefail"]
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONTEXTS: [ExecutionContext; 2] =
        [ExecutionContext::Host, ExecutionContext::IsolatedLinux];
    const DIRECTORIES: [DirectoryOrigin; 5] = [
        DirectoryOrigin::Literal,
        DirectoryOrigin::Artifact,
        DirectoryOrigin::LiveHost,
        DirectoryOrigin::ProductionSource,
        DirectoryOrigin::StagingOutput,
    ];
    const POSITIONS: [ArgumentPosition; 3] = [
        ArgumentPosition::Argument,
        ArgumentPosition::Environment,
        ArgumentPosition::Interpreter,
    ];
    const ORIGINS: [ArgumentOrigin; 6] = [
        ArgumentOrigin::Literal,
        ArgumentOrigin::Artifact,
        ArgumentOrigin::CapturedInput,
        ArgumentOrigin::PackageCommand,
        ArgumentOrigin::ProductionSource,
        ArgumentOrigin::StagingOutput,
    ];
    const OWNERS: [CommandOwner; 4] = [
        CommandOwner::Production,
        CommandOwner::ImmutableSubject,
        CommandOwner::FileSubject,
        CommandOwner::Invocation,
    ];

    #[test]
    fn isolated_execution_never_admits_a_live_host_directory() {
        for context in CONTEXTS {
            for origin in DIRECTORIES {
                let expected = !(context == ExecutionContext::IsolatedLinux
                    && origin == DirectoryOrigin::LiveHost);
                assert_eq!(
                    admit_directory(context, origin),
                    expected,
                    "{context:?} {origin:?}"
                );
            }
        }
    }

    #[test]
    fn interpreter_and_environment_roles_are_disjoint() {
        for position in POSITIONS {
            for origin in ORIGINS {
                let expected = match position {
                    ArgumentPosition::Argument => true,
                    ArgumentPosition::Environment => origin != ArgumentOrigin::PackageCommand,
                    ArgumentPosition::Interpreter => origin == ArgumentOrigin::PackageCommand,
                };
                assert_eq!(
                    admit_argument(position, origin),
                    expected,
                    "{position:?} {origin:?}"
                );
            }
        }
        for origin in ORIGINS {
            assert!(
                !(admit_argument(ArgumentPosition::Interpreter, origin)
                    && admit_argument(ArgumentPosition::Environment, origin)),
                "{origin:?}"
            );
        }
    }

    #[test]
    fn only_production_binds_its_staging_output() {
        for owner in OWNERS {
            for origin in ORIGINS {
                let expected = owner == CommandOwner::Production
                    || (matches!(
                        owner,
                        CommandOwner::ImmutableSubject | CommandOwner::FileSubject
                    ) && origin == ArgumentOrigin::ProductionSource)
                    || !matches!(
                        origin,
                        ArgumentOrigin::ProductionSource | ArgumentOrigin::StagingOutput
                    );
                assert_eq!(
                    admit_production_binding(owner, origin),
                    expected,
                    "{owner:?} {origin:?}"
                );
            }
        }
        assert!(!admit_production_binding(
            CommandOwner::FileSubject,
            ArgumentOrigin::StagingOutput
        ));
    }

    #[test]
    fn bash_options_are_exactly_the_strict_set_in_order() {
        let strict = ["-e", "-u", "-o", "pipefail"].map(String::from);
        assert!(admit_bash_options(&strict));
        for wrong in [
            &["-e", "-e", "-o", "pipefail"][..],
            &["-u", "-e", "-o", "pipefail"],
            &["-e", "-u", "-o"],
            &["-e", "-u", "-o", "pipefail", "-x"],
            &[],
        ] {
            let wrong: Vec<String> = wrong.iter().map(|o| o.to_string()).collect();
            assert!(!admit_bash_options(&wrong), "{wrong:?}");
        }
    }
}
