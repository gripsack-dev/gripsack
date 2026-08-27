//! Embed the Python frontend into the binary (the zero-provisioning
//! bootstrap): `python/gripsack/*.py` becomes FRONTEND_FILES, generated
//! here so crates.io builds — which don't have the repo's python tree —
//! fall back to an empty table and the provisioning path, unchanged.

use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let src = manifest.join("../../python/gripsack");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let mut entries = Vec::new();
    if src.is_dir() {
        println!("cargo:rerun-if-changed={}", src.display());
        let mut files: Vec<PathBuf> = std::fs::read_dir(&src)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "py"))
            .collect();
        files.sort();
        for f in files {
            let name = f.file_name().unwrap().to_str().unwrap().to_string();
            let flat = out.join(format!("frontend_{name}"));
            std::fs::copy(&f, &flat).unwrap();
            entries.push((name, flat));
        }
    }
    let mut out_rs = String::from(
        "/// (path inside the archive, source file) — empty on crates.io builds.\n\
         pub static FRONTEND_FILES: &[(&str, &str)] = &[\n",
    );
    for (name, flat) in &entries {
        out_rs.push_str(&format!(
            "    (\"gripsack/{name}\", include_str!(\"{}\")),\n",
            flat.display()
        ));
    }
    out_rs.push_str("];\n");
    std::fs::write(out.join("frontend_files.rs"), out_rs).unwrap();
}
