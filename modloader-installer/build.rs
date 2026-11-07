fn main() {
    // Without a manifest, Windows' installer detection runs 32-bit programs with "install" in
    // their name elevated, behind a UAC prompt. This program only needs normal rights.
    println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg-bins=/MANIFESTUAC:level='asInvoker'");
}
