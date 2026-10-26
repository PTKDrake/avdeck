//! Embeds the icon and the version information Windows shows in the file's properties and in
//! Task Manager.

fn main() {
    println!("cargo:rerun-if-changed=assets/avdeck.ico");
    let authors = std::env::var("CARGO_PKG_AUTHORS").unwrap_or_default();
    // `Name <email>`: the name only.
    let author = authors.split(" <").next().unwrap_or_default();
    let mut resource = winresource::WindowsResource::new();
    resource
        .set_icon("assets/avdeck.ico")
        .set("ProductName", "Avdeck")
        .set("FileDescription", "Avdeck")
        .set("LegalCopyright", &format!("© 2026 {author}"));
    resource
        .compile()
        .expect("cannot embed the icon and version information");
}
