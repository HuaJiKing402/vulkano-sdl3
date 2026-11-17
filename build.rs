fn main() {
    // 本机是 windows-gnu 工具链，SDL3 SDK 只提供 MSVC 的 .lib；
    // native-libs/libSDL3.dll.a 由 gendef + dlltool 从 SDL3.dll 生成。
    println!(
        "cargo:rustc-link-search=native={}/native-libs",
        env!("CARGO_MANIFEST_DIR").replace('\\', "/")
    );
}
