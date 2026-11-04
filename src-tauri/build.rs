fn main() {
    println!("cargo:rerun-if-changed=windows-app-manifest.xml");
    // A custom manifest: Tauri's Common Controls dependency plus Windows 8+
    // compatibility (required for layered child windows) and per-monitor-v2
    // DPI awareness. Ignored on non-Windows targets.
    let windows = tauri_build::WindowsAttributes::new().app_manifest(include_str!("windows-app-manifest.xml"));
    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))
        .expect("failed to run tauri-build");
}
