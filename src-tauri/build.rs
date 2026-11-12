use std::path::Path;

fn main() {
    tauri_build::build();
    link_sherpa_onnx();
    copy_sqlite3_dll();
}

/// sherpa-onnx（语音管线，M1-3）：官方预编译 DLL + 导入库（坑 25 同款方案，坑 25b 的
/// C++ 本机编不了）。lib/ 自带 MSVC 导入库，rust-lld 可直接消费。
fn link_sherpa_onnx() {
    let lib = Path::new("../tools/sherpa-onnx/sherpa-onnx-v2.0.1-win-x64-shared-MD-MinSizeRel/lib");
    println!("cargo:rerun-if-changed={}", lib.display());
    if !lib.join("sherpa-onnx-c-api.lib").exists() {
        println!(
            "cargo:warning=sherpa-onnx 预编译库不存在，跳过链接（运行时需自行放置，见 dev 文档）"
        );
        return;
    }
    println!(
        "cargo:rustc-link-search=native={}",
        lib.canonicalize().unwrap().display()
    );
    println!("cargo:rustc-link-lib=dylib=sherpa-onnx-c-api");

    // DLL 必须随 exe 同目录（同 sqlite，坑 27c 的教训：不走 resources）
    let out = std::env::var("OUT_DIR").expect("OUT_DIR");
    let target_dir = Path::new(&out)
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .expect("resolve target dir");
    for dll in [
        "sherpa-onnx-c-api.dll",
        "onnxruntime.dll",
        "onnxruntime_providers_shared.dll",
    ] {
        std::fs::copy(lib.join(dll), target_dir.join(dll)).expect("copy {dll}");
        let deps = target_dir.join("deps");
        if deps.is_dir() {
            std::fs::copy(lib.join(dll), deps.join(dll)).expect("copy {dll} to deps");
        }
    }
}

/// sqlite3.dll 是动态链接（坑 25：本机 gcc 5.3 编不了 sqlite3.c，用官方预编译 dll），
/// 必须随 exe 同目录。开发模式直接拷到 target/<profile>/，打包走 tauri resources（M1-1 后半）。
fn copy_sqlite3_dll() {
    println!("cargo:rerun-if-changed=../tools/sqlite3/dll/sqlite3.dll");
    let dll = Path::new("../tools/sqlite3/dll/sqlite3.dll");
    if !dll.exists() {
        // 允许只跑 cargo check 的环境（依赖下载由文档步骤负责），运行时才需要 dll
        println!("cargo:warning=sqlite3.dll 不存在，跳过拷贝（运行时需自行放置）");
        return;
    }
    let out = std::env::var("OUT_DIR").expect("OUT_DIR");
    // OUT_DIR = target/<profile>/build/chaojiandai-*/out → 上三级即 target/<profile>/
    let target_dir = Path::new(&out)
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .expect("resolve target dir");
    std::fs::copy(dll, target_dir.join("sqlite3.dll")).expect("copy sqlite3.dll");
    // 测试二进制在 target/<profile>/deps/ 下运行，同样要能找到 dll
    let deps = target_dir.join("deps");
    if deps.is_dir() {
        std::fs::copy(dll, deps.join("sqlite3.dll")).expect("copy sqlite3.dll to deps");
        // tauri 只把 WebView2Loader.dll 拷到 target/<profile>/，测试二进制在 deps/ 下加载不到
        let wv2 = target_dir.join("WebView2Loader.dll");
        if wv2.is_file() {
            std::fs::copy(&wv2, deps.join("WebView2Loader.dll"))
                .expect("copy WebView2Loader.dll to deps");
        }
    }
}
