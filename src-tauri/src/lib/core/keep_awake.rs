//! macOS IOKit 防休眠 RAII guard。
//!
//! 长任务（analyze / clean / uninstall / optimize）进入 `spawn_blocking` 时，
//! 创建 `KeepAwakeGuard::acquire(reason)` 持有 IOKit 电源断言，阻止系统空闲休眠；
//! 任务结束 guard 自动 Drop，释放断言。
//!
//! 使用 IOKit C FFI，零新依赖。超时设为 1 小时作为安全兜底——
//! 如果进程异常退出（断言随进程消亡），断言也不会永久残留。

#[cfg(target_os = "macos")]
mod inner {
    use std::ffi::c_void;

    // ── IOKit C API ──
    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IOPMAssertionCreateWithName(
            assertion_type: *const c_void, // CFStringRef
            timeout: u32,
            assertion_name: *const c_void, // CFStringRef
            assertion_id: *mut u32,
        ) -> i32; // IOReturn

        fn IOPMAssertionRelease(assertion_id: u32) -> i32;
    }

    /// IOKit 断言类型：阻止用户空闲导致系统休眠。
    const ASSERTION_TYPE: &str = "PreventUserIdleSystemSleep";
    /// 安全兜底超时（秒）：1 小时。防止 guard 泄漏（如 panic 后未 Drop）导致断言永久残留。
    const TIMEOUT_SECONDS: u32 = 3600;

    /// IOKit 电源断言 RAII guard。
    ///
    /// - `acquire()` 创建断言，阻止系统休眠
    /// - `Drop` 释放断言，系统恢复正常休眠策略
    pub struct KeepAwakeGuard {
        assertion_id: u32,
    }

    impl KeepAwakeGuard {
        /// 获取 IOKit 防休眠断言。
        ///
        /// `reason` 会在 `pmset -g assertions` 中展示，建议使用英文描述任务类型。
        /// 失败时返回 `None`（不会 panic，任务仍可正常执行）。
        pub fn acquire(reason: &str) -> Option<Self> {
            use core_foundation::base::TCFType;
            use core_foundation::string::CFString;

            let type_cf = CFString::new(ASSERTION_TYPE);
            let name_cf = CFString::new(reason);

            let mut id: u32 = 0;
            let status = unsafe {
                IOPMAssertionCreateWithName(
                    type_cf.as_concrete_TypeRef() as *const c_void,
                    TIMEOUT_SECONDS,
                    name_cf.as_concrete_TypeRef() as *const c_void,
                    &mut id,
                )
            };

            if status == 0 {
                log::info!(
                    "[keep_awake] acquired assertion id={}, reason={}",
                    id,
                    reason
                );
                Some(Self { assertion_id: id })
            } else {
                log::warn!(
                    "[keep_awake] failed to acquire assertion: status={}",
                    status
                );
                None
            }
        }
    }

    impl Drop for KeepAwakeGuard {
        fn drop(&mut self) {
            let status = unsafe { IOPMAssertionRelease(self.assertion_id) };
            if status != 0 {
                log::warn!(
                    "[keep_awake] failed to release assertion id={}: status={}",
                    self.assertion_id,
                    status
                );
            } else {
                log::info!("[keep_awake] released assertion id={}", self.assertion_id);
            }
        }
    }
}

// 非 macOS 平台：no-op stub，保证跨平台编译。
#[cfg(not(target_os = "macos"))]
mod inner {
    pub struct KeepAwakeGuard;

    impl KeepAwakeGuard {
        pub fn acquire(_reason: &str) -> Option<Self> {
            Some(Self)
        }
    }
}

pub use inner::KeepAwakeGuard;
