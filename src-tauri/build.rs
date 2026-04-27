use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    tauri_build::build();

    // 生成前后端共享常量（Rust → TypeScript），单一事实来源。
    // 当 constants.rs 变化时重新运行本脚本，从而重新生成 shared.ts。
    println!("cargo:rerun-if-changed=src/constants.rs");

    mod constants {
        include!("src/constants.rs");
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = manifest_dir.join("../src/constants");
    fs::create_dir_all(&out_dir).unwrap();

    // 逐项拼接生成 TS。key/value 的安全性由 constants.rs 内的单测保证
    // （type_desc_keys_are_safe_for_ts_generation：全小写、不含引号/反斜杠），此处不做转义。
    let mut ts = String::new();
    ts.push_str("// 此文件由 src-tauri/build.rs 自动生成，请勿手动修改。\n");
    ts.push_str("// 单一事实来源：src-tauri/src/constants.rs\n\n");
    ts.push_str(&format!(
        "export const SIZE_BASE = {} as const\n",
        constants::SIZE_BASE
    ));

    ts.push_str("\n// Analyze EntryRow 副标题：文件扩展名（小写、不含点）→ 类型描述\n");
    ts.push_str("export const FILE_TYPE_DESCS: Record<string, string> = {\n");
    for (k, v) in constants::FILE_TYPE_DESCS {
        ts.push_str(&format!("  '{}': '{}',\n", k, v));
    }
    ts.push_str("} as const\n");

    ts.push_str(
        "\n// Analyze EntryRow 副标题：目录名后缀（含点）→ 类型描述（.app 等目录型 bundle）\n",
    );
    ts.push_str("export const BUNDLE_TYPE_DESCS: Record<string, string> = {\n");
    for (k, v) in constants::BUNDLE_TYPE_DESCS {
        ts.push_str(&format!("  '{}': '{}',\n", k, v));
    }
    ts.push_str("} as const\n");

    let ts_file_path = out_dir.join("shared.ts");
    // 仅在内容变化时写入，避免每次构建都更新 mtime 触发前端 Vite 重新处理模块图。
    if let Ok(existing) = fs::read_to_string(&ts_file_path) {
        if existing == ts {
            return;
        }
    }
    fs::write(&ts_file_path, ts).unwrap();

    println!(
        "cargo:warning=Generated shared constants at {}",
        ts_file_path.display()
    );
}
