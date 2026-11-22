fn main() {
    let root = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("..");
    let output =
        std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("catalogs.json");
    println!("cargo:rerun-if-changed=../po");
    println!("cargo:rerun-if-changed=../scripts/build-workspace-catalogs.py");
    let status = std::process::Command::new("python3")
        .arg(root.join("scripts/build-workspace-catalogs.py"))
        .arg(&output)
        .status();
    assert!(
        status.is_ok_and(|s| s.success()),
        "Install python3 and gettext (msgfmt) to build the desktop catalogs."
    );
    tauri_build::build()
}
