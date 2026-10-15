fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=assets/app.manifest");

    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "windows" {
        return;
    }

    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_env != "msvc" {
        panic!("Windows builds require the MSVC target so the administrator manifest can be embedded only in the application binary");
    }

    let manifest = std::path::Path::new("assets/app.manifest");
    if !manifest.is_file() {
        panic!(
            "required Windows application manifest is missing: {}",
            manifest.display()
        );
    }

    let mut res = winresource::WindowsResource::new();
    let icon = std::path::Path::new("assets/icon.ico");
    if icon.is_file() {
        res.set_icon(icon.to_str().expect("icon path must be valid UTF-8"));
    }
    res.set("ProductName", "Game Optimizer");
    res.set("FileDescription", "Game Optimizer");
    res.set("LegalCopyright", "MIT");
    let windows_version = format!(
        "{}.{}.{}.0",
        env!("CARGO_PKG_VERSION_MAJOR"),
        env!("CARGO_PKG_VERSION_MINOR"),
        env!("CARGO_PKG_VERSION_PATCH")
    );
    res.set("ProductVersion", &windows_version);
    res.set("FileVersion", &windows_version);
    res.compile()
        .expect("failed to compile required Windows icon/version resources");

    // Keep the elevation request exclusive to the real application binary.
    // Library test harnesses do not receive these flags. Cargo.toml disables
    // the empty main.rs harness, which would otherwise inherit bin elevation.
    let manifest = std::fs::canonicalize(manifest)
        .expect("failed to resolve required Windows application manifest");
    let manifest = manifest
        .to_str()
        .expect("manifest path must be valid UTF-8")
        .replace('/', "\\");
    println!("cargo:rustc-link-arg-bin=game_optimizer=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg-bin=game_optimizer=/MANIFESTINPUT:{manifest}");
    println!("cargo:rustc-link-arg-bin=game_optimizer=/MANIFESTUAC:level='requireAdministrator' uiAccess='false'");
}
