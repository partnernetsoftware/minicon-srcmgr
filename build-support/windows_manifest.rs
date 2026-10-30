// Shared by the product executable and private adapter test target. The
// subclass query API is exported by Common Controls v6, not every default DLL.
fn main() {
    println!("cargo:rerun-if-changed=../../build-support/windows_manifest.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows")
        || std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc")
    {
        return;
    }
    println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg=/MANIFESTDEPENDENCY:type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'");

    // A signed Windows deliverable needs an explicit product identity and
    // version on every PE. The values come from the package manifest so the
    // resource can never drift from the released version.
    let version = std::env::var("CARGO_PKG_VERSION").expect("package version");
    let mut resource = winresource::WindowsResource::new();
    resource
        .set("ProductName", "MiniCon SrcMgr")
        .set("FileDescription", "MiniCon SrcMgr source control client")
        .set("CompanyName", "PARTNERNET SOFTWARE PTY LTD")
        .set("OriginalFilename", "minicon-srcmgr.exe")
        .set("InternalName", "minicon-srcmgr")
        .set("ProductVersion", &version)
        .set("FileVersion", &version);
    resource.compile().expect("failed to embed product version resources");
}
