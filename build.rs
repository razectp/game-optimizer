fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=assets/app.manifest");

    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "windows" {
        return;
    }

    let mut res = winresource::WindowsResource::new();
    if std::path::Path::new("assets/icon.ico").exists() {
        res.set_icon("assets/icon.ico");
    }
    if std::path::Path::new("assets/app.manifest").exists() {
        res.set_manifest_file("assets/app.manifest");
    }
    res.set("ProductName", "Game Optimizer");
    res.set("FileDescription", "Game Optimizer");
    res.set("LegalCopyright", "MIT");
    res.set("ProductVersion", "0.4.0.0");
    res.set("FileVersion", "0.4.0.0");
    if let Err(err) = res.compile() {
        println!("cargo:warning=windows resource compile skipped: {err}");
    }
}
