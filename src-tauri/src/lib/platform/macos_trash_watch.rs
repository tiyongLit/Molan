//! 当前用户废纸篓顶层原生监听。kqueue 仅发 dirty，统计交给唯一调度线程。
use crate::runtime::trash_watch::Service;
use std::ffi::CString;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

struct Queue(OwnedFd);
impl Queue {
    fn new() -> io::Result<Self> {
        let fd = unsafe { libc::kqueue() };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let queue = Self(unsafe { OwnedFd::from_raw_fd(fd) });
        queue.change(1, libc::EVFILT_USER, libc::EV_ADD | libc::EV_CLEAR, 0)?;
        Ok(queue)
    }
    fn change(&self, ident: usize, filter: i16, flags: u16, fflags: u32) -> io::Result<()> {
        let event = libc::kevent {
            ident,
            filter,
            flags,
            fflags,
            data: 0,
            udata: std::ptr::null_mut(),
        };
        let result = unsafe {
            libc::kevent(
                self.0.as_raw_fd(),
                &event,
                1,
                std::ptr::null_mut(),
                0,
                std::ptr::null(),
            )
        };
        if result < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
    fn bind(&self, path: &Path) -> io::Result<OwnedFd> {
        let name = CString::new(path.as_os_str().as_bytes())?;
        let fd = unsafe {
            libc::open(
                name.as_ptr(),
                libc::O_EVTONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_DIRECTORY,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        self.change(
            fd.as_raw_fd() as usize,
            libc::EVFILT_VNODE,
            libc::EV_ADD | libc::EV_CLEAR,
            libc::NOTE_WRITE
                | libc::NOTE_EXTEND
                | libc::NOTE_ATTRIB
                | libc::NOTE_DELETE
                | libc::NOTE_RENAME
                | libc::NOTE_REVOKE,
        )?;
        Ok(fd)
    }
    fn wait(&self, duration: Duration) -> io::Result<Option<(i16, u32)>> {
        let mut event: libc::kevent = unsafe { std::mem::zeroed() };
        let timeout = libc::timespec {
            tv_sec: duration.as_secs() as _,
            tv_nsec: duration.subsec_nanos() as _,
        };
        let n = unsafe {
            libc::kevent(
                self.0.as_raw_fd(),
                std::ptr::null(),
                0,
                &mut event,
                1,
                &timeout,
            )
        };
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        if n == 0 {
            return Ok(None);
        }
        if event.flags & libc::EV_ERROR != 0 {
            return Err(io::Error::from_raw_os_error(event.data as i32));
        }
        Ok(Some((event.filter, event.fflags)))
    }
    fn stop(&self) {
        let _ = self.change(1, libc::EVFILT_USER, 0, libc::NOTE_TRIGGER);
    }
}

pub fn start(service: Arc<Service>) {
    std::thread::Builder::new()
        .name("trash-vnode".into())
        .spawn(move || {
            let Some(path) = crate::core::base::home_dir_opt().map(|p| p.join(".Trash")) else {
                return;
            };
            let thread = std::thread::current();
            let wake = thread.clone();
            service.install_native_stop(Box::new(move || wake.unpark()));
            let mut queue: Option<Arc<Queue>> = None;
            let mut fd = None;
            let mut identity = None;
            let mut failed = false;
            while !service.is_stopped() {
                if queue.is_none() {
                    match Queue::new() {
                        Ok(new) => {
                            let new = Arc::new(new);
                            let signal = new.clone();
                            let wake = thread.clone();
                            service.install_native_stop(Box::new(move || {
                                signal.stop();
                                wake.unpark();
                            }));
                            queue = Some(new);
                        }
                        Err(error) => {
                            if !failed {
                                log::warn!(
                                    "[trash-watch] kqueue unavailable, retry in 120s: {error}"
                                );
                            }
                            failed = true;
                            std::thread::park_timeout(Duration::from_secs(120));
                            continue;
                        }
                    }
                }
                let current_queue = queue.as_ref().unwrap();
                if fd.is_none() {
                    match current_queue.bind(&path) {
                        Ok(opened) => {
                            fd = Some(opened);
                            identity = std::fs::symlink_metadata(&path)
                                .ok()
                                .map(|m| (m.dev(), m.ino()));
                            if failed {
                                log::info!("[trash-watch] native watch recovered");
                            }
                            failed = false;
                            service.dirty();
                        }
                        Err(error) => {
                            if !failed {
                                log::warn!(
                                    "[trash-watch] watch unavailable; retry in 120s: {error}"
                                );
                            }
                            failed = true;
                        }
                    }
                }
                match current_queue.wait(Duration::from_secs(120)) {
                    Ok(Some((libc::EVFILT_USER, _))) => break,
                    Ok(Some((_, flags))) => {
                        service.dirty();
                        if flags
                            & (libc::NOTE_DELETE
                                | libc::NOTE_RENAME
                                | libc::NOTE_REVOKE
                                | libc::NOTE_ATTRIB)
                            != 0
                        {
                            fd = None;
                        }
                    }
                    Ok(None) => {
                        let current = std::fs::symlink_metadata(&path)
                            .ok()
                            .map(|m| (m.dev(), m.ino()));
                        if current != identity {
                            fd = None;
                            service.dirty();
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                    Err(error) => {
                        log::warn!(
                            "[trash-watch] native wait failed, using health polling: {error}"
                        );
                        fd = None;
                        queue = None;
                        std::thread::park_timeout(Duration::from_secs(120));
                    }
                }
            }
        })
        .expect("trash vnode worker");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn watches_changes_and_rebinds_recreated_directory() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("trash");
        std::fs::create_dir(&root).unwrap();
        let queue = Queue::new().unwrap();
        let fd = queue.bind(&root).unwrap();
        std::fs::write(root.join("item"), b"x").unwrap();
        assert!(queue.wait(Duration::from_secs(2)).unwrap().is_some());
        std::fs::rename(&root, dir.path().join("old")).unwrap();
        assert!(queue.wait(Duration::from_secs(2)).unwrap().is_some());
        drop(fd);
        std::fs::create_dir(&root).unwrap();
        let _rebound = queue.bind(&root).unwrap();
        std::fs::write(root.join("new"), b"x").unwrap();
        assert!(queue.wait(Duration::from_secs(2)).unwrap().is_some());
        queue.stop();
        assert_eq!(
            queue.wait(Duration::from_secs(2)).unwrap().unwrap().0,
            libc::EVFILT_USER
        );
    }
}
