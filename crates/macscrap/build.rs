//! Embed UI assets so the app ships as one binary.
use std::fmt::Write;

fn main() {
    let assets = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
    let mut out = String::from("pub const ICONS: &[(&str, &[u8])] = &[\n");
    let mut files = Vec::new();
    for folder in ["icons", "images"] {
        let dir = assets.join(folder);
        println!("cargo:rerun-if-changed={}", dir.display());
        files.extend(std::fs::read_dir(dir).unwrap().flatten().filter_map(|entry| {
            let path = entry.path();
            path.is_file().then_some(path)
        }));
    }
    files.sort();
    for path in files {
        let relative = path.strip_prefix(&assets).unwrap().to_string_lossy().replace('\\', "/");
        writeln!(out, "    (\"{relative}\", include_bytes!(\"{}\")),", path.display()).unwrap();
    }
    out.push_str("];\n");
    std::fs::write(std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("icons.rs"), out).unwrap();
}
