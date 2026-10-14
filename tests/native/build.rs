fn main() {
    // The channel is read by proc macros at expansion time; make cargo
    // re-expand them when it changes.
    println!("cargo:rerun-if-env-changed=SOLEX_CHANNEL");
    println!("cargo:rerun-if-env-changed=SOLEX_MANIFEST");
    println!("cargo:rerun-if-env-changed=SOLEX_CLUSTER");
    println!("cargo:rerun-if-changed=Solex.toml");
}
