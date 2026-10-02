use super::*;
verus! {
#[derive(Clone, Copy)]
pub(super) enum Kinds { Artifact, Recipe, Package, Check, Task, Environment, Schedule, Hook, All }
pub(super) open spec fn names_spec(kind: Kinds) -> Seq<Seq<char>> {
    match kind {
        Kinds::Artifact => artifact_kinds(),
        Kinds::Recipe => seq!["recipe"@], Kinds::Package => seq!["package"@],
        Kinds::Check => seq!["check"@], Kinds::Task => seq!["task"@],
        Kinds::Environment => seq!["environment"@], Kinds::Schedule => seq!["schedule"@],
        Kinds::Hook => seq!["hook"@], Kinds::All => all_kinds(),
    }
}
#[inline(always)]
pub(super) fn kind_names(kind: Kinds) -> (names: &'static [&'static str])
    ensures names@.map_values(|s: &str| s@) =~= names_spec(kind),
{
    match kind {
        Kinds::Artifact => &["recipe", "package"],
        Kinds::Recipe => &["recipe"], Kinds::Package => &["package"],
        Kinds::Check => &["check"], Kinds::Task => &["task"],
        Kinds::Environment => &["environment"], Kinds::Schedule => &["schedule"],
        Kinds::Hook => &["hook"],
        Kinds::All => &["recipe", "package", "environment", "task", "schedule", "check", "image", "profile", "hook"],
    }
}
}
