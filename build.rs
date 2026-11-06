// Embeds the application icon into the Windows executable (works with MSVC and MinGW).
fn main() {
    println!("cargo:rerun-if-changed=assets/icons/mnemo.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icons/mnemo.ico");
        res.set("ProductName", "Mnemo");
        res.set("FileDescription", "Mnemo - notes Markdown sur PostgreSQL");
        if let Err(e) = res.compile() {
            println!("cargo:warning=icône Windows non intégrée : {e}");
        }
    }
}
