//! `rust-embed` needs the folder to exist at compile time. Create it if the
//! game has not been built yet so the server still compiles (serving a hint page).
fn main() {
    let dist = std::path::Path::new("../ludo-game/dist");
    std::fs::create_dir_all(dist).expect("create ludo-game/dist");
    println!("cargo:rerun-if-changed=../ludo-game/dist");
}
