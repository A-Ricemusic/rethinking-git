//! Descriptor-pinned SQLite namespace access for Unix repositories.
//!
//! The bundled Unix VFS already implements subtle process-local POSIX lock
//! bookkeeping and WAL shared-memory locking. This module preserves those I/O
//! methods and only replaces pathname resolution through SQLite's documented
//! system-call sandbox hooks.

#![cfg(unix)]

use std::{
    ffi::{CStr, CString, OsStr},
    io,
    os::{
        fd::RawFd,
        raw::{c_char, c_int},
        unix::ffi::OsStrExt,
    },
    panic::{AssertUnwindSafe, catch_unwind},
    path::Path,
    ptr,
    sync::{Mutex, OnceLock},
};

use rusqlite::ffi;

pub(crate) const VFS_NAME: &str = "rgit-pinned-unix";

const VFS_NAME_C: &[u8] = b"rgit-pinned-unix\0";
const WAL_SUFFIX: &[u8] = b"-wal";
const SHM_SUFFIX: &[u8] = b"-shm";
const JOURNAL_SUFFIX: &[u8] = b"-journal";

type OpenFn = unsafe extern "C" fn(*const c_char, c_int, c_int) -> c_int;
type AccessFn = unsafe extern "C" fn(*const c_char, c_int) -> c_int;
type StatFn = unsafe extern "C" fn(*const c_char, *mut libc::stat) -> c_int;
type UnlinkFn = unsafe extern "C" fn(*const c_char) -> c_int;

#[derive(Clone, Copy)]
struct OriginalCalls {
    open: OpenFn,
    access: AccessFn,
    stat: StatFn,
    lstat: StatFn,
    unlink: UnlinkFn,
}

struct Entry {
    directory_fd: RawFd,
    directory_path: Vec<u8>,
    database_path: Vec<u8>,
    database_name: Vec<u8>,
    wal_name: Vec<u8>,
    shm_name: Vec<u8>,
    journal_name: Vec<u8>,
    device: libc::dev_t,
    inode: libc::ino_t,
    references: usize,
}

#[derive(Default)]
struct Registry {
    entries: Vec<Entry>,
}

static ORIGINAL_CALLS: OnceLock<OriginalCalls> = OnceLock::new();
static VFS_INITIALIZATION: OnceLock<c_int> = OnceLock::new();
static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();

/// Keeps a descriptor-pinned metadata registration alive.
///
/// This guard must outlive every SQLite connection opened with [`VFS_NAME`]
/// for this path. The module duplicates the supplied descriptor.
pub(crate) struct PinnedSqliteRegistration {
    database_path: Vec<u8>,
}

impl PinnedSqliteRegistration {
    /// Pins an absolute logical database name to `metadata_directory_fd`.
    ///
    /// The caller is responsible for verifying that the logical parent
    /// originally names the supplied directory. All subsequent operations use
    /// the descriptor, so renaming or substituting that pathname is harmless.
    pub(crate) fn register(
        metadata_directory_fd: RawFd,
        logical_database: &Path,
    ) -> io::Result<Self> {
        ensure_vfs()?;
        if !logical_database.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "SQLite database path must be absolute",
            ));
        }
        let database_path = os_bytes(logical_database.as_os_str())?;
        let parent = logical_database.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "database path has no parent")
        })?;
        let database_name = os_bytes(logical_database.file_name().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "database path has no filename")
        })?)?;
        if database_name.is_empty() || database_name.contains(&b'/') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid SQLite database filename",
            ));
        }
        let directory_path = os_bytes(parent.as_os_str())?;

        let duplicate =
            unsafe { libc::fcntl(metadata_directory_fd, libc::F_DUPFD_CLOEXEC, 0 as c_int) };
        if duplicate < 0 {
            return Err(io::Error::last_os_error());
        }
        let mut metadata = unsafe { std::mem::zeroed::<libc::stat>() };
        if unsafe { libc::fstat(duplicate, &mut metadata) } != 0 {
            let error = io::Error::last_os_error();
            unsafe { libc::close(duplicate) };
            return Err(error);
        }
        if metadata.st_mode & libc::S_IFMT != libc::S_IFDIR {
            unsafe { libc::close(duplicate) };
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "metadata descriptor is not a directory",
            ));
        }

        let registry = REGISTRY.get_or_init(|| Mutex::new(Registry::default()));
        let mut registry = match registry.lock() {
            Ok(registry) => registry,
            Err(_) => {
                unsafe { libc::close(duplicate) };
                return Err(io::Error::other("SQLite VFS registry is poisoned"));
            }
        };
        if let Some(entry) = registry
            .entries
            .iter_mut()
            .find(|entry| entry.database_path == database_path)
        {
            if entry.device != metadata.st_dev || entry.inode != metadata.st_ino {
                unsafe { libc::close(duplicate) };
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "logical SQLite path is pinned to another directory",
                ));
            }
            let Some(references) = entry.references.checked_add(1) else {
                unsafe { libc::close(duplicate) };
                return Err(io::Error::other("SQLite registration count overflow"));
            };
            entry.references = references;
            unsafe { libc::close(duplicate) };
        } else {
            registry.entries.push(Entry {
                directory_fd: duplicate,
                directory_path,
                wal_name: with_suffix(&database_name, WAL_SUFFIX),
                shm_name: with_suffix(&database_name, SHM_SUFFIX),
                journal_name: with_suffix(&database_name, JOURNAL_SUFFIX),
                database_path: database_path.clone(),
                database_name,
                device: metadata.st_dev,
                inode: metadata.st_ino,
                references: 1,
            });
        }
        Ok(Self { database_path })
    }
}

impl Drop for PinnedSqliteRegistration {
    fn drop(&mut self) {
        let Some(registry) = REGISTRY.get() else {
            return;
        };
        let Ok(mut registry) = registry.lock() else {
            // A leak is safer than closing a descriptor after registry poison.
            return;
        };
        let Some(index) = registry
            .entries
            .iter()
            .position(|entry| entry.database_path == self.database_path)
        else {
            return;
        };
        if registry.entries[index].references > 1 {
            registry.entries[index].references -= 1;
        } else {
            let entry = registry.entries.swap_remove(index);
            unsafe { libc::close(entry.directory_fd) };
        }
    }
}

fn ensure_vfs() -> io::Result<()> {
    let result = *VFS_INITIALIZATION.get_or_init(|| unsafe { initialize_vfs() });
    if result == ffi::SQLITE_OK {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "pinned SQLite VFS initialization failed ({result})"
        )))
    }
}

unsafe fn initialize_vfs() -> c_int {
    let result = unsafe { ffi::sqlite3_initialize() };
    if result != ffi::SQLITE_OK {
        return result;
    }
    let base = unsafe { ffi::sqlite3_vfs_find(c_str(b"unix\0").as_ptr()) };
    if base.is_null() {
        return ffi::SQLITE_NOTFOUND;
    }
    let Some(get_call) = (unsafe { (*base).xGetSystemCall }) else {
        return ffi::SQLITE_NOTFOUND;
    };
    let Some(set_call) = (unsafe { (*base).xSetSystemCall }) else {
        return ffi::SQLITE_NOTFOUND;
    };

    macro_rules! original {
        ($name:literal, $kind:ty) => {{
            let Some(pointer) = (unsafe { get_call(base, c_str($name).as_ptr()) }) else {
                return ffi::SQLITE_NOTFOUND;
            };
            unsafe { std::mem::transmute::<unsafe extern "C" fn(), $kind>(pointer) }
        }};
    }
    let originals = OriginalCalls {
        open: original!(b"open\0", OpenFn),
        access: original!(b"access\0", AccessFn),
        stat: original!(b"stat\0", StatFn),
        lstat: original!(b"lstat\0", StatFn),
        unlink: original!(b"unlink\0", UnlinkFn),
    };
    if ORIGINAL_CALLS.set(originals).is_err() {
        return ffi::SQLITE_MISUSE;
    }

    let replacements = [
        (c_str(b"open\0"), unsafe { erase_open(pinned_open) }),
        (c_str(b"access\0"), unsafe { erase_access(pinned_access) }),
        (c_str(b"stat\0"), unsafe { erase_stat(pinned_stat) }),
        (c_str(b"lstat\0"), unsafe { erase_stat(pinned_lstat) }),
        (c_str(b"unlink\0"), unsafe { erase_unlink(pinned_unlink) }),
    ];
    let mut installed = 0;
    for (name, replacement) in replacements {
        let result = unsafe { set_call(base, name.as_ptr(), Some(replacement)) };
        if result != ffi::SQLITE_OK {
            unsafe { restore_calls(base, set_call, originals, installed) };
            return result;
        }
        installed += 1;
    }

    let mut vfs = unsafe { *base };
    vfs.pNext = ptr::null_mut();
    vfs.zName = VFS_NAME_C.as_ptr().cast();
    vfs.xFullPathname = Some(pinned_full_pathname);
    vfs.xDlOpen = None;
    vfs.xDlError = None;
    vfs.xDlSym = None;
    vfs.xDlClose = None;
    let vfs = Box::into_raw(Box::new(vfs));
    let result = unsafe { ffi::sqlite3_vfs_register(vfs, 0) };
    if result != ffi::SQLITE_OK {
        unsafe { drop(Box::from_raw(vfs)) };
        unsafe { restore_calls(base, set_call, originals, installed) };
    }
    result
}

unsafe fn restore_calls(
    base: *mut ffi::sqlite3_vfs,
    set_call: unsafe extern "C" fn(
        *mut ffi::sqlite3_vfs,
        *const c_char,
        ffi::sqlite3_syscall_ptr,
    ) -> c_int,
    originals: OriginalCalls,
    installed: usize,
) {
    let values = [
        (c_str(b"open\0"), unsafe { erase_open(originals.open) }),
        (c_str(b"access\0"), unsafe {
            erase_access(originals.access)
        }),
        (c_str(b"stat\0"), unsafe { erase_stat(originals.stat) }),
        (c_str(b"lstat\0"), unsafe { erase_stat(originals.lstat) }),
        (c_str(b"unlink\0"), unsafe {
            erase_unlink(originals.unlink)
        }),
    ];
    for (name, original) in values.into_iter().take(installed) {
        let _ = unsafe { set_call(base, name.as_ptr(), Some(original)) };
    }
}

unsafe fn erase_open(function: OpenFn) -> unsafe extern "C" fn() {
    unsafe { std::mem::transmute::<OpenFn, unsafe extern "C" fn()>(function) }
}
unsafe fn erase_access(function: AccessFn) -> unsafe extern "C" fn() {
    unsafe { std::mem::transmute::<AccessFn, unsafe extern "C" fn()>(function) }
}
unsafe fn erase_stat(function: StatFn) -> unsafe extern "C" fn() {
    unsafe { std::mem::transmute::<StatFn, unsafe extern "C" fn()>(function) }
}
unsafe fn erase_unlink(function: UnlinkFn) -> unsafe extern "C" fn() {
    unsafe { std::mem::transmute::<UnlinkFn, unsafe extern "C" fn()>(function) }
}

unsafe extern "C" fn pinned_open(path: *const c_char, flags: c_int, mode: c_int) -> c_int {
    ffi_boundary(-1, || unsafe { pinned_open_inner(path, flags, mode) })
}

unsafe fn pinned_open_inner(path: *const c_char, flags: c_int, mode: c_int) -> c_int {
    let Some(bytes) = (unsafe { path_bytes(path) }) else {
        set_errno(libc::EINVAL);
        return -1;
    };
    let Some(registry) = REGISTRY.get() else {
        return unsafe { (originals().open)(path, flags, mode) };
    };
    let Ok(registry) = registry.lock() else {
        set_errno(libc::EIO);
        return -1;
    };
    match classify(&registry, bytes) {
        Match::File(entry, name) => {
            let Ok(name) = CString::new(name) else {
                set_errno(libc::EINVAL);
                return -1;
            };
            let fd = unsafe {
                libc::openat(
                    entry.directory_fd,
                    name.as_ptr(),
                    flags | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                    0o600,
                )
            };
            if fd < 0 {
                return -1;
            }
            if regular_single_link(fd) {
                fd
            } else {
                unsafe { libc::close(fd) };
                set_errno(libc::ELOOP);
                -1
            }
        }
        Match::Directory(entry) => unsafe {
            libc::fcntl(entry.directory_fd, libc::F_DUPFD_CLOEXEC, 0)
        },
        Match::Reject => reject(),
        Match::Unrelated => unsafe { (originals().open)(path, flags, mode) },
    }
}

unsafe extern "C" fn pinned_stat(path: *const c_char, output: *mut libc::stat) -> c_int {
    ffi_boundary(-1, || unsafe { pinned_stat_inner(path, output, false) })
}

unsafe extern "C" fn pinned_lstat(path: *const c_char, output: *mut libc::stat) -> c_int {
    ffi_boundary(-1, || unsafe { pinned_stat_inner(path, output, true) })
}

unsafe fn pinned_stat_inner(
    path: *const c_char,
    output: *mut libc::stat,
    use_lstat: bool,
) -> c_int {
    if output.is_null() {
        set_errno(libc::EINVAL);
        return -1;
    }
    let Some(bytes) = (unsafe { path_bytes(path) }) else {
        set_errno(libc::EINVAL);
        return -1;
    };
    let Some(registry) = REGISTRY.get() else {
        return unsafe { call_original_stat(path, output, use_lstat) };
    };
    let Ok(registry) = registry.lock() else {
        set_errno(libc::EIO);
        return -1;
    };
    match classify(&registry, bytes) {
        Match::File(entry, name) => {
            let Ok(name) = CString::new(name) else {
                set_errno(libc::EINVAL);
                return -1;
            };
            let result = unsafe {
                libc::fstatat(
                    entry.directory_fd,
                    name.as_ptr(),
                    output,
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            };
            if result == 0 && !stat_is_regular_single_link(unsafe { &*output }) {
                set_errno(libc::ELOOP);
                -1
            } else {
                result
            }
        }
        Match::Directory(entry) => unsafe { libc::fstat(entry.directory_fd, output) },
        Match::Reject => reject(),
        Match::Unrelated => unsafe { call_original_stat(path, output, use_lstat) },
    }
}

unsafe fn call_original_stat(
    path: *const c_char,
    output: *mut libc::stat,
    use_lstat: bool,
) -> c_int {
    if use_lstat {
        unsafe { (originals().lstat)(path, output) }
    } else {
        unsafe { (originals().stat)(path, output) }
    }
}

unsafe extern "C" fn pinned_access(path: *const c_char, mode: c_int) -> c_int {
    ffi_boundary(-1, || unsafe { pinned_access_inner(path, mode) })
}

unsafe fn pinned_access_inner(path: *const c_char, mode: c_int) -> c_int {
    let Some(bytes) = (unsafe { path_bytes(path) }) else {
        set_errno(libc::EINVAL);
        return -1;
    };
    let Some(registry) = REGISTRY.get() else {
        return unsafe { (originals().access)(path, mode) };
    };
    let Ok(registry) = registry.lock() else {
        set_errno(libc::EIO);
        return -1;
    };
    match classify(&registry, bytes) {
        Match::File(entry, name) => access_file(entry, name, mode),
        Match::Directory(entry) => {
            let mut metadata = unsafe { std::mem::zeroed::<libc::stat>() };
            unsafe { libc::fstat(entry.directory_fd, &mut metadata) }
        }
        Match::Reject => reject(),
        Match::Unrelated => unsafe { (originals().access)(path, mode) },
    }
}

fn access_file(entry: &Entry, name: &[u8], mode: c_int) -> c_int {
    let Ok(name) = CString::new(name) else {
        set_errno(libc::EINVAL);
        return -1;
    };
    if mode == libc::F_OK {
        let mut metadata = unsafe { std::mem::zeroed::<libc::stat>() };
        let result = unsafe {
            libc::fstatat(
                entry.directory_fd,
                name.as_ptr(),
                &mut metadata,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if result == 0 && !stat_is_regular_single_link(&metadata) {
            set_errno(libc::ELOOP);
            -1
        } else {
            result
        }
    } else {
        let access = if mode & libc::W_OK != 0 {
            libc::O_RDWR
        } else {
            libc::O_RDONLY
        };
        let fd = unsafe {
            libc::openat(
                entry.directory_fd,
                name.as_ptr(),
                access | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return -1;
        }
        let valid = regular_single_link(fd);
        unsafe { libc::close(fd) };
        if valid {
            0
        } else {
            set_errno(libc::ELOOP);
            -1
        }
    }
}

unsafe extern "C" fn pinned_unlink(path: *const c_char) -> c_int {
    ffi_boundary(-1, || unsafe { pinned_unlink_inner(path) })
}

unsafe fn pinned_unlink_inner(path: *const c_char) -> c_int {
    let Some(bytes) = (unsafe { path_bytes(path) }) else {
        set_errno(libc::EINVAL);
        return -1;
    };
    let Some(registry) = REGISTRY.get() else {
        return unsafe { (originals().unlink)(path) };
    };
    let Ok(registry) = registry.lock() else {
        set_errno(libc::EIO);
        return -1;
    };
    match classify(&registry, bytes) {
        Match::File(entry, name) => {
            let Ok(name) = CString::new(name) else {
                set_errno(libc::EINVAL);
                return -1;
            };
            let mut metadata = unsafe { std::mem::zeroed::<libc::stat>() };
            let inspected = unsafe {
                libc::fstatat(
                    entry.directory_fd,
                    name.as_ptr(),
                    &mut metadata,
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            };
            if inspected == 0 && !stat_is_regular_single_link(&metadata) {
                set_errno(libc::ELOOP);
                return -1;
            }
            if inspected != 0 && io::Error::last_os_error().kind() != io::ErrorKind::NotFound {
                return -1;
            }
            unsafe { libc::unlinkat(entry.directory_fd, name.as_ptr(), 0) }
        }
        Match::Directory(_) | Match::Reject => reject(),
        Match::Unrelated => unsafe { (originals().unlink)(path) },
    }
}

unsafe extern "C" fn pinned_full_pathname(
    _vfs: *mut ffi::sqlite3_vfs,
    path: *const c_char,
    output_len: c_int,
    output: *mut c_char,
) -> c_int {
    ffi_boundary(ffi::SQLITE_CANTOPEN, || {
        if output.is_null() || output_len <= 0 {
            return ffi::SQLITE_CANTOPEN;
        }
        let Some(path) = (unsafe { path_bytes(path) }) else {
            return ffi::SQLITE_CANTOPEN;
        };
        let Some(registry) = REGISTRY.get() else {
            return ffi::SQLITE_CANTOPEN;
        };
        let Ok(registry) = registry.lock() else {
            return ffi::SQLITE_CANTOPEN;
        };
        if !registry
            .entries
            .iter()
            .any(|entry| entry.database_path == path)
            || path.len().saturating_add(1) > output_len as usize
        {
            return ffi::SQLITE_CANTOPEN;
        }
        unsafe {
            ptr::copy_nonoverlapping(path.as_ptr(), output.cast(), path.len());
            *output.add(path.len()) = 0;
        }
        ffi::SQLITE_OK
    })
}

enum Match<'a> {
    File(&'a Entry, &'a [u8]),
    Directory(&'a Entry),
    Reject,
    Unrelated,
}

fn classify<'a>(registry: &'a Registry, path: &[u8]) -> Match<'a> {
    for entry in &registry.entries {
        if path == entry.directory_path {
            return Match::Directory(entry);
        }
        if path == entry.database_path {
            return Match::File(entry, &entry.database_name);
        }
        if path_matches_sidecar(path, &entry.database_path, WAL_SUFFIX) {
            return Match::File(entry, &entry.wal_name);
        }
        if path_matches_sidecar(path, &entry.database_path, SHM_SUFFIX) {
            return Match::File(entry, &entry.shm_name);
        }
        if path_matches_sidecar(path, &entry.database_path, JOURNAL_SUFFIX) {
            return Match::File(entry, &entry.journal_name);
        }
    }
    if registry
        .entries
        .iter()
        .any(|entry| is_child_of(path, &entry.directory_path))
    {
        Match::Reject
    } else {
        Match::Unrelated
    }
}

fn path_matches_sidecar(path: &[u8], database: &[u8], suffix: &[u8]) -> bool {
    path.len() == database.len() + suffix.len()
        && path.starts_with(database)
        && path.ends_with(suffix)
}

fn is_child_of(path: &[u8], directory: &[u8]) -> bool {
    path.strip_prefix(directory)
        .is_some_and(|suffix| suffix.first() == Some(&b'/'))
}

fn with_suffix(value: &[u8], suffix: &[u8]) -> Vec<u8> {
    let mut result = Vec::with_capacity(value.len() + suffix.len());
    result.extend_from_slice(value);
    result.extend_from_slice(suffix);
    result
}

fn regular_single_link(fd: RawFd) -> bool {
    let mut metadata = unsafe { std::mem::zeroed::<libc::stat>() };
    (unsafe { libc::fstat(fd, &mut metadata) }) == 0 && stat_is_regular_single_link(&metadata)
}

fn stat_is_regular_single_link(metadata: &libc::stat) -> bool {
    metadata.st_mode & libc::S_IFMT == libc::S_IFREG && metadata.st_nlink == 1
}

fn originals() -> &'static OriginalCalls {
    ORIGINAL_CALLS
        .get()
        .expect("callbacks are installed only after originals are saved")
}

fn ffi_boundary<T: Copy>(failure: T, operation: impl FnOnce() -> T) -> T {
    match catch_unwind(AssertUnwindSafe(operation)) {
        Ok(value) => value,
        Err(_) => {
            set_errno(libc::EIO);
            failure
        }
    }
}

unsafe fn path_bytes<'a>(path: *const c_char) -> Option<&'a [u8]> {
    if path.is_null() {
        None
    } else {
        Some(unsafe { CStr::from_ptr(path) }.to_bytes())
    }
}

fn os_bytes(value: &OsStr) -> io::Result<Vec<u8>> {
    let bytes = value.as_bytes();
    if bytes.contains(&0) {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path contains NUL",
        ))
    } else {
        Ok(bytes.to_vec())
    }
}

const fn c_str(bytes: &'static [u8]) -> &'static CStr {
    // All call sites pass static strings with exactly one trailing NUL.
    unsafe { CStr::from_bytes_with_nul_unchecked(bytes) }
}

fn reject() -> c_int {
    set_errno(libc::EACCES);
    -1
}

#[cfg(target_os = "linux")]
fn set_errno(value: c_int) {
    unsafe { *libc::__errno_location() = value };
}

#[cfg(not(target_os = "linux"))]
fn set_errno(value: c_int) {
    unsafe { *libc::__error() = value };
}

#[cfg(test)]
mod tests {
    use std::{
        error::Error,
        fs::{self, File},
        os::{fd::AsRawFd, unix::fs::symlink},
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    use rusqlite::{Connection, OpenFlags};

    use super::{PinnedSqliteRegistration, VFS_NAME, pinned_open};

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn create(name: &str) -> std::io::Result<Self> {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            for _ in 0..100 {
                let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir().join(format!(
                    "rgit-sqlite-vfs-{name}-{}-{nonce}-{sequence}",
                    std::process::id()
                ));
                match fs::create_dir(&path) {
                    Ok(()) => return Ok(Self(path)),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(error),
                }
            }
            Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "could not allocate unique test directory",
            ))
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn sqlite_flags() -> OpenFlags {
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
    }

    #[test]
    fn renamed_metadata_directory_cannot_redirect_database_open() -> Result<(), Box<dyn Error>> {
        let root = TestDirectory::create("rename")?;
        let logical_metadata = root.path().join("metadata");
        let pinned_metadata = root.path().join("metadata-pinned");
        fs::create_dir(&logical_metadata)?;
        let directory = File::open(&logical_metadata)?;
        let database = logical_metadata.join("repository.sqlite3");
        let registration = PinnedSqliteRegistration::register(directory.as_raw_fd(), &database)?;

        fs::rename(&logical_metadata, &pinned_metadata)?;
        fs::create_dir(&logical_metadata)?;
        fs::write(logical_metadata.join("replacement-marker"), b"attacker")?;

        let connection = Connection::open_with_flags_and_vfs(&database, sqlite_flags(), VFS_NAME)?;
        connection.execute_batch(
            "CREATE TABLE pinned(value INTEGER NOT NULL);
             INSERT INTO pinned(value) VALUES(7);",
        )?;
        drop(connection);

        assert!(pinned_metadata.join("repository.sqlite3").is_file());
        assert!(!logical_metadata.join("repository.sqlite3").exists());
        assert_eq!(
            fs::read(logical_metadata.join("replacement-marker"))?,
            b"attacker"
        );
        drop(registration);
        Ok(())
    }

    #[test]
    fn wal_and_shm_reject_symlink_and_hardlink_entries() -> Result<(), Box<dyn Error>> {
        let root = TestDirectory::create("sidecars")?;
        let metadata = root.path().join("metadata");
        fs::create_dir(&metadata)?;
        let directory = File::open(&metadata)?;
        let database = metadata.join("repository.sqlite3");
        let registration = PinnedSqliteRegistration::register(directory.as_raw_fd(), &database)?;
        let victim = root.path().join("victim");
        fs::write(&victim, b"must remain unchanged")?;

        for suffix in ["-wal", "-shm"] {
            let sidecar = metadata.join(format!("repository.sqlite3{suffix}"));
            symlink(&victim, &sidecar)?;
            assert_open_rejected(&sidecar)?;
            fs::remove_file(&sidecar)?;

            fs::hard_link(&victim, &sidecar)?;
            assert_open_rejected(&sidecar)?;
            fs::remove_file(&sidecar)?;
        }
        assert_eq!(fs::read(&victim)?, b"must remain unchanged");
        drop(registration);
        Ok(())
    }

    fn assert_open_rejected(path: &Path) -> Result<(), Box<dyn Error>> {
        let path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes())?;
        let descriptor = unsafe { pinned_open(path.as_ptr(), libc::O_RDWR, 0o600) };
        if descriptor >= 0 {
            unsafe { libc::close(descriptor) };
            return Err("unsafe SQLite sidecar entry was opened".into());
        }
        Ok(())
    }
}
