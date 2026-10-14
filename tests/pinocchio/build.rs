fn main() {
    println!("cargo:rerun-if-env-changed=SOLEX_CHANNEL");
    println!("cargo:rerun-if-env-changed=SOLEX_MANIFEST");
    println!("cargo:rerun-if-changed=Solex.toml");
}
