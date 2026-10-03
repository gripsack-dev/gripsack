//! Context admission for the current workspace command grammar. The decoded
//! variants, not filenames or cache flags, determine which references may cross
//! each boundary. Effects and executable-byte acquisition remain in adapters.
use vstd::prelude::*;

verus! {
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionContext { Host, IsolatedLinux }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectoryOrigin { Literal, Artifact, LiveHost, ProductionSource, StagingOutput }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgumentPosition { Argument, Environment, Interpreter }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgumentOrigin { Literal, Artifact, CapturedInput, PackageCommand, ProductionSource, StagingOutput }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandOwner { Production, Invocation }

pub fn admit_production_binding(owner: CommandOwner, origin: ArgumentOrigin) -> (admitted: bool)
    ensures admitted == (
        owner == CommandOwner::Production
        || (origin != ArgumentOrigin::ProductionSource && origin != ArgumentOrigin::StagingOutput)
    ),
{
    matches!(owner, CommandOwner::Production)
        || !matches!(origin, ArgumentOrigin::ProductionSource | ArgumentOrigin::StagingOutput)
}

pub open spec fn permitted_directory(context: ExecutionContext, origin: DirectoryOrigin) -> bool {
    !(context == ExecutionContext::IsolatedLinux && origin == DirectoryOrigin::LiveHost)
}
pub fn admit_directory(context: ExecutionContext, origin: DirectoryOrigin) -> (admitted: bool)
    ensures admitted == permitted_directory(context, origin),
{
    !matches!((context, origin), (ExecutionContext::IsolatedLinux, DirectoryOrigin::LiveHost))
}

pub open spec fn permitted_argument(position: ArgumentPosition, origin: ArgumentOrigin) -> bool {
    match position {
        ArgumentPosition::Argument => true,
        ArgumentPosition::Environment => origin != ArgumentOrigin::PackageCommand,
        ArgumentPosition::Interpreter => origin == ArgumentOrigin::PackageCommand,
    }
}
pub fn admit_argument(position: ArgumentPosition, origin: ArgumentOrigin) -> (admitted: bool)
    ensures admitted == permitted_argument(position, origin),
{
    match position {
        ArgumentPosition::Argument => true,
        ArgumentPosition::Environment => !matches!(origin, ArgumentOrigin::PackageCommand),
        ArgumentPosition::Interpreter => matches!(origin, ArgumentOrigin::PackageCommand),
    }
}

pub open spec fn strict_options(options: Seq<String>) -> bool {
    options.len() == 4
        && options[0]@ == "-e"@
        && options[1]@ == "-u"@
        && options[2]@ == "-o"@
        && options[3]@ == "pipefail"@
}
/// Admit the literal, ordered strict option set. Executable acquisition and
/// propagation of these options into the launcher are separate adapter duties.
pub fn admit_bash_options(options: &[String]) -> (admitted: bool)
    ensures admitted == strict_options(options@),
{
    options.len() == 4 && options[0].as_str() == "-e" && options[1].as_str() == "-u"
        && options[2].as_str() == "-o" && options[3].as_str() == "pipefail"
}

pub proof fn interpreter_and_environment_roles_are_disjoint(origin: ArgumentOrigin)
    ensures !(permitted_argument(ArgumentPosition::Interpreter, origin)
        && permitted_argument(ArgumentPosition::Environment, origin)),
{}

pub proof fn isolated_execution_never_admits_live_host_directory(origin: DirectoryOrigin)
    requires permitted_directory(ExecutionContext::IsolatedLinux, origin),
    ensures origin != DirectoryOrigin::LiveHost,
{}
}
