//! 安装会话编排：prepare（feed → 下载 → 逐门验证 → 暂存就绪）与
//! commit（替换 + 重启）。
//!
//! 事件对接在 controllers 层（D 阶段）：本模块只通过 `on_stage` 回调上报阶段，
//! 不依赖 Tauri。
//!
//! 并发口径：同一时刻仅允许一个安装会话——由控制器层保证（`PreparedInstall`
//! 单持；前端亦有 pending 防重）。

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::verify::codesign::{self, CodeSignatureInfo};
use super::{archive, download, feed, installer, staging};
use crate::updates::appcast;

/// 退出等待上限（对齐 Burrow 的 8s deadline）。
pub const QUIT_TIMEOUT: Duration = Duration::from_secs(8);

/// 会话阶段（D 阶段映射到 `updates::install-progress` 事件的 stage 字段）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallStage {
    /// 下载中（第二参 = 已下载字节数；total 暂不可得）
    Downloading,
    /// 验证中（EdDSA → 解压 → Apple 锚定 → 身份一致性）
    Verifying,
    /// 暂存就绪，等待用户确认「安装并重启」
    ReadyToInstall,
    /// 替换中
    Installing,
    /// 已完成
    Completed,
}

/// prepare 的产物（会话对象）：持有暂存目录与前后身份快照。
#[derive(Debug)]
pub struct PreparedInstall {
    pub app_path: String,
    pub bundle_id: String,
    pub new_version: String,
    /// 0700 暂存目录（成功后 / cancel 时清理；commit 失败保留以便重试）
    pub staged_dir: PathBuf,
    pub staged_app: PathBuf,
    pub old_identity: CodeSignatureInfo,
    pub new_identity: CodeSignatureInfo,
}

/// prepare：拉取 feed → 选择条目 → 下载 → 门 1/4/5 验证 → 暂存就绪。
///
/// 任何一步失败：丢弃暂存、不触碰系统文件（铁律）。
pub fn prepare_install(
    app_path: &str,
    current_version: &str,
    on_stage: &mut dyn FnMut(InstallStage, Option<u64>),
) -> Result<PreparedInstall, String> {
    // ── 元数据（缺 EdDSA 公钥 = Sparkle 1.x 场景，P0 明确不支持）──
    let meta = feed::read_sparkle_meta(app_path);
    if meta.feed_url.is_empty() {
        return Err("目标应用缺少 SUFeedURL，无法原地更新".to_string());
    }
    if meta.public_key_b64.is_empty() {
        return Err(
            "目标应用缺少 SUPublicEDKey（Sparkle 1.x 或无签名配置），当前不支持原地更新"
                .to_string(),
        );
    }

    // ── feed 拉取与选包（channel 过滤已按订阅语义处理）──
    let xml = fetch_feed_xml(&meta.feed_url)?;
    let items = appcast::parse_appcast_items(&xml);
    let channel = feed::subscribed_channel(&meta.feed_url);
    let item = feed::select_update(&items, current_version, channel.as_deref())
        .ok_or_else(|| "appcast 中没有可用的全量更新包".to_string())?;

    // ── 暂存目录（后续任何失败都丢弃）──
    let staged_dir = staging::create_staging_dir()?;
    let result = prepare_inner(app_path, &meta, item, &staged_dir, on_stage);
    match result {
        Ok(prepared) => {
            on_stage(InstallStage::ReadyToInstall, None);
            Ok(prepared)
        }
        Err(e) => {
            staging::discard_staging_dir(&staged_dir);
            Err(e)
        }
    }
}

fn prepare_inner(
    app_path: &str,
    meta: &feed::SparkleMeta,
    item: &appcast::AppcastItem,
    staged_dir: &Path,
    on_stage: &mut dyn FnMut(InstallStage, Option<u64>),
) -> Result<PreparedInstall, String> {
    let download_url = item.download_url.clone().unwrap_or_default();
    let ed_signature = item.ed_signature.clone().unwrap_or_default();
    let new_version = item.comparable_version().to_string();

    // ── 门 2：下载（https-only / 512MB 上限 / 进度回调）──
    on_stage(InstallStage::Downloading, Some(0));
    let zip_path = staged_dir.join("update.zip");
    download::download_to_file(
        &download_url,
        &zip_path,
        download::MAX_DOWNLOAD_BYTES,
        &mut |done, _| on_stage(InstallStage::Downloading, Some(done)),
    )?;

    // ── 门 1：EdDSA 验签（对包原始字节）──
    on_stage(InstallStage::Verifying, None);
    let bytes = std::fs::read(&zip_path).map_err(|e| format!("读取更新包失败: {e}"))?;
    super::verify::eddsa::verify_ed25519(&meta.public_key_b64, &bytes, &ed_signature)?;

    // ── 解压（Zip Slip 双保险）──
    let extracted = staged_dir.join("extracted");
    let staged_app = archive::extract_zip_and_find_app(&zip_path, &extracted)?;

    // ── 门 4：新包 Apple 锚定校验 ──
    let new_str = staged_app.to_string_lossy().to_string();
    let new_identity = codesign::inspect(&new_str)?;

    // ── 门 5：新旧身份一致性 ──
    let old_identity = codesign::inspect(app_path)?;
    super::identity::ensure_same_identity(&old_identity, &new_identity)?;

    Ok(PreparedInstall {
        app_path: app_path.to_string(),
        bundle_id: old_identity.bundle_id.clone().unwrap_or_default(),
        new_version,
        staged_dir: staged_dir.to_path_buf(),
        staged_app,
        old_identity,
        new_identity,
    })
}

/// commit：安装并重启。
/// - 失败：保留暂存（可重试），不触碰不确定状态；
/// - 成功：清理暂存并上报 `Completed`。
pub fn commit_prepared(
    prepared: PreparedInstall,
    on_stage: &mut dyn FnMut(InstallStage, Option<u64>),
) -> Result<(), String> {
    on_stage(InstallStage::Installing, None);
    let outcome = installer::commit_install(
        Path::new(&prepared.app_path),
        &prepared.staged_app,
        &prepared.old_identity,
        &prepared.new_identity,
        QUIT_TIMEOUT,
    );
    match outcome {
        Ok(_) => {
            // staged_app 已被 rename 走；剩下 zip / extracted 一并清理。
            staging::discard_staging_dir(&prepared.staged_dir);
            on_stage(InstallStage::Completed, None);
            Ok(())
        }
        Err(e) => Err(e),
    }
}

/// cancel：丢弃暂存（不触碰系统文件）。
pub fn cancel_prepared(prepared: &PreparedInstall) {
    staging::discard_staging_dir(&prepared.staged_dir);
}

// ── 进程级会话表与并发闸 ──

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

fn sessions() -> &'static Mutex<HashMap<String, PreparedInstall>> {
    static SESSIONS: OnceLock<Mutex<HashMap<String, PreparedInstall>>> = OnceLock::new();
    SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 存入会话（供后续 commit / cancel 取用）。
/// 同 app_path 的旧会话先丢弃暂存（防暂存泄漏）。
pub fn stash_session(prepared: PreparedInstall) {
    let mut map = sessions().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(old) = map.insert(prepared.app_path.clone(), prepared) {
        staging::discard_staging_dir(&old.staged_dir);
    }
}

/// 取出会话（取出即从表中移除）。
pub fn take_session(app_path: &str) -> Option<PreparedInstall> {
    sessions()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(app_path)
}

/// 安装通道闸：prepare / commit 执行期间占用（同一时刻仅一个任务）。
/// 达意为 `Drop` 自动释放——不会因错误路径漏放。
pub struct InstallSlotGuard;

static INSTALL_SLOT: AtomicBool = AtomicBool::new(false);

impl InstallSlotGuard {
    /// 尝试占用安装通道；失败 = 已有任务进行中。
    pub fn acquire() -> Result<Self, String> {
        INSTALL_SLOT
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map(|_| InstallSlotGuard)
            .map_err(|_| "已有安装任务正在进行，请稍后再试".to_string())
    }
}

impl Drop for InstallSlotGuard {
    fn drop(&mut self) {
        INSTALL_SLOT.store(false, Ordering::SeqCst);
    }
}

/// `InstallStage` → 事件 stage 字符串（前端契约）。
pub fn stage_str(stage: InstallStage) -> &'static str {
    match stage {
        InstallStage::Downloading => "downloading",
        InstallStage::Verifying => "verifying",
        InstallStage::ReadyToInstall => "ready_to_install",
        InstallStage::Installing => "installing",
        InstallStage::Completed => "completed",
    }
}

/// 拉取 appcast XML（curl 子进程，https-only，15s 超时）。
fn fetch_feed_xml(url: &str) -> Result<String, String> {
    if !url.trim().to_ascii_lowercase().starts_with("https://") {
        return Err(format!("feed 非 https，已拒绝: {url}"));
    }
    crate::core::timeout::run_with_timeout_capture_lossy(
        15.0,
        "/usr/bin/curl",
        &["-sSL", "--max-time", "15", url],
    )
    .filter(|s| !s.trim().is_empty())
    .ok_or_else(|| format!("拉取 feed 失败: {url}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepare_rejects_app_without_feed() {
        // 不存在的 App：读不到 SUFeedURL → 明确报错（不触碰网络/文件系统）
        let r = prepare_install(
            "/nonexistent-molan/probe-9f3a.app",
            "1.0",
            &mut |_, _| {},
        );
        let err = r.unwrap_err();
        assert!(err.contains("SUFeedURL"), "应提示缺少 feed: {err}");
    }

    #[test]
    fn fetch_rejects_non_https_feed() {
        let err = fetch_feed_xml("http://example.com/appcast.xml").unwrap_err();
        assert!(err.contains("https"), "{err}");
    }

    #[test]
    fn cancel_on_missing_dir_is_safe() {
        let prepared = PreparedInstall {
            app_path: "/tmp/none".to_string(),
            bundle_id: "com.example.none".to_string(),
            new_version: "2.0".to_string(),
            staged_dir: PathBuf::from("/nonexistent-molan/staging-9f3a"),
            staged_app: PathBuf::from("/nonexistent-molan/staging-9f3a/App.app"),
            old_identity: CodeSignatureInfo::default(),
            new_identity: CodeSignatureInfo::default(),
        };
        cancel_prepared(&prepared); // 不得 panic
    }
}
