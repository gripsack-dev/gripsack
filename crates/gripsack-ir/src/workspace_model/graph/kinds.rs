#[derive(Clone, Copy)]
pub(super) enum Kinds {
    Artifact,
    Recipe,
    Package,
    Check,
    Task,
    Environment,
    Schedule,
    Hook,
    Service,
    All,
}
#[inline(always)]
pub(super) fn kind_names(kind: Kinds) -> &'static [&'static str] {
    match kind {
        Kinds::Artifact => &["recipe", "package"],
        Kinds::Recipe => &["recipe"],
        Kinds::Package => &["package"],
        Kinds::Check => &["check"],
        Kinds::Task => &["task"],
        Kinds::Environment => &["environment"],
        Kinds::Schedule => &["schedule"],
        Kinds::Hook => &["hook"],
        Kinds::Service => &["service"],
        Kinds::All => &[
            "recipe",
            "package",
            "environment",
            "task",
            "schedule",
            "check",
            "image",
            "profile",
            "hook",
            "service",
        ],
    }
}
