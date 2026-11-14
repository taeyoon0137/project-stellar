//! Read-only kernel proof: the unreaped Git child has this lock inode open for writing.
//! A lock pathname, PID liveness, or a trace message alone is not proof.
use std::io;
use std::path::Path;

#[cfg(target_os = "macos")]
use std::os::unix::fs::MetadataExt;

pub(super) fn owns_lock(pid: u32, path: &Path) -> io::Result<bool> {
    #[cfg(target_os = "macos")]
    {
        let metadata = match std::fs::symlink_metadata(path) {
            Ok(m) if m.is_file() => m,
            Ok(_) => return Err(io::ErrorKind::InvalidData.into()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e),
        };
        platform::owns(pid, metadata.dev(), metadata.ino())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (pid, path);
        Err(io::ErrorKind::Unsupported.into())
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use std::ffi::c_void;
    use std::mem::size_of;

    // macOS SDK sys/proc_info.h: PROC_PIDLISTFDS / PROC_PIDFDVNODEINFO.
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct FdInfo {
        fd: i32,
        kind: u32,
    }
    #[repr(C)]
    #[derive(Default)]
    struct FileInfo {
        openflags: u32,
        status: u32,
        offset: i64,
        kind: i32,
        guard: u32,
    }
    #[repr(C)]
    #[derive(Default)]
    struct VinfoStat {
        dev: u32,
        mode: u16,
        nlink: u16,
        ino: u64,
        uid: u32,
        gid: u32,
        atime: i64,
        atimensec: i64,
        mtime: i64,
        mtimensec: i64,
        ctime: i64,
        ctimensec: i64,
        birthtime: i64,
        birthtimensec: i64,
        size: i64,
        blocks: i64,
        blksize: i32,
        flags: u32,
        gen: u32,
        rdev: u32,
        spare: [i64; 2],
    }
    #[repr(C)]
    #[derive(Default)]
    struct VnodeInfo {
        stat: VinfoStat,
        kind: i32,
        pad: i32,
        fsid: [i32; 2],
    }
    #[repr(C)]
    #[derive(Default)]
    struct VnodeFdInfo {
        file: FileInfo,
        vnode: VnodeInfo,
    }

    #[link(name = "proc")]
    extern "C" {
        fn proc_pidinfo(pid: i32, flavor: i32, arg: u64, buffer: *mut c_void, size: i32) -> i32;
        fn proc_pidfdinfo(pid: i32, fd: i32, flavor: i32, buffer: *mut c_void, size: i32) -> i32;
    }

    pub(super) fn owns(pid: u32, dev: u64, ino: u64) -> io::Result<bool> {
        let pid = i32::try_from(pid).map_err(|_| io::ErrorKind::InvalidInput)?;
        // SAFETY: null buffer asks the kernel for the required byte count only.
        let size = unsafe { proc_pidinfo(pid, 1, 0, std::ptr::null_mut(), 0) };
        if size <= 0 {
            return Err(io::Error::last_os_error());
        }
        let mut fds = vec![FdInfo::default(); size as usize / size_of::<FdInfo>() + 16];
        let capacity = i32::try_from(fds.len() * size_of::<FdInfo>())
            .map_err(|_| io::ErrorKind::InvalidData)?;
        // SAFETY: aligned repr(C) integer buffer has capacity bytes and remains alive during the call.
        let used = unsafe { proc_pidinfo(pid, 1, 0, fds.as_mut_ptr().cast(), capacity) };
        if used <= 0 || used > capacity || !(used as usize).is_multiple_of(size_of::<FdInfo>()) {
            return Err(io::ErrorKind::InvalidData.into());
        }
        for fd in &fds[..used as usize / size_of::<FdInfo>()] {
            if fd.kind != 1 {
                continue;
            } // PROX_FDTYPE_VNODE
            let mut info = VnodeFdInfo::default();
            // SAFETY: SDK-compatible repr(C) output with its exact size; only read on a full result.
            let n = unsafe {
                proc_pidfdinfo(
                    pid,
                    fd.fd,
                    1,
                    (&mut info as *mut VnodeFdInfo).cast(),
                    size_of::<VnodeFdInfo>() as i32,
                )
            };
            if n == size_of::<VnodeFdInfo>() as i32
                && info.file.openflags & 2 != 0 // FWRITE (kernel flags, not O_WRONLY)
                && u64::from(info.vnode.stat.dev) == dev
                && info.vnode.stat.ino == ino
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
}
