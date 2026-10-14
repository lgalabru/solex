fn main() {
    // Solex macros read the channel at expansion time; rebuild when it changes.
    println!("cargo:rerun-if-env-changed=SOLEX_CHANNEL");
    println!("cargo:rerun-if-env-changed=SOLEX_MANIFEST");
    println!("cargo:rerun-if-changed=Solex.yaml");
}
