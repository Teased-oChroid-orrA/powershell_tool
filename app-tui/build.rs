// Embeds the GS Engineering icon (and version info) into the Windows .exe so
// Explorer, the taskbar and shortcuts show it. Only runs when building ON
// Windows (the MSVC `rc.exe` from the Windows SDK is what compiles the
// resource; CI builds on windows-latest). Other hosts skip it - a macOS/Linux
// dev build has no .exe to brand.
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../GS_Engineering_Brand_Assets/windows/GS_Engineering.ico");
    #[cfg(windows)]
    embed_windows_resources();
}

#[cfg(windows)]
fn embed_windows_resources() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../GS_Engineering_Brand_Assets/windows/GS_Engineering.ico");
    res.set("ProductName", "GS Engineering Toolbench");
    res.set("CompanyName", "GS Engineering");
    if let Err(e) = res.compile() {
        panic!("failed to embed the Windows icon resource: {e}");
    }
}
