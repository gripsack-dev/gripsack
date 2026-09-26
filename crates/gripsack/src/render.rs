//! Diagnostic rendering (0004 §3): colors when the terminal supports
//! them, source snippets when the file is reachable. Spans reference the
//! user's frontend code — a missing file degrades to the header alone,
//! never an error.

use gripsack_ir::{Diagnostic, HostName, Ir, Severity, Span};
use owo_colors::OwoColorize;
use std::io::{IsTerminal, Read};
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Copy, Default)]
pub struct Palette {
    pub enabled: bool,
}

impl Palette {
    pub fn detect() -> Self {
        Palette {
            enabled: std::io::stdout().is_terminal(),
        }
    }

    #[cfg(test)]
    pub fn plain() -> Self {
        Palette::default()
    }

    /// Styling helpers: colors follow the terminal (main.rs doc) —
    /// piped output is plain. Every ad-hoc `.green()` at a call site
    /// was a leak of that contract; these make the gate structural.
    pub fn good(&self, text: &str) -> String {
        self.style(text, |t| t.green().bold().to_string())
    }

    pub fn warn(&self, text: &str) -> String {
        self.style(text, |t| t.yellow().bold().to_string())
    }

    pub fn badge(&self, text: &str) -> String {
        self.style(text, |t| t.blue().bold().to_string())
    }

    pub fn cyan(&self, text: &str) -> String {
        self.style(text, |t| t.cyan().to_string())
    }

    pub fn dim(&self, text: &str) -> String {
        self.style(text, |t| t.dimmed().to_string())
    }

    pub fn error(&self, text: &str) -> String {
        self.style(text, |t| t.red().bold().to_string())
    }

    fn style(&self, text: &str, styled: impl FnOnce(&str) -> String) -> String {
        if self.enabled {
            styled(text)
        } else {
            text.to_string()
        }
    }
}

/// Optional source snippets never cross the evaluated repo capability.
/// File contents are diagnostic decoration, bounded independently of
/// the IR and omitted when a path is outside this pinned directory.
struct SourceRoot {
    path: PathBuf,
    dir: gripsack_fs::Dir,
}

impl SourceRoot {
    fn open(path: &Path) -> Option<Self> {
        let path = path.canonicalize().ok()?;
        let dir = gripsack_fs::open(&path).ok()?;
        Some(Self { path, dir })
    }

    fn read(&self, source: &str) -> Option<String> {
        let path = Path::new(source);
        let relative = if path.is_absolute() {
            path.strip_prefix(&self.path).ok()?
        } else {
            path
        };
        if !relative
            .components()
            .all(|part| matches!(part, Component::Normal(_) | Component::CurDir))
        {
            return None;
        }
        let file = self.dir.open(relative).ok()?;
        let metadata = file.metadata().ok()?;
        if !metadata.is_file() || metadata.len() > MAX_SOURCE_SNIPPET_BYTES {
            return None;
        }
        let mut contents = Vec::with_capacity(metadata.len() as usize);
        file.take(MAX_SOURCE_SNIPPET_BYTES + 1)
            .read_to_end(&mut contents)
            .ok()?;
        if contents.len() as u64 > MAX_SOURCE_SNIPPET_BYTES {
            return None;
        }
        String::from_utf8(contents).ok()
    }
}

/// Optional snippets are bounded to 1 MiB per source file. Larger
/// files retain the diagnostic code and label without a source read.
const MAX_SOURCE_SNIPPET_BYTES: u64 = 1_048_576;

fn render_diagnostic_impl(d: &Diagnostic, palette: Palette, root: Option<&SourceRoot>) -> String {
    let mut out = String::new();
    let header = format!("{}[{}]: {}", d.severity, d.code, d.message);
    out.push_str(&match (d.severity, palette.enabled) {
        (Severity::Error, true) => header.red().bold().to_string(),
        (Severity::Warning, true) => header.yellow().bold().to_string(),
        _ => header,
    });
    for label in &d.labels {
        match &label.span {
            Some(span) => {
                let arrow = format!("\n  --> {span}");
                out.push_str(&if palette.enabled {
                    arrow.blue().to_string()
                } else {
                    arrow
                });
                out.push_str(&snippet(span, &label.note, palette, root));
            }
            None if !label.note.is_empty() => {
                out.push_str(&format!("\n  = {}", label.note));
            }
            None => {}
        }
    }
    if let Some(help) = &d.help {
        let line = format!("\n  help: {help}");
        out.push_str(&if palette.enabled {
            line.green().to_string()
        } else {
            line
        });
    }
    out
}

/// The rustc-style snippet: gutter, source line, bounded caret.
/// Invalid coordinates and source files outside the repo fail closed
/// to the label alone. The directory capability resolves symlinks
/// without an out-of-root check/use race.
fn snippet(span: &Span, note: &str, palette: Palette, root: Option<&SourceRoot>) -> String {
    if span.line == 0 || span.col == Some(0) {
        return String::new();
    }
    let Some(contents) = root.and_then(|root| root.read(&span.file)) else {
        return String::new();
    };
    let Some(source_line) = contents.lines().nth((span.line - 1) as usize) else {
        return String::new();
    };
    let caret_pad = span.col.unwrap_or(1).saturating_sub(1) as usize;
    if caret_pad > source_line.len() {
        return String::new();
    }
    let gutter = format!("{:>3} |", span.line);
    let caret = format!("{}^", " ".repeat(caret_pad));
    let mut out = format!("\n   |\n {gutter} {source_line}\n   | {caret}");
    if !note.is_empty() {
        out.push(' ');
        out.push_str(note);
    }
    if palette.enabled {
        format!("\n{}", out.dimmed())
    } else {
        out
    }
}

/// Render one module's plan (0007 §5): what it fetches, deploys, needs,
/// which wave it lands in — and how reversible each mutation is
/// (review 0020 §10): a plan that says what it will do should also
/// say what undoing it means.
pub fn render_module(ir: &Ir, name: &str, waves: &[Vec<String>], palette: Palette) -> String {
    let Some(module) = ir.modules.get(name) else {
        return format!("no module {name:?}");
    };
    let wave = waves
        .iter()
        .position(|w| w.iter().any(|m| m == name))
        .map(|i| i.to_string())
        .unwrap_or_else(|| "?".into());
    let title = format!("{name}  (wave {wave})");
    let mut out = if palette.enabled {
        title.green().bold().to_string()
    } else {
        title
    };
    if let Some(fetch) = &module.fetch {
        out.push_str(&format!(
            "\n  fetch    {}",
            gripsack_exec::report::describe_fetch(fetch)
        ));
    }
    // 0039: a build-only dependency deploys nothing — its declared
    // install/config lines must not read as plan intent
    let build_only = gripsack_ir::dependencies::build_only_modules(&ir.modules).contains(name);
    if build_only {
        let consumers: Vec<&str> = ir
            .modules
            .iter()
            .filter(|(_, m)| {
                m.depends
                    .iter()
                    .any(|d| d.module == name && d.edge == gripsack_ir::EdgeKind::Build)
            })
            .map(|(n, _)| n.as_str())
            .collect();
        out.push_str(&format!(
            "\n  closure  build-only ({} builds against it) — fetched + staged, never deployed",
            consumers.join(", ")
        ));
    }
    for entry in module.install.iter().filter(|_| !build_only) {
        out.push_str(&format!(
            "\n  install  {} → {} ({:?}) [reversible: prior state recorded]",
            entry.from, entry.to, entry.mode
        ));
    }
    for entry in module.config.iter().filter(|_| !build_only) {
        out.push_str(&format!(
            "\n  config   {} → {} ({:?}) [reversible: prior state recorded]",
            entry.from, entry.to, entry.mode
        ));
    }
    for dep in module.depends.iter() {
        out.push_str(&format!("\n  depends  {} ({:?})", dep.module, dep.edge));
    }
    let dependents: Vec<_> = ir
        .modules
        .iter()
        .filter(|(_, m)| m.depends.iter().any(|d| d.module == name))
        .map(|(n, _)| n.as_str())
        .collect();
    if !dependents.is_empty() {
        out.push_str(&format!("\n  blocks   {}", dependents.join(", ")));
    }
    for intent in module.activate.iter().filter(|_| !build_only) {
        out.push_str(&format!(
            "\n  activate {:?} [best-effort: adapter re-runs, no automatic inverse]",
            intent.action
        ));
    }
    if let Some(steps) = &module.steps {
        out.push_str("\n  steps");
        for step in steps {
            if build_only
                && matches!(
                    step.action,
                    gripsack_ir::StepAction::Install { .. }
                        | gripsack_ir::StepAction::ConfigDeploy { .. }
                        | gripsack_ir::StepAction::Intent { .. }
                        | gripsack_ir::StepAction::Verify {
                            verify: gripsack_ir::Verify::FileDeployed { .. }
                        }
                )
            {
                continue;
            }
            let needs = if step.needs.is_empty() {
                String::new()
            } else {
                format!(" ← {}", step.needs.join(", "))
            };
            let risk = match &step.action {
                // deploy steps record priors — the journal covers them
                gripsack_ir::step::StepAction::Install { .. }
                | gripsack_ir::step::StepAction::ConfigDeploy { .. } => "",
                gripsack_ir::step::StepAction::Fetch { .. } => "",
                // custom code runs with your privileges; undoing it
                // needs its own inverse, which gripsack cannot know
                gripsack_ir::step::StepAction::Run { .. }
                | gripsack_ir::step::StepAction::CustomShell { .. }
                | gripsack_ir::step::StepAction::Build { .. } => {
                    "  [no automatic inverse: runs custom code]"
                }
                gripsack_ir::step::StepAction::Intent { .. } => "  [best-effort: adapter re-runs]",
                _ => "",
            };
            out.push_str(&format!("\n    {}{needs}{risk}", step.id));
        }
    }
    out
}

/// Render diagnostic facts without opening source paths. Use the
/// bounded variant when a trusted repo root is available.
pub fn render_diagnostics(diagnostics: &[Diagnostic], palette: Palette) -> String {
    render_diagnostics_impl(diagnostics, palette, None)
}

/// Render source snippets only through a capability rooted at `root`.
pub fn render_diagnostics_bounded(
    diagnostics: &[Diagnostic],
    palette: Palette,
    root: &Path,
) -> String {
    let source = SourceRoot::open(root);
    render_diagnostics_impl(diagnostics, palette, source.as_ref())
}

fn render_diagnostics_impl(
    diagnostics: &[Diagnostic],
    palette: Palette,
    source: Option<&SourceRoot>,
) -> String {
    let mut out = String::new();
    for diagnostic in diagnostics {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&render_diagnostic_impl(diagnostic, palette, source));
    }
    out
}

/// Where source-aware diagnostics go (0052 A1-06): every Diagnostic
/// the eval/check pipeline raises flows through this sink, so the
/// terminal rendering (color + source snippets) and `grip check
/// --json`'s document carry the SAME values — the facts cannot drift
/// between surfaces.
pub struct DiagnosticSink {
    palette: Palette,
    /// Pinned evaluated repo; absent for JSON (no source reads).
    source: Option<SourceRoot>,
    /// Some(collected) under --json; None prints as raised.
    json: Option<Vec<Diagnostic>>,
}

impl DiagnosticSink {
    /// Human surface: diagnostics render to stderr as they are raised.
    pub fn terminal(palette: Palette, repo: &Path) -> Self {
        DiagnosticSink {
            palette,
            json: None,
            source: SourceRoot::open(repo),
        }
    }

    /// Machine surface: diagnostics collect into the check document;
    /// source snippets are never read for JSON output.
    pub fn json() -> Self {
        DiagnosticSink {
            palette: Palette::default(),
            json: Some(Vec::new()),
            source: None,
        }
    }

    /// Styling for non-diagnostic lines; disabled under --json (the
    /// document is never colored).
    pub fn palette(&self) -> Palette {
        self.palette
    }

    pub fn is_json(&self) -> bool {
        self.json.is_some()
    }

    /// Record diagnostics: rendered now on the terminal surface,
    /// collected for the JSON document on the machine surface.
    pub fn report(&mut self, diagnostics: &[Diagnostic]) {
        match &mut self.json {
            Some(collected) => collected.extend(diagnostics.iter().cloned()),
            None if !diagnostics.is_empty() => {
                eprintln!(
                    "{}",
                    render_diagnostics_impl(diagnostics, self.palette, self.source.as_ref())
                );
            }
            None => {}
        }
    }

    /// The collected diagnostics (json mode); empty on the terminal
    /// surface, where they printed as raised.
    pub fn into_collected(self) -> Vec<Diagnostic> {
        self.json.unwrap_or_default()
    }

    /// Close a failing run: the JSON surface emits its document on
    /// stdout; the terminal surface already printed. Operational
    /// failures with no diagnostics (trust gate, missing deno) keep
    /// their stderr text; the empty document is the honest JSON fact.
    pub fn finish_failure(self, code: std::process::ExitCode) -> std::process::ExitCode {
        if let Some(diagnostics) = self.json {
            println!("{}", CheckReport::failure(diagnostics).to_json());
        }
        code
    }
}

/// `grip check --json`'s document format version.
pub const CHECK_JSON_VERSION: u32 = 1;

/// `grip check --json`'s document (0052 A1-06): failures carry the
/// diagnostics; success carries the same facts the terminal listing
/// prints (host, named outputs or legacy modules, layout notes).
#[derive(Debug, serde::Serialize)]
pub struct CheckReport {
    pub version: u32,
    pub ok: bool,
    /// Every diagnostic raised — errors on failure, non-fatal warnings
    /// on success. The same values the terminal renders.
    pub diagnostics: Vec<Diagnostic>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<CheckHostReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outputs: Option<Vec<CheckOutputReport>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modules: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layouts: Option<std::collections::BTreeMap<String, String>>,
}

#[derive(Debug, serde::Serialize)]
pub struct CheckHostReport {
    pub os: String,
    pub arch: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

#[derive(Debug, serde::Serialize)]
pub struct CheckOutputReport {
    pub name: String,
    pub kind: String,
    pub span: Span,
}

impl CheckReport {
    fn base(ok: bool, diagnostics: Vec<Diagnostic>) -> Self {
        CheckReport {
            version: CHECK_JSON_VERSION,
            ok,
            diagnostics,
            host: None,
            outputs: None,
            modules: None,
            layouts: None,
        }
    }

    pub fn failure(diagnostics: Vec<Diagnostic>) -> Self {
        CheckReport::base(false, diagnostics)
    }

    /// Success carries any non-fatal warnings plus the listing facts.
    pub fn success(diagnostics: Vec<Diagnostic>) -> Self {
        CheckReport::base(true, diagnostics)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("the check report is plain owned data")
    }
}

/// `grip plan`'s change section: what apply would do, computed against
/// the current generation (0004 pass 5 — the diff that sells the
/// architecture). Config entries hash offline; fetched modules show
/// their store-path satisfaction without a fetch.
pub fn diff_section(
    ir: &Ir,
    repo: &Path,
    host: &HostName,
    adopting: &std::collections::BTreeSet<String>,
    palette: Palette,
) -> Result<String, gripsack_exec::ExecError> {
    let home = gripsack_store::gripsack_home();
    let current = gripsack_store::current_generation(&home)?
        .map(|number| gripsack_store::read_manifest(&home, number))
        .transpose()?;
    let c = |s: &str| {
        if palette.enabled {
            s.cyan().to_string()
        } else {
            s.to_string()
        }
    };
    let b = |s: &str| {
        if palette.enabled {
            s.bold().to_string()
        } else {
            s.to_string()
        }
    };
    let mut out = vec![match &current {
        Some(m) => format!("{} generation {}:", b("changes vs"), m.number),
        None => b("changes: no current generation (first apply)"),
    }];

    // THE operation list (0034): what apply would execute, rendered.
    // plan/apply agreement is by construction — one planner.
    // the lockfile resolves warm fetched payloads to their store
    // paths (0035 F7) — a deployed module previews satisfied, not
    // deferred; a cold or unpinned one stays deferred
    let lock = match gripsack_exec::lockfile::read(repo, host) {
        gripsack_exec::lockfile::LockRead::Parsed(lock) => lock,
        gripsack_exec::lockfile::LockRead::Missing => Default::default(),
        gripsack_exec::lockfile::LockRead::Corrupt(detail) => {
            return Err(gripsack_exec::ExecError::Step {
                module: "*".into(),
                step: "lockfile".into(),
                detail,
            });
        }
    };
    let ops = gripsack_exec::ops::preview_ops(ir, repo, current.as_ref(), adopting, &lock)?;
    let mut by_module: std::collections::BTreeMap<&str, Vec<String>> =
        std::collections::BTreeMap::new();
    for op in &ops {
        use gripsack_exec::ops::{Authority, OpKind};
        let line = match op.kind() {
            OpKind::Link { .. } | OpKind::Write { .. } => match op.authority() {
                Some(Authority::Fresh) => {
                    let from = op
                        .produces()
                        .map(|p| p.from.to_string_lossy().into_owned())
                        .unwrap_or_else(|| op.declared_to().to_string());
                    format!("  + {from} → {} (new)", op.declared_to())
                }
                Some(Authority::Update) => {
                    let from = op
                        .produces()
                        .map(|p| p.from.to_string_lossy().into_owned())
                        .unwrap_or_else(|| op.declared_to().to_string());
                    format!("  ~ {from} → {} (update)", op.declared_to())
                }
                Some(Authority::TakeOver) => {
                    // 0015 §7 S6: this take-over is the point of the
                    // command — say so, don't demand a flag
                    format!("  ↻ {} will be adopted (prior recorded)", op.declared_to())
                }
                _ => format!("  ? {}", op.declared_to()),
            },
            OpKind::MergeUpsert { .. } => {
                let from = op
                    .produces()
                    .map(|p| p.from.to_string_lossy().into_owned())
                    .unwrap_or_else(|| op.declared_to().to_string());
                format!("  ~ {from} → {} (update)", op.declared_to())
            }
            OpKind::Remove(_) => format!("  - {} (prune)", op.declared_to()),
            OpKind::Satisfied => format!("  = {} (satisfied)", op.declared_to()),
            OpKind::Preserved => match op.authority() {
                Some(Authority::Foreign) => {
                    format!(
                        "  ! {} exists, not ours — needs --take-over",
                        op.declared_to()
                    )
                }
                _ => format!("  ~ {} drifted — kept (apply preserves)", op.declared_to()),
            },
            OpKind::RunEffect | OpKind::Deferred => {
                format!("  · {}", op.note().unwrap_or("deferred"))
            }
        };
        by_module.entry(op.module()).or_default().push(line);
    }
    for (name, lines) in &by_module {
        // one marker note per module, not per step (dedup)
        let mut seen_notes = std::collections::BTreeSet::new();
        let mut deduped = Vec::new();
        for line in lines {
            if line.starts_with("  ·") && !seen_notes.insert(line.clone()) {
                continue;
            }
            deduped.push(line.clone());
        }
        out.push(format!("  {}", c(name)));
        out.extend(deduped);
    }
    if out.len() == 1 {
        out.push("  nothing would change".into());
    }
    Ok(out.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gripsack_ir::codes;

    #[test]
    fn renders_header_and_help_without_color() {
        let d = Diagnostic::error(codes::UNKNOWN_DEPENDENCY, "module \"a\" is unknown")
            .with_help("declare it in modules/");
        let out = render_diagnostics(&[d], Palette::plain());
        assert!(out.contains("error[E101]"));
        assert!(out.contains("help: declare it"));
        assert!(!out.contains('\u{1b}'));
    }

    #[test]
    fn renders_snippet_when_file_exists() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("mod.py");
        std::fs::write(&file, "line one\nline two\nline three\n").unwrap();
        let d = Diagnostic::error(codes::BAD_DESTINATION, "bad dest").with_label(
            Some(Span {
                file: file.to_string_lossy().into_owned(),
                line: 2,
                col: Some(3),
            }),
            "here",
        );
        let out = render_diagnostics_bounded(&[d], Palette::plain(), dir.path());
        assert!(out.contains("line two"));
        assert!(out.contains("  ^ here"));
    }

    #[test]
    fn missing_file_degrades_gracefully() {
        let d = Diagnostic::error(codes::BAD_DESTINATION, "bad dest").with_label(
            Some(Span {
                file: "/nonexistent/mod.py".into(),
                line: 1,
                col: None,
            }),
            "here",
        );
        let out = render_diagnostics(&[d], Palette::plain());
        assert!(out.contains("--> /nonexistent/mod.py:1"));
        assert!(!out.contains('|'));
    }

    #[test]
    fn invalid_diagnostic_coordinates_never_read_a_source_snippet() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("secret.ts");
        std::fs::write(&file, "secret material\n").unwrap();
        for (line, col) in [(0, None), (1, Some(u32::MAX))] {
            let diagnostic = Diagnostic::error(codes::BAD_DESTINATION, "invalid source location")
                .with_label(
                    Some(Span {
                        file: file.to_string_lossy().into_owned(),
                        line,
                        col,
                    }),
                    "source reference",
                );
            let rendered = render_diagnostics_bounded(&[diagnostic], Palette::plain(), dir.path());
            assert!(rendered.starts_with("error["));
            assert!(!rendered.contains("secret material"));
        }
    }

    #[cfg(unix)]
    #[test]
    fn source_snippets_cannot_escape_the_repo_capability() {
        let repo = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        let private_file = external.path().join("private.txt");
        std::fs::write(&private_file, "outside secret\n").unwrap();
        let diagnostic = |file: &Path| {
            Diagnostic::error(codes::BAD_DESTINATION, "bad source").with_label(
                Some(Span {
                    file: file.to_string_lossy().into_owned(),
                    line: 1,
                    col: None,
                }),
                "source reference",
            )
        };
        let direct =
            render_diagnostics_bounded(&[diagnostic(&private_file)], Palette::plain(), repo.path());
        assert!(!direct.contains("outside secret"));

        let link = repo.path().join("outside.txt");
        std::os::unix::fs::symlink(&private_file, &link).unwrap();
        let linked =
            render_diagnostics_bounded(&[diagnostic(&link)], Palette::plain(), repo.path());
        assert!(!linked.contains("outside secret"));

        let local = repo.path().join("inside.txt");
        std::fs::write(&local, "inside source\n").unwrap();
        let admitted =
            render_diagnostics_bounded(&[diagnostic(&local)], Palette::plain(), repo.path());
        assert!(admitted.contains("inside source"));
    }
}
