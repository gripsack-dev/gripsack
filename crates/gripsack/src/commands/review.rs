//! Offline review is a reader, never an evaluator or an execution authority.
//! Terminal and JSON output serialize the same admitted, disclosure-filtered facts.
use gripsack_exec::semantic::{self, SemanticSnapshot};
use std::{fs::OpenOptions, io::Read, os::unix::fs::OpenOptionsExt, path::Path, process::ExitCode};

/// The CLI's input ceiling also bounds proposed IR and explicit config views.
const MAX_REVIEW_INPUT_BYTES: u64 = 16 * 1024 * 1024;
const CHANGED: u8 = 2;
const INCOMPLETE: u8 = 3;

pub fn diff(before: &str, after: &str, repo: &Path, json: bool) -> ExitCode {
    let result = (|| {
        let before = select(before, repo)?;
        let after = select(after, repo)?;
        let changes = semantic::compare(&before, &after);
        let status = if !changes.is_complete() {
            INCOMPLETE
        } else if changes.has_changes() {
            CHANGED
        } else {
            0
        };
        emit(
            &changes,
            json,
            match status {
                INCOMPLETE => "Incomplete comparison: unknown evidence is not equality.",
                CHANGED => "Captured meaning or provenance changed.",
                _ => "No change in the captured comparison model.",
            },
        )?;
        Ok(ExitCode::from(status))
    })();
    finish(result)
}

pub fn capture(selector: &str, repo: &Path, out: &Path) -> ExitCode {
    finish((|| {
        let snapshot = select(selector, repo)?;
        semantic::write_snapshot_private(out, &snapshot).map_err(|error| error.to_string())?;
        println!("Saved private semantic snapshot; it grants no execution authority.");
        Ok(ExitCode::SUCCESS)
    })())
}

#[derive(serde::Serialize)]
struct PlanReport<'a> {
    version: u32,
    selected_profiles: &'a [String],
    complete: bool,
    execution_authority: bool,
    comparison_scope: &'static str,
    changes: &'a semantic::ChangeSet,
    operations: &'a [semantic::operations::OperationExplanation],
}

/// Explicit live planning supplies already evaluated IR; this function itself
/// remains read-only and reports the shared planner without filtering its ops.
pub fn plan(
    ir: &gripsack_ir::Ir,
    repository: &gripsack_exec::Repository,
    selected: &[String],
    limits: gripsack_fetch::FetchLimits,
    json: bool,
) -> ExitCode {
    finish((|| {
        let home = gripsack_store::gripsack_home();
        let current = gripsack_store::current_generation(&home)
            .map_err(|error| error.to_string())?
            .map(|number| gripsack_store::generations::admit_manifest(&home, number))
            .transpose()
            .map_err(|error| error.to_string())?;
        let operations = gripsack_exec::ops::preview_ops(
            ir,
            repository,
            current.as_ref(),
            &Default::default(),
            limits,
            selected,
        )
        .map_err(|error| error.to_string())?;
        let before = semantic::capture_current(&home).map_err(|error| error.to_string())?;
        let after =
            semantic::capture_repository(ir, repository).map_err(|error| error.to_string())?;
        let changes = semantic::compare(&before, &after);
        let operations =
            semantic::operations::explain(&operations).map_err(|error| error.to_string())?;
        let complete = changes.is_complete()
            && operations.iter().all(|operation| {
                operation.configuration_unknown.is_none()
                    && operation
                        .configuration
                        .as_ref()
                        .is_none_or(|view| view.complete)
            });
        let report = PlanReport {
            version: 1,
            selected_profiles: selected,
            complete,
            execution_authority: false,
            comparison_scope: "workspace_declarations",
            changes: &changes,
            operations: &operations,
        };
        emit(
            &report,
            json,
            "Shared deployment preview; unknown content is not pre-reviewed.",
        )?;
        // Preserve plan's existing success semantics. Incompleteness is a typed
        // report field; only the explicit diff commands use change exit codes.
        Ok(ExitCode::SUCCESS)
    })())
}

pub fn config(
    before: &Path,
    after: &Path,
    format: &str,
    local_values: bool,
    json: bool,
) -> ExitCode {
    use semantic::config_view::{ConfigFormat, ConfigScope, ConfigViewOptions, compare_config};
    finish((|| {
        let format = match format {
            "json" => ConfigFormat::Json,
            "toml" => ConfigFormat::Toml,
            "bytes" => ConfigFormat::Unsupported,
            _ => return Err("configuration format must be json, toml, or bytes".into()),
        };
        let before = read_bounded(before)?;
        let after = read_bounded(after)?;
        let options = if local_values {
            ConfigViewOptions::local()
        } else {
            ConfigViewOptions::default()
        };
        let view = compare_config(&before, &after, format, ConfigScope::WholeFile, &options);
        emit(
            &view,
            json,
            "Configuration explanation only; deployed bytes are not rewritten.",
        )?;
        let status = if !view.complete {
            INCOMPLETE
        } else if view.exact_bytes_equal == Some(true) {
            0
        } else {
            CHANGED
        };
        Ok(ExitCode::from(status))
    })())
}

fn select(selector: &str, repo: &Path) -> Result<SemanticSnapshot, String> {
    let home = gripsack_store::gripsack_home();
    if selector == "current" {
        return semantic::capture_current(&home).map_err(|error| error.to_string());
    }
    if let Some(number) = selector.strip_prefix("generation:") {
        let generation = number
            .parse()
            .map_err(|_| "invalid generation number".to_owned())?;
        return semantic::capture_generation(&home, generation).map_err(|error| error.to_string());
    }
    if let Some(path) = selector.strip_prefix("proposed:") {
        let bytes = read_bounded(Path::new(path))?;
        semantic::admission::parse_json(&bytes).map_err(|error| error.to_string())?;
        let text =
            std::str::from_utf8(&bytes).map_err(|_| "proposed IR is not UTF-8".to_owned())?;
        let ir = gripsack_ir::check(text).map_err(|diagnostics| {
            diagnostics
                .iter()
                .map(|diagnostic| format!("{}: {}", diagnostic.code, diagnostic.message))
                .collect::<Vec<_>>()
                .join("; ")
        })?;
        return semantic::capture_proposed(&ir, repo).map_err(|error| error.to_string());
    }
    let path = selector.strip_prefix("snapshot:").unwrap_or(selector);
    semantic::read_snapshot(Path::new(path)).map_err(|error| error.to_string())
}

fn read_bounded(path: &Path) -> Result<Vec<u8>, String> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .map_err(|error| format!("cannot open review input: {error}"))?;
    if !file
        .metadata()
        .map_err(|error| error.to_string())?
        .is_file()
    {
        return Err("review input must be a regular file".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_REVIEW_INPUT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read review input: {error}"))?;
    if bytes.len() as u64 > MAX_REVIEW_INPUT_BYTES {
        return Err("review input exceeds the 16 MiB byte limit".into());
    }
    Ok(bytes)
}

fn emit(value: &impl serde::Serialize, json: bool, heading: &str) -> Result<(), String> {
    let serialized = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    if !json {
        println!("{heading}");
    }
    // JSON escapes C0 controls even in the terminal view. Escape Unicode format
    // controls as well: bidi overrides must not reorder diagnostic authority.
    println!("{}", terminal_safe(&serialized));
    Ok(())
}

fn terminal_safe(text: &str) -> String {
    let mut safe = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '\u{061c}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}' => safe.push_str(&format!("\\u{:04x}", character as u32)),
            character if character.is_control() && character != '\n' && character != '\t' => {
                safe.push_str(&format!("\\u{:04x}", character as u32));
            }
            _ => safe.push(character),
        }
    }
    safe
}

fn finish(result: Result<ExitCode, String>) -> ExitCode {
    match result {
        Ok(code) => code,
        Err(error) => {
            eprintln!(
                "grip: review: {}",
                terminal_safe(&error)
                    .replace('\n', "\\n")
                    .replace('\t', "\\t")
            );
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::terminal_safe;

    #[test]
    fn review_neutralizes_terminal_and_bidi_controls() {
        let text = terminal_safe("bad\u{1b}[2J\u{202e}authority");
        assert!(!text.contains('\u{1b}'));
        assert!(!text.contains('\u{202e}'));
        assert!(text.contains("\\u202e"));
    }

    #[test]
    fn control_escaping_preserves_json_encoding() {
        let serialized = serde_json::to_string("next\u{0085}\u{202e}").unwrap();
        let safe = terminal_safe(&serialized);
        assert_eq!(
            serde_json::from_str::<String>(&safe).unwrap(),
            "next\u{0085}\u{202e}"
        );
    }
}
