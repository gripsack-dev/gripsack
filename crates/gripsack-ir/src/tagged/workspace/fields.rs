//! The current workspace wire's closed tagged unions.

pub(super) fn output(kind: &str) -> Option<&'static [&'static str]> {
    Some(match kind {
        "recipe" => &[
            "kind",
            "name",
            "span",
            "source",
            "execution",
            "output_kind",
            "target",
            "steps",
            "checks",
        ],
        "package" => &[
            "kind",
            "name",
            "span",
            "producer",
            "commands",
            "runtime",
            "target",
            "layout",
            "host_runtime",
            "archive_root",
        ],
        "environment" => &[
            "kind", "name", "span", "packages", "target", "prefix", "env",
        ],
        "task" => &[
            "kind",
            "name",
            "span",
            "steps",
            "context",
            "services",
            "deps",
            "environment",
            "checks",
            "mutation_locks",
        ],
        "schedule" => &["kind", "name", "span", "task", "trigger", "scope"],
        "check" => &["kind", "name", "span", "run", "subject"],
        "image" => &[
            "kind",
            "name",
            "span",
            "packages",
            "target",
            "base",
            "destinations",
            "config",
        ],
        "profile" => &[
            "kind",
            "name",
            "span",
            "files",
            "environment",
            "schedules",
            "hooks",
            "launchers",
        ],
        "hook" => &["kind", "name", "span", "run", "trigger"],
        "service" => &[
            "kind",
            "name",
            "span",
            "image",
            "ports",
            "volumes",
            "env",
            "command",
            "health",
            "depends_on",
        ],
        _ => return None,
    })
}

pub(super) fn execution(kind: &str) -> Option<&'static [&'static str]> {
    Some(match kind {
        "host" => &["kind", "access"],
        "isolated_linux" => &["kind", "worker", "platform", "toolchain"],
        _ => return None,
    })
}

pub(super) fn layout(kind: &str) -> Option<&'static [&'static str]> {
    Some(match kind {
        "relocatable" | "prefix_materialized" => &["kind"],
        "fixed_prefix" => &["kind", "prefix"],
        _ => return None,
    })
}

pub(super) fn acquisition(kind: &str) -> Option<&'static [&'static str]> {
    Some(match kind {
        "fetch" => &["kind", "fetch", "span"],
        "package" => &["kind", "manifest", "span"],
        "conda_environment" => &[
            "kind",
            "channels",
            "packages",
            "wheels",
            "platforms",
            "system_requirements",
            "span",
        ],
        "pixi_lock" => &["kind", "manifest", "lock", "environment", "span"],
        _ => return None,
    })
}

pub(super) fn producer(kind: &str) -> Option<&'static [&'static str]> {
    Some(match kind {
        "recipe" => &["kind", "recipe"],
        "provider" => &["kind", "provider"],
        _ => return None,
    })
}

pub(super) fn command(kind: &str) -> Option<&'static [&'static str]> {
    Some(match kind {
        "exec" => &["kind", "span", "argv", "env", "cwd"],
        "run_bash" => &[
            "kind",
            "span",
            "interpreter",
            "options",
            "body",
            "env",
            "cwd",
            "line_map",
        ],
        _ => return None,
    })
}

pub(super) fn argument(kind: &str) -> Option<&'static [&'static str]> {
    Some(match kind {
        "literal" => &["kind", "value"],
        "artifact" => &["kind", "output", "selector"],
        "package_command" => &["kind", "package", "command", "sha256"],
        "input" => &["kind", "input"],
        "source" | "output" => &["kind", "selector"],
        _ => return None,
    })
}

pub(super) fn path(kind: &str) -> Option<&'static [&'static str]> {
    Some(match kind {
        "literal" => &["kind", "value"],
        "artifact" => &["kind", "output", "selector"],
        "host" => &["kind", "path"],
        "source" | "output" => &["kind", "selector"],
        _ => return None,
    })
}

pub(super) fn source(kind: &str) -> Option<&'static [&'static str]> {
    Some(match kind {
        "repo_file" => &["kind", "path"],
        "artifact_file" => &["kind", "output", "selector"],
        "repo_tree" => &["kind", "path", "include", "exclude"],
        "tree" => &["kind", "output", "include", "exclude"],
        _ => return None,
    })
}

pub(super) fn content(kind: &str) -> Option<&'static [&'static str]> {
    Some(match kind {
        "identity" => &["kind"],
        "literal" => &["kind", "text"],
        "template" => &["kind", "template", "variables", "result_digest"],
        "template_file" => &["kind", "variables", "result_digest"],
        _ => return None,
    })
}

pub(super) fn destination(kind: &str) -> Option<&'static [&'static str]> {
    Some(match kind {
        "symlink" | "tracked_copy" => &["kind", "path"],
        "managed_block" => &["kind", "path", "marker"],
        _ => return None,
    })
}

pub(super) fn calendar(kind: &str) -> Option<&'static [&'static str]> {
    Some(match kind {
        "daily" => &["kind", "time"],
        "weekly" => &["kind", "weekday", "time"],
        _ => return None,
    })
}

pub(super) fn archive_root(kind: &str) -> Option<&'static [&'static str]> {
    match kind {
        "single_directory" => Some(&["kind"]),
        _ => None,
    }
}

pub(super) fn input(kind: &str) -> Option<&'static [&'static str]> {
    Some(match kind {
        "repo_file" => &["kind", "path"],
        "repo_directory" => &["kind", "path", "include", "exclude"],
        _ => return None,
    })
}

pub(super) fn action(kind: &str) -> Option<&'static [&'static str]> {
    match kind {
        "ensure_artifact" => Some(&["kind", "output", "span"]),
        _ => None,
    }
}

pub(super) fn context(kind: &str) -> Option<&'static [&'static str]> {
    match kind {
        "host" => Some(&["kind", "mutable_paths"]),
        _ => None,
    }
}
