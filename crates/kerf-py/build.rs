// PyO3 extension modules resolve CPython symbols at runtime, not link time. On macOS a plain
// `cargo build` of this cdylib needs `-undefined dynamic_lookup`; scope it here to kerf-py only so a
// normal binary (e.g. kerf-cli) is not built with error-masking linker flags. maturin injects the
// same flag when building the wheel, so this is idempotent under `uv sync` / `maturin build`.
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg=-undefined");
        println!("cargo:rustc-link-arg=dynamic_lookup");
    }
}
