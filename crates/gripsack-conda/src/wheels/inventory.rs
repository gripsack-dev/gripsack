use super::*;
use astral_ini::{Delimiters, Options, Section};

/// Exact expected regular file, relative to the environment root.
#[derive(Debug)]
pub struct ExpectedWheelFile {
    path: PathBuf,
    bytes: Arc<Vec<u8>>,
    mode: u32,
    generated: bool,
}
impl ExpectedWheelFile {
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn mode(&self) -> u32 {
        self.mode
    }
    pub fn is_generated(&self) -> bool {
        self.generated
    }
}
impl WheelArchive {
    pub fn installation_inventory(
        &self,
        python_version: &str,
        final_prefix: &Path,
    ) -> Result<Vec<ExpectedWheelFile>, WheelError> {
        let (major, minor) = python_parts(python_version)?;
        self.installation_inventory_at(
            python_version,
            final_prefix,
            &PathBuf::from(format!("lib/python{major}.{minor}/site-packages")),
        )
    }
    pub fn installation_inventory_at(
        &self,
        python_version: &str,
        final_prefix: &Path,
        site_packages: &Path,
    ) -> Result<Vec<ExpectedWheelFile>, WheelError> {
        let (_, minor) = python_parts(python_version)?;
        let site = site_packages
            .to_str()
            .ok_or_else(|| invalid("non-UTF8 site-packages path"))?;
        safe_path(site)?;
        if !final_prefix.is_absolute()
            || final_prefix
                .to_str()
                .is_none_or(|s| s.chars().any(|c| c.is_control()))
        {
            return Err(invalid("invalid final Python prefix"));
        }
        let mut output = BTreeMap::<String, ExpectedWheelFile>::new();
        let mut record = Vec::<(String, String, String)>::new();
        let record_name = format!("{}/RECORD", self.dist);
        let data = format!("{}/", self.dist.replace(".dist-info", ".data"));
        for (name, member) in &self.files {
            if suppressed(name) {
                continue;
            }
            let destination = if let Some(rest) = name.strip_prefix(&data) {
                let (scheme, relative) = rest
                    .split_once('/')
                    .ok_or_else(|| invalid("empty wheel data scheme"))?;
                match scheme {
                    "purelib" | "platlib" => site_packages.join(relative),
                    "headers" => Path::new("include")
                        .join(normalize(&self.metadata.name))
                        .join(relative),
                    "data" => {
                        let lower = relative.to_ascii_lowercase();
                        if lower == "bin"
                            || lower == "conda-meta"
                            || lower.starts_with("bin/")
                            || lower.starts_with("conda-meta/")
                        {
                            return Err(invalid(
                                "wheel data claims executable or package receipt namespace",
                            ));
                        }
                        PathBuf::from(relative)
                    }
                    _ => return Err(invalid("unsupported wheel data scheme")),
                }
            } else {
                site_packages.join(name)
            };
            for component in destination.components() {
                let component = component.as_os_str().to_string_lossy();
                if (component.ends_with(".dist-info") && component != self.dist)
                    || component.ends_with(".egg-info")
                {
                    return Err(invalid("wheel claims foreign distribution metadata"));
                }
            }
            let relative = relative_to(&destination, site_packages);
            record.push(if name == &record_name {
                (relative, String::new(), String::new())
            } else {
                (
                    relative,
                    hash(&member.bytes),
                    member.bytes.len().to_string(),
                )
            });
            insert(
                &mut output,
                destination,
                member.bytes.clone(),
                member.mode,
                false,
            )?;
        }
        if let Some(entrypoints) = self.files.get(&format!("{}/entry_points.txt", self.dist)) {
            let text = std::str::from_utf8(&entrypoints.bytes).map_err(invalid)?;
            let ini = Options::default()
                .case_sensitive(true)
                .delimiters(Delimiters::Equals)
                .parse(text)
                .map_err(invalid)?;
            let mut console = Vec::<(String, String)>::new();
            for (name, value) in ini
                .section("console_scripts")
                .into_iter()
                .flat_map(Section::iter)
            {
                if name
                    .strip_prefix("pip3.")
                    .is_some_and(|n| n.parse::<u8>().is_ok())
                {
                    continue;
                }
                console.push((name.to_owned(), value.to_owned()));
            }
            if let Some((_, value)) = console.iter().find(|(n, _)| n == "pip3") {
                console.push((format!("pip3.{minor}"), value.clone()));
            }
            if ini
                .section("gui_scripts")
                .into_iter()
                .flat_map(Section::iter)
                .next()
                .is_some()
            {
                return Err(invalid(
                    "GUI entrypoints are not supported; only standard console scripts are admitted",
                ));
            }
            for (name, value) in console {
                safe_path(&name)?;
                if name.contains('/') {
                    return Err(invalid("entrypoint name contains directory"));
                }
                if reserved_script_name(&name.to_ascii_lowercase()) {
                    return Err(invalid(
                        "entrypoint claims a reserved Python interpreter name",
                    ));
                }
                let value = entrypoint_reference(&value)?;
                let (module, function) = value
                    .split_once(':')
                    .ok_or_else(|| invalid("invalid entrypoint reference"))?;
                let (module, function) = (module.trim(), function.trim());
                if !identifier(module) || !identifier(function) {
                    return Err(invalid("unsafe entrypoint reference"));
                }
                let executable = final_prefix.join("bin/python");
                let executable = executable
                    .to_str()
                    .ok_or_else(|| invalid("non-UTF8 interpreter path"))?;
                let shebang = if executable.len() + 3 > 127 || executable.contains(' ') {
                    format!(
                        "#!/bin/sh\n'''exec' '{}' \"$0\" \"$@\"\n' '''",
                        uv_shell::escape_posix_for_single_quotes(executable)
                    )
                } else {
                    format!("#!{executable}")
                };
                let import_name = function.split('.').next().unwrap();
                // This is an independent expectation for uv-install-wheel 0.0.98, not an installer.
                let bytes = format!("{shebang}\n# -*- coding: utf-8 -*-\nimport sys\nfrom {module} import {import_name}\nif __name__ == \"__main__\":\n    if sys.argv[0].endswith(\"-script.pyw\"):\n        sys.argv[0] = sys.argv[0][:-11]\n    elif sys.argv[0].endswith(\".exe\"):\n        sys.argv[0] = sys.argv[0][:-4]\n    sys.exit({function}())\n").into_bytes();
                let destination = Path::new("bin").join(name);
                record.push((
                    relative_to(&destination, site_packages),
                    hash(&bytes),
                    bytes.len().to_string(),
                ));
                insert(&mut output, destination, Arc::new(bytes), 0o755, true)?;
            }
        }
        record.sort();
        let mut writer = csv::WriterBuilder::new()
            .has_headers(false)
            .escape(b'"')
            .from_writer(Vec::new());
        for row in record {
            writer.serialize(row).map_err(invalid)?;
        }
        let bytes = writer.into_inner().map_err(invalid)?;
        let installed_record = output
            .get_mut(
                &site_packages
                    .join(record_name)
                    .to_string_lossy()
                    .to_ascii_lowercase(),
            )
            .ok_or_else(|| invalid("missing installed RECORD"))?;
        installed_record.bytes = Arc::new(bytes);
        installed_record.generated = true;
        Ok(output.into_values().collect())
    }
}
fn insert(
    files: &mut BTreeMap<String, ExpectedWheelFile>,
    path: PathBuf,
    bytes: Arc<Vec<u8>>,
    mode: u32,
    generated: bool,
) -> Result<(), WheelError> {
    let name = path
        .to_str()
        .ok_or_else(|| invalid("non-UTF8 installed path"))?;
    safe_path(name)?;
    reject_startup(name)?;
    let key = name.to_ascii_lowercase();
    let descendants = key.clone() + "/";
    if files.contains_key(&key)
        || files
            .range(descendants.clone()..)
            .next()
            .is_some_and(|(next, _)| next.starts_with(&descendants))
    {
        return Err(invalid("installed wheel path collision"));
    }
    for (offset, _) in key.match_indices('/') {
        if files.contains_key(&key[..offset]) {
            return Err(invalid("installed wheel file/directory collision"));
        }
        let prefix = key[..offset].to_owned() + "/";
        if let Some((existing, file)) = files.range(prefix.clone()..).next()
            && existing.starts_with(&prefix)
            && file
                .path
                .to_str()
                .is_some_and(|p| p[..offset] != name[..offset])
        {
            return Err(invalid("installed wheel component case collision"));
        }
    }
    files.insert(
        key,
        ExpectedWheelFile {
            path,
            bytes,
            mode,
            generated,
        },
    );
    Ok(())
}
fn relative_to(path: &Path, base: &Path) -> String {
    let path: Vec<_> = path.components().collect();
    let base: Vec<_> = base.components().collect();
    let shared = path.iter().zip(&base).take_while(|(a, b)| a == b).count();
    let mut result = vec!["..".to_owned(); base.len() - shared];
    result.extend(
        path[shared..]
            .iter()
            .map(|p| p.as_os_str().to_string_lossy().into_owned()),
    );
    result.join("/")
}
fn identifier(value: &str) -> bool {
    value.split('.').all(|part| {
        !part.is_empty()
            && part
                .bytes()
                .enumerate()
                .all(|(i, b)| b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit()))
    })
}
fn python_parts(version: &str) -> Result<(u8, u8), WheelError> {
    let mut parts = version.split('.');
    let major = parts
        .next()
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| invalid("invalid Python major"))?;
    let minor = parts
        .next()
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| invalid("invalid Python minor"))?;
    if major != 3 {
        return Err(invalid("only CPython 3 wheels are supported"));
    }
    Ok((major, minor))
}
fn entrypoint_reference(value: &str) -> Result<&str, WheelError> {
    let value = value.trim();
    let Some((reference, extras)) = value.split_once('[') else {
        return Ok(value);
    };
    let extras = extras
        .strip_suffix(']')
        .ok_or_else(|| invalid("unterminated entrypoint extras"))?;
    if !extras.split(',').all(|extra| {
        let extra = extra.trim();
        !extra.is_empty()
            && extra
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(&byte))
    }) {
        return Err(invalid("invalid entrypoint extras"));
    }
    Ok(reference.trim_end())
}
fn reserved_script_name(name: &str) -> bool {
    let name = name.strip_suffix(".py").unwrap_or(name);
    [
        "python", "pythonw", "python3", "graalpy", "pypy", "pypy2", "pypy3",
    ]
    .contains(&name)
        || name
            .strip_prefix("python3.")
            .is_some_and(|minor| minor.parse::<u8>().is_ok())
        || ["python3.", "pythonw3."].iter().any(|prefix| {
            name.strip_prefix(prefix)
                .and_then(|minor| minor.strip_suffix('t'))
                .is_some_and(|minor| minor.parse::<u8>().is_ok())
        })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn entrypoint_extras_are_fully_validated() {
        assert_eq!(
            entrypoint_reference("demo:main [cli, extra-2]").unwrap(),
            "demo:main"
        );
        for reference in [
            "demo:main []",
            "demo:main [cli",
            "demo:main [cli] ignored",
            "demo:main [cli,,other]",
        ] {
            assert!(entrypoint_reference(reference).is_err(), "{reference}");
        }
    }
    #[test]
    fn interpreter_entrypoints_are_reserved() {
        for name in [
            "python",
            "python3",
            "python3.12",
            "python3.14t",
            "pythonw3.14t",
            "python.py",
            "pypy3",
        ] {
            assert!(reserved_script_name(name), "{name}");
        }
        assert!(!reserved_script_name("demo"));
    }
}
