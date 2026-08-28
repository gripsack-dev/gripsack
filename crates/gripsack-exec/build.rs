//! Embed the TypeScript frontend into the binary (plan/0013 D3): the
//! `typescript/src` tree (driver + @gripsack/core, relative imports
//! only) plus `typescript/deno.json` (the frontend's import map, when
//! the tree ships one) become FRONTEND_FILES — (materialized rel
//! path, source file) pairs. The src tree lands nested (`src/…`,
//! `deno.json` at the root) so the materialized layout mirrors
//! `typescript/` itself and the import map resolves identically in
//! both places. Generated here so crates.io builds — which don't
//! have the repo's typescript tree — fall back to an empty table and
//! eval fails with "no embedded frontend", unchanged from the old
//! python-embed behavior.

use std::path::{Path, PathBuf};

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let ts = manifest.join("../../typescript");
    let src = ts.join("src");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let mut entries: Vec<(String, PathBuf)> = Vec::new();

    fn walk(dir: &Path, base: &Path, out: &mut Vec<(String, PathBuf)>) {
        let Ok(read) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in read.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, base, out);
            } else if path.extension().is_some_and(|x| x == "ts") {
                let rel = path
                    .strip_prefix(base)
                    .expect("walked under base")
                    .to_string_lossy()
                    .into_owned();
                out.push((format!("src/{rel}"), path));
            }
        }
    }

    if src.is_dir() {
        println!("cargo:rerun-if-changed={}", src.display());
        walk(&src, &src, &mut entries);
    }
    // the frontend's deno.json (import map + tasks) is the package
    // metadata — optional, embedded at the materialized root
    let deno_json = ts.join("deno.json");
    if deno_json.is_file() {
        println!("cargo:rerun-if-changed={}", deno_json.display());
        entries.push(("deno.json".into(), deno_json));
    }
    entries.sort();

    // flat copies under OUT_DIR + include_str! — the embedded content
    // is bytes in the binary, not paths
    let flat_dir = out.join("frontend_src");
    std::fs::create_dir_all(&flat_dir).unwrap();
    let mut out_rs = String::from(
        "/// (path inside the materialized frontend dir, source file) —\n\
         /// empty on crates.io builds.\n\
         pub static FRONTEND_FILES: &[(&str, &str)] = &[\n",
    );
    for (i, (rel, path)) in entries.iter().enumerate() {
        let flat = flat_dir.join(i.to_string());
        std::fs::copy(path, &flat).unwrap();
        out_rs.push_str(&format!(
            "    ({rel:?}, include_str!(\"{}\")),\n",
            flat.display()
        ));
    }
    out_rs.push_str("];\n");
    std::fs::write(out.join("frontend_files.rs"), out_rs).unwrap();
}
