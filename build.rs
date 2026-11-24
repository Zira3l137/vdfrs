//! Embeds the icon in the Windows executable as resource 1: the one GPUI loads for the window and
//! taskbar, and the one Explorer shows for archives once vdfrs is in it (`vdfrs.exe,0`).

fn main() {
    println!("cargo:rerun-if-changed=assets/vdfrs.ico");
    // The host decides which build dependencies exist, the target whether there is an executable
    // to put a Windows resource into.
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("assets/vdfrs.ico")
            .compile()
            .expect("cannot embed the icon; is the Windows SDK's rc.exe installed?");
    }
}
