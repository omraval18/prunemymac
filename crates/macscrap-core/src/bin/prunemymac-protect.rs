//! Headless Protection / YARA helper. Kept as a separate binary so the GUI
//! does not have to link cranelift/wasmtime from yara-x.
fn main() {
    macscrap_core::sys::background_priority();
    macscrap_core::malware::helper_main();
}
