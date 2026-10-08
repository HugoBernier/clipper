//! Compile l'icône et les informations de version (lues dans Cargo.toml) dans l'exe.

fn main() {
    let v = |name: &str| std::env::var(name).unwrap_or_else(|_| "0".into());
    let macros = [
        format!("VER_MAJOR={}", v("CARGO_PKG_VERSION_MAJOR")),
        format!("VER_MINOR={}", v("CARGO_PKG_VERSION_MINOR")),
        format!("VER_PATCH={}", v("CARGO_PKG_VERSION_PATCH")),
        format!("VER_STR=\"{}\"", v("CARGO_PKG_VERSION")),
    ];
    println!("cargo:rerun-if-changed=assets/clipper.rc");
    println!("cargo:rerun-if-changed=assets/clipper.ico");
    embed_resource::compile("assets/clipper.rc", macros)
        .manifest_optional()
        .expect("compilation des ressources (rc.exe du SDK Windows)");
}
