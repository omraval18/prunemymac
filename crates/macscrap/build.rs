//! Embed every SVG in assets/icons so the app ships as one binary.
use std::fmt::Write;

fn main() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/icons");
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .filter_map(|e| e.file_name().to_str().filter(|n| n.ends_with(".svg")).map(str::to_string))
        .collect();
    names.sort();
    let mut out = String::from("pub const ICONS: &[(&str, &[u8])] = &[\n");
    for n in names {
        writeln!(out, "    (\"icons/{n}\", include_bytes!(\"{}\")),", dir.join(&n).display()).unwrap();
    }
    out.push_str("];\n");
    std::fs::write(std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("icons.rs"), out).unwrap();
}
