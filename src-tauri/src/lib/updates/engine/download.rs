//! 带防护的下载器：https-only、大小上限、流式读取 + 进度回调。
//!
//! 防护口径（设计 §4 门 2）：
//! - 仅接受 https URL（协议白名单同时传给 curl，防重定向降级）；
//! - 边下边累计，超过 `max_bytes` 立即中止；
//! - `--fail`：HTTP 4xx/5xx 不把错误页落盘；
//! - `--max-time` 总限时，防连接挂起；
//! - 任何失败删除半成品文件。

use std::io::{Read, Write};
use std::path::Path;

/// 下载上限：512 MB（对齐 Burrow `maximumArchiveBytes`）。
pub const MAX_DOWNLOAD_BYTES: u64 = 512 * 1024 * 1024;

/// 下载总限时（curl `--max-time`，秒）。
const DOWNLOAD_MAX_TIME_SECS: &str = "900";

/// URL 是否为允许的下载来源（https-only，大小写宽容）。
pub fn is_allowed_url(url: &str) -> bool {
    url.trim().to_ascii_lowercase().starts_with("https://")
}

/// 下载 `url` 到 `dest`（curl 子进程 stdout 流式写入，边下边报进度）。
///
/// `on_progress(downloaded, total)`：total 暂恒为 `None`——不额外发 HEAD 请求，
/// UI 以"已下载 X MB"展示；需要百分比时再补 Content-Length 探测。
///
/// 返回实际下载字节数；失败返回可读原因并清理半成品。
pub fn download_to_file(
    url: &str,
    dest: &Path,
    max_bytes: u64,
    on_progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<u64, String> {
    if !is_allowed_url(url) {
        return Err(format!("拒绝非 https 下载地址: {url}"));
    }
    let mut child = std::process::Command::new("/usr/bin/curl")
        .args([
            "-sS",
            "--fail",
            "-L",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--max-time",
            DOWNLOAD_MAX_TIME_SECS,
            url,
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("启动下载失败: {e}"))?;

    let mut stdout = child.stdout.take().ok_or("无法读取下载流")?;
    let mut file = std::fs::File::create(dest).map_err(|e| format!("创建下载文件失败: {e}"))?;

    let mut downloaded: u64 = 0;
    let mut chunk = vec![0u8; 128 * 1024];
    let read_result: Result<(), String> = loop {
        match stdout.read(&mut chunk) {
            Ok(0) => break Ok(()),
            Ok(n) => {
                downloaded += n as u64;
                if downloaded > max_bytes {
                    let _ = child.kill();
                    break Err(format!(
                        "下载超过大小上限（{} MB），已中止",
                        max_bytes / (1024 * 1024)
                    ));
                }
                if let Err(e) = file.write_all(&chunk[..n]) {
                    let _ = child.kill();
                    break Err(format!("写入下载文件失败: {e}"));
                }
                on_progress(downloaded, None);
            }
            Err(e) => {
                let _ = child.kill();
                break Err(format!("读取下载流出错: {e}"));
            }
        }
    };

    // 读干净 stderr（拿 curl 的错误摘要），随后等待退出。
    let mut err_text = String::new();
    if let Some(mut stderr) = child.stderr.take() {
        let _ = stderr.read_to_string(&mut err_text);
    }
    let status = child.wait().map_err(|e| format!("等待下载进程退出失败: {e}"))?;

    let result = match read_result {
        Err(e) => Err(e),
        Ok(()) => {
            if status.success() {
                Ok(downloaded)
            } else {
                let reason = err_text.trim();
                Err(if reason.is_empty() {
                    "下载失败（HTTP 错误或连接中断）".to_string()
                } else {
                    format!("下载失败: {reason}")
                })
            }
        }
    };
    if result.is_err() {
        drop(file);
        let _ = std::fs::remove_file(dest);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_https() {
        assert!(!is_allowed_url("http://example.com/a.zip"));
        assert!(!is_allowed_url("ftp://example.com/a.zip"));
        assert!(!is_allowed_url("file:///tmp/a.zip"));
        assert!(!is_allowed_url(""));
        assert!(is_allowed_url("https://iterm2.com/a.zip"));
        assert!(is_allowed_url("  HTTPS://ITerm2.COM/a.zip  "));
    }

    #[test]
    fn rejects_non_https_before_spawning() {
        let dest = std::env::temp_dir().join("molan-dl-test-should-not-exist.bin");
        let mut progress = |_: u64, _: Option<u64>| {};
        let r = download_to_file("http://example.com/a.zip", &dest, 1024, &mut progress);
        assert!(r.is_err());
        assert!(!dest.exists());
    }
}
