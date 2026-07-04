// PyO3 extension modules resolve CPython symbols at runtime, not link time. On macOS this cdylib
// needs `-undefined dynamic_lookup`; scope it to kerf-py so other binaries are not built with
// error-masking linker flags.
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg=-undefined");
        println!("cargo:rustc-link-arg=dynamic_lookup");
    }
}
