//! Strict, one-time deserialization and semantic admission of pack data.
//! Configuration key names are data, not metadata: do not reserve an
//! underscore prefix or infer a tool's vocabulary here.

use crate::{FileTable, Format, Meta, Pack, Rule, RuleValue, SectionRules, ValueType};
use serde::{Deserialize, Deserializer, de::Error as _};
use std::collections::BTreeMap;

fn nonempty(value: &str, name: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("{name} must not be empty"))
    } else {
        Ok(())
    }
}

fn names(values: &[String], name: &str, required: bool) -> Result<(), String> {
    if required && values.is_empty() {
        return Err(format!("{name} must not be empty"));
    }
    for value in values {
        nonempty(value, name)?;
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawMeta {
    tool: String,
    handles: Vec<String>,
    format: Format,
    supported: Vec<String>,
    #[serde(default)]
    lenient: Vec<String>,
    series: String,
    #[serde(default)]
    coverage_warning: Option<String>,
}

impl TryFrom<RawMeta> for Meta {
    type Error = String;

    fn try_from(raw: RawMeta) -> Result<Self, String> {
        nonempty(&raw.tool, "meta.tool")?;
        nonempty(&raw.series, "meta.series")?;
        names(&raw.handles, "meta.handles", true)?;
        names(&raw.supported, "meta.supported", true)?;
        names(&raw.lenient, "meta.lenient", false)?;
        for handle in &raw.handles {
            if handle.contains('/') || handle.contains('\\') || handle == "." || handle == ".." {
                return Err(format!("handled file {handle:?} must be a basename"));
            }
        }
        for name in &raw.lenient {
            if !raw.handles.contains(name) {
                return Err(format!("lenient file {name:?} is not in meta.handles"));
            }
        }
        // Leniency is a dispatch policy, not a parser option: it is
        // meaningful for shared files in any of the three formats.
        // Do not infer format from extensions (e.g. extensionless YAML).
        if let Some(warning) = &raw.coverage_warning {
            nonempty(warning, "meta.coverage_warning")?;
        }
        Ok(Self {
            tool: raw.tool,
            handles: raw.handles,
            format: raw.format,
            supported: raw.supported,
            lenient: raw.lenient,
            series: raw.series,
            coverage_warning: raw.coverage_warning,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawRule {
    types: Vec<ValueType>,
    #[serde(default)]
    choices: Option<Vec<serde_json::Value>>,
    #[serde(default)]
    deprecated: Option<String>,
}

impl TryFrom<RawRule> for Rule {
    type Error = String;

    fn try_from(raw: RawRule) -> Result<Self, String> {
        if raw.types.is_empty() {
            return Err("rule.types must not be empty".into());
        }
        if let Some(replacement) = &raw.deprecated {
            nonempty(replacement, "rule.deprecated")?;
        }
        // An empty choice list is a rule no value can ever satisfy —
        // every lint against it would be an unavoidable A05.
        if raw.choices.as_ref().is_some_and(Vec::is_empty) {
            return Err("rule.choices must not be empty".into());
        }
        Ok(Self {
            types: raw.types,
            choices: raw.choices,
            deprecated: raw.deprecated,
        })
    }
}

impl<'de> Deserialize<'de> for RuleValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = toml::Value::deserialize(deserializer)?;
        match value {
            toml::Value::String(s) if s == "free" => Ok(Self::Free),
            toml::Value::Table(_) => value
                .try_into::<Rule>()
                .map(Self::Rule)
                .map_err(D::Error::custom),
            _ => Err(D::Error::custom(
                "key rule must be exactly \"free\" or a rule table",
            )),
        }
    }
}

impl<'de> Deserialize<'de> for SectionRules {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut map = BTreeMap::<String, toml::Value>::deserialize(deserializer)?;
        if map.contains_key("_free") || map.contains_key("_rule") {
            if map.len() != 1 {
                return Err(D::Error::custom(
                    "section marker _free or _rule must be exclusive",
                ));
            }
            if let Some(value) = map.remove("_free") {
                return match value {
                    toml::Value::Boolean(true) => Ok(Self::Free),
                    _ => Err(D::Error::custom("section _free must be true")),
                };
            }
            let value = map.remove("_rule").expect("reserved marker exists");
            return value
                .try_into::<Rule>()
                .map(Self::WholeRule)
                .map_err(D::Error::custom);
        }
        let mut keys = BTreeMap::new();
        for (key, value) in map {
            nonempty(&key, "rule key").map_err(D::Error::custom)?;
            let rule = value
                .try_into::<RuleValue>()
                .map_err(|e| D::Error::custom(format!("key {key:?}: {e}")))?;
            keys.insert(key, rule);
        }
        Ok(Self::Keys(keys))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawFileTable {
    #[serde(default)]
    no_table: bool,
    #[serde(default)]
    rules: BTreeMap<String, SectionRules>,
}

impl TryFrom<RawFileTable> for FileTable {
    type Error = String;

    fn try_from(raw: RawFileTable) -> Result<Self, String> {
        if raw.no_table && !raw.rules.is_empty() {
            return Err("no_table = true cannot coexist with rules".into());
        }
        if !raw.no_table && raw.rules.is_empty() {
            return Err("file must provide rules or explicitly set no_table = true".into());
        }
        for section in raw.rules.keys() {
            // The empty section denotes the root. Dots and other tool
            // punctuation are deliberately not restricted.
            if !section.is_empty() {
                nonempty(section, "section name")?;
            }
        }
        Ok(Self {
            no_table: raw.no_table,
            rules: raw.rules,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawPack {
    meta: Meta,
    files: BTreeMap<String, FileTable>,
}

impl TryFrom<RawPack> for Pack {
    type Error = String;

    fn try_from(raw: RawPack) -> Result<Self, String> {
        for handle in &raw.meta.handles {
            if !raw.files.contains_key(handle) {
                return Err(format!("handled file {handle:?} has no file table"));
            }
        }
        for name in raw.files.keys() {
            if !raw.meta.handles.contains(name) {
                return Err(format!("file table {name:?} is not in meta.handles"));
            }
        }
        Ok(Self {
            meta: raw.meta,
            files: raw.files,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::{Format, PACKS, Pack, checks::lint_file, load_pack, load_pack_str};

    fn source(rules: &str) -> String {
        format!(
            r#"[meta]
tool = "test"
handles = ["config"]
format = "toml"
supported = ["1."]
series = "1"
[files.config.rules.""]
{rules}
"#
        )
    }

    fn reject(text: &str) {
        assert!(
            load_pack_str(text).is_err(),
            "admitted malformed pack: {text}"
        );
        assert!(
            toml::from_str::<Pack>(text).is_err(),
            "direct deserialization bypassed admission"
        );
    }

    #[test]
    fn malformed_key_rules_never_turn_sections_free() {
        for rule in [
            "x = { types = ['string'], choices = 'x' }",
            "x = { types = ['string'], choices = [] }",
            "x = { types = ['string', 'null'] }",
            "x = { types = 'string' }",
            "x = { choices = ['x'] }",
            "x = { types = ['string'], typo = true }",
            "x = { types = ['string'], deprecated = '' }",
            "x = { types = ['string'], choices = 'x' }",
            "_fre = true",
            "no_table = true",
        ] {
            reject(&source(rule));
        }
    }

    #[test]
    fn markers_are_typed_and_exclusive() {
        for rule in [
            "_free = false",
            "_free = 'true'",
            "_free = 'free'",
            "_free = {}",
            "_rule = 'free'",
            "_rule = true",
            "_rule = { types = [] }",
            "_rule = { types = ['tablle'] }",
            "_rule = { types = ['table'], extra = true }",
            "_free = true\nx = 'free'",
            "_rule = { types = ['table'] }\nx = 'free'",
            "_free = true\n_rule = { types = ['table'] }",
        ] {
            reject(&source(rule));
        }
    }

    #[test]
    fn metadata_and_dispatch_must_be_actionable() {
        let valid = source("x = 'free'");
        for (from, to) in [
            ("tool = \"test\"", "tool = ''"),
            ("series = \"1\"", "series = ' '"),
            ("supported = [\"1.\"]", "supported = []"),
            ("supported = [\"1.\"]", "supported = ['']"),
            ("handles = [\"config\"]", "handles = []"),
            ("handles = [\"config\"]", "handles = ['other']"),
            ("handles = [\"config\"]", "handles = ['dir/config']"),
            ("format = \"toml\"", "format = 'xml'"),
            ("format = \"toml\"", "format = 'TOML'"),
            ("series = \"1\"", "series = '1'\nlenient = ['other']"),
            ("series = \"1\"", "series = '1'\nformatt = 'toml'"),
        ] {
            reject(&valid.replace(from, to));
        }
        reject(&format!("unknown = true\n{valid}"));
        reject(&valid.replace(
            "[files.config.rules.\"\"]",
            "[files.config]\nunknown = true\n[files.config.rules.\"\"]",
        ));
        reject(&valid.replace(
            "[files.config.rules.\"\"]",
            "[files.config]\nno_table = true\n[files.config.rules.\"\"]",
        ));
        reject(&valid.replace("[files.config.rules.\"\"]\nx = 'free'", "[files.config]"));
    }

    #[test]
    fn file_loader_has_the_same_admission_boundary() {
        let path =
            std::env::temp_dir().join(format!("griplint-admission-{}.toml", std::process::id()));
        std::fs::write(&path, source("x = 'fre'")).unwrap();
        let result = load_pack(&path);
        std::fs::remove_file(path).unwrap();
        assert!(result.is_err());
    }

    #[test]
    fn free_keys_and_sections_preserve_arbitrary_descendants() {
        let text = source("_ = 'free'\n_private = 'free'\nx = { types = ['integer'] }")
            + "[files.config.rules.open]\n_free = true\n";
        let pack = load_pack_str(&text).unwrap();
        let diagnostics = lint_file(
            &pack,
            "config",
            "_ = { anything = [1, 2] }\n_private = false\nx = true\n[open.deep]\nunknown = 42\n",
            None,
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, "A04");
        assert_eq!(diagnostics[0].message, "'x' must be integer, got boolean");
        let free = load_pack_str(&source("_free = true")).unwrap();
        assert!(lint_file(&free, "config", "[anything.deep]\nx = 1", None).is_empty());
    }

    #[test]
    fn whole_rules_check_sections_without_checking_their_members() {
        let text = source("x = 'free'")
            + "[files.config.rules.whole]\n_rule = { types = ['table'] }\n"
            + "[files.config.rules.\"parent.child\"]\n_rule = { types = ['table'] }\n";
        let pack = load_pack_str(&text).unwrap();
        assert!(
            lint_file(
                &pack,
                "config",
                "[whole]\narbitrary = 1\n[parent.child]\nanything = false",
                None
            )
            .is_empty()
        );
        for input in ["whole = 1", "[[whole]]\nx = 1", "[parent]\nchild = false"] {
            let diagnostics = lint_file(&pack, "config", input, None);
            assert_eq!(diagnostics.len(), 1, "{input}");
            assert_eq!(diagnostics[0].code, "A04");
        }
    }

    #[test]
    fn key_diagnostics_and_leniency_survive_admission() {
        let text = source(
            "x = { types = ['string'], choices = ['yes'] }\nold = { types = ['integer'], deprecated = 'new' }",
        );
        let pack = load_pack_str(&text).unwrap();
        let diagnostics = lint_file(&pack, "config", "x = 'no'\nold = 1\nunknown = true", None);
        assert_eq!(
            diagnostics.iter().map(|d| &*d.code).collect::<Vec<_>>(),
            ["A05", "A03", "A01"]
        );
        let lenient =
            load_pack_str(&text.replace("series = \"1\"", "series = '1'\nlenient = ['config']"))
                .unwrap();
        let diagnostics = lint_file(&lenient, "config", "x = 1\nunknown = true", None);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, "A04");
    }

    #[test]
    fn formats_dispatch_without_extension_assumptions() {
        for (format, input, expected) in [
            ("toml", "x = true", Format::Toml),
            ("yaml", "x: true", Format::Yaml),
            ("json", "{\"x\":true}", Format::Json),
        ] {
            let text = source("x = { types = ['integer'] }")
                .replace("format = \"toml\"", &format!("format = '{format}'"));
            let pack = load_pack_str(&text).unwrap();
            assert_eq!(pack.meta.format, expected);
            let diagnostics = lint_file(&pack, "config", input, None);
            assert_eq!(diagnostics.len(), 1);
            assert_eq!(diagnostics[0].message, "'x' must be integer, got boolean");
        }
    }

    #[test]
    fn explicit_no_table_preserves_coverage_without_parsing() {
        let text = source("x = 'free'").replace(
            "[files.config.rules.\"\"]\nx = 'free'",
            "[files.config]\nno_table = true",
        );
        let pack = load_pack_str(&text).unwrap();
        assert!(lint_file(&pack, "config", "invalid [[[", None).is_empty());
        let diagnostics = lint_file(&pack, "config", "invalid [[[", Some("2.0"));
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, "W10");
    }

    #[test]
    fn every_embedded_pack_is_admitted() {
        assert_eq!(PACKS.len(), 23);
        for (name, text) in PACKS {
            let pack = load_pack_str(text).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(&pack.meta.tool, name);
            for handle in &pack.meta.handles {
                assert!(pack.files.contains_key(handle), "{name}: {handle}");
            }
        }
    }
}
