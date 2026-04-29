/// Tauri のビルド設定を初期化する
fn main() {
    println!("cargo:rerun-if-changed=Info.plist");

    tauri_build::build();
}
