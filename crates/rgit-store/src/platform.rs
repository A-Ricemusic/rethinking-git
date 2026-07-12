use std::{fs::File, io, path::Path};

#[cfg(unix)]
use std::ffi::CString;

#[cfg(unix)]
use std::os::{
    fd::{AsRawFd, FromRawFd},
    unix::ffi::OsStrExt,
};

#[cfg(unix)]
pub(crate) struct DirectoryHandle(File);

#[cfg(windows)]
pub(crate) struct DirectoryHandle;

#[cfg(unix)]
impl DirectoryHandle {
    pub(crate) fn try_clone(&self) -> io::Result<Self> {
        self.0.try_clone().map(Self)
    }

    pub(crate) fn metadata(&self) -> io::Result<std::fs::Metadata> {
        self.0.metadata()
    }

    pub(crate) fn raw_fd(&self) -> std::os::fd::RawFd {
        self.0.as_raw_fd()
    }
}

#[cfg(windows)]
impl DirectoryHandle {
    pub(crate) fn try_clone(&self) -> io::Result<Self> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "NTFS directory handle adapter is not qualified",
        ))
    }

    pub(crate) fn raw_fd(&self) -> i32 {
        -1
    }
}

#[cfg(unix)]
fn directory_flags() -> std::ffi::c_int {
    #[cfg(target_os = "macos")]
    {
        0x0010_0000 | 0x0000_0100 | 0x0100_0000
    }
    #[cfg(not(target_os = "macos"))]
    {
        0x0001_0000 | 0x0002_0000 | 0x0008_0000
    }
}

#[cfg(unix)]
fn file_flags() -> std::ffi::c_int {
    #[cfg(target_os = "macos")]
    {
        0x0002 | 0x0200 | 0x0800 | 0x0000_0100 | 0x0100_0000
    }
    #[cfg(not(target_os = "macos"))]
    {
        0x0002 | 0x0040 | 0x0080 | 0x0002_0000 | 0x0008_0000
    }
}

#[cfg(unix)]
unsafe extern "C" {
    fn open(path: *const std::ffi::c_char, flags: std::ffi::c_int, ...) -> std::ffi::c_int;
    fn openat(
        fd: std::ffi::c_int,
        path: *const std::ffi::c_char,
        flags: std::ffi::c_int,
        ...
    ) -> std::ffi::c_int;
    fn mkdirat(
        fd: std::ffi::c_int,
        path: *const std::ffi::c_char,
        mode: std::ffi::c_uint,
    ) -> std::ffi::c_int;
    fn unlinkat(
        fd: std::ffi::c_int,
        path: *const std::ffi::c_char,
        flags: std::ffi::c_int,
    ) -> std::ffi::c_int;
    fn flock(fd: std::ffi::c_int, operation: std::ffi::c_int) -> std::ffi::c_int;
    fn dup(fd: std::ffi::c_int) -> std::ffi::c_int;
    fn fdopendir(fd: std::ffi::c_int) -> *mut CDirectory;
    fn readdir(directory: *mut CDirectory) -> *mut CDirent;
    fn closedir(directory: *mut CDirectory) -> std::ffi::c_int;
}

#[cfg(unix)]
#[repr(C)]
struct CDirectory {
    _private: [u8; 0],
}

#[cfg(target_os = "macos")]
#[repr(C)]
struct CDirent {
    inode: u64,
    seek_offset: u64,
    record_length: u16,
    name_length: u16,
    entry_type: u8,
    name: [std::ffi::c_char; 1024],
}

#[cfg(all(unix, not(target_os = "macos")))]
#[repr(C)]
struct CDirent {
    inode: u64,
    offset: i64,
    record_length: u16,
    entry_type: u8,
    name: [std::ffi::c_char; 256],
}

pub(crate) fn open_directory(path: &Path) -> io::Result<DirectoryHandle> {
    #[cfg(unix)]
    {
        let path = CString::new(path.as_os_str().as_bytes())?;
        let fd = unsafe { open(path.as_ptr(), directory_flags()) };
        if fd < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(DirectoryHandle(unsafe { File::from_raw_fd(fd) }))
        }
    }
    #[cfg(windows)]
    {
        let _ = path;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "NTFS directory handle adapter is not qualified",
        ))
    }
}

pub(crate) fn open_directory_at(
    parent: &DirectoryHandle,
    name: &str,
) -> io::Result<DirectoryHandle> {
    #[cfg(unix)]
    {
        let name = CString::new(name)?;
        let fd = unsafe { openat(parent.0.as_raw_fd(), name.as_ptr(), directory_flags()) };
        if fd < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(DirectoryHandle(unsafe { File::from_raw_fd(fd) }))
        }
    }
    #[cfg(windows)]
    {
        let _ = (parent, name);
        Err(io::Error::new(io::ErrorKind::Unsupported, "unsupported"))
    }
}

pub(crate) fn create_directory_at(parent: &DirectoryHandle, name: &str) -> io::Result<bool> {
    #[cfg(unix)]
    {
        let name = CString::new(name)?;
        let result = unsafe { mkdirat(parent.0.as_raw_fd(), name.as_ptr(), 0o700) };
        if result == 0 {
            Ok(true)
        } else {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::AlreadyExists {
                Ok(false)
            } else {
                Err(error)
            }
        }
    }
    #[cfg(windows)]
    {
        let _ = (parent, name);
        Err(io::Error::new(io::ErrorKind::Unsupported, "unsupported"))
    }
}

pub(crate) fn create_file_at(parent: &DirectoryHandle, name: &str) -> io::Result<File> {
    #[cfg(unix)]
    {
        let name = CString::new(name)?;
        let fd = unsafe { openat(parent.0.as_raw_fd(), name.as_ptr(), file_flags(), 0o600_u32) };
        if fd < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(unsafe { File::from_raw_fd(fd) })
        }
    }
    #[cfg(windows)]
    {
        let _ = (parent, name);
        Err(io::Error::new(io::ErrorKind::Unsupported, "unsupported"))
    }
}

pub(crate) fn open_lock_file_at(parent: &DirectoryHandle, name: &str) -> io::Result<File> {
    #[cfg(unix)]
    {
        let name = CString::new(name)?;
        #[cfg(target_os = "macos")]
        let flags = 0x0002 | 0x0200 | 0x0000_0100 | 0x0100_0000;
        #[cfg(not(target_os = "macos"))]
        let flags = 0x0002 | 0x0040 | 0x0002_0000 | 0x0008_0000;
        let fd = unsafe { openat(parent.0.as_raw_fd(), name.as_ptr(), flags, 0o600_u32) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let file = unsafe { File::from_raw_fd(fd) };
        use std::os::unix::fs::MetadataExt;
        let metadata = file.metadata()?;
        if !metadata.file_type().is_file() || metadata.nlink() != 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unsafe lock entry",
            ));
        }
        Ok(file)
    }
    #[cfg(windows)]
    {
        let _ = (parent, name);
        Err(io::Error::new(io::ErrorKind::Unsupported, "unsupported"))
    }
}

pub(crate) struct ExclusiveFileLock {
    #[cfg(unix)]
    file: File,
}

pub(crate) fn try_lock_exclusive(file: File) -> io::Result<ExclusiveFileLock> {
    #[cfg(unix)]
    {
        if unsafe { flock(file.as_raw_fd(), 0x02 | 0x04) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(ExclusiveFileLock { file })
    }
    #[cfg(windows)]
    {
        let _ = file;
        Err(io::Error::new(io::ErrorKind::Unsupported, "unsupported"))
    }
}

#[cfg(unix)]
impl Drop for ExclusiveFileLock {
    fn drop(&mut self) {
        let _ = unsafe { flock(self.file.as_raw_fd(), 0x08) };
    }
}

pub(crate) fn open_file_at(parent: &DirectoryHandle, name: &str) -> io::Result<File> {
    #[cfg(unix)]
    {
        let name = CString::new(name)?;
        #[cfg(target_os = "macos")]
        let flags = 0x0000_0100 | 0x0100_0000;
        #[cfg(not(target_os = "macos"))]
        let flags = 0x0002_0000 | 0x0008_0000;
        let fd = unsafe { openat(parent.0.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(unsafe { File::from_raw_fd(fd) })
        }
    }
    #[cfg(windows)]
    {
        let _ = (parent, name);
        Err(io::Error::new(io::ErrorKind::Unsupported, "unsupported"))
    }
}

pub(crate) fn remove_file_at(parent: &DirectoryHandle, name: &str) -> io::Result<()> {
    #[cfg(unix)]
    {
        let name = CString::new(name)?;
        if unsafe { unlinkat(parent.0.as_raw_fd(), name.as_ptr(), 0) } == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
    #[cfg(windows)]
    {
        let _ = (parent, name);
        Err(io::Error::new(io::ErrorKind::Unsupported, "unsupported"))
    }
}

pub(crate) fn sync_handle(directory: &DirectoryHandle) -> io::Result<()> {
    #[cfg(unix)]
    {
        directory.0.sync_all()
    }
    #[cfg(windows)]
    {
        let _ = directory;
        Err(io::Error::new(io::ErrorKind::Unsupported, "unsupported"))
    }
}

pub(crate) fn list_directory_names(directory: &DirectoryHandle) -> io::Result<Vec<String>> {
    #[cfg(unix)]
    {
        let duplicated = unsafe { dup(directory.0.as_raw_fd()) };
        if duplicated < 0 {
            return Err(io::Error::last_os_error());
        }
        let stream = unsafe { fdopendir(duplicated) };
        if stream.is_null() {
            let error = io::Error::last_os_error();
            let _ = unsafe { File::from_raw_fd(duplicated) };
            return Err(error);
        }
        let mut names = Vec::new();
        loop {
            let entry = unsafe { readdir(stream) };
            if entry.is_null() {
                break;
            }
            let name = match unsafe { std::ffi::CStr::from_ptr((*entry).name.as_ptr()) }.to_str() {
                Ok(name) => name,
                Err(_) => {
                    let _ = unsafe { closedir(stream) };
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "non-UTF-8 entry",
                    ));
                }
            };
            if name != "." && name != ".." {
                names.push(name.to_owned());
            }
        }
        if unsafe { closedir(stream) } != 0 {
            return Err(io::Error::last_os_error());
        }
        names.sort();
        Ok(names)
    }
    #[cfg(windows)]
    {
        let _ = directory;
        Err(io::Error::new(io::ErrorKind::Unsupported, "unsupported"))
    }
}

pub(crate) fn rename_no_replace_at(
    source_parent: &DirectoryHandle,
    source_name: &str,
    destination_parent: &DirectoryHandle,
    destination_name: &str,
) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        unsafe extern "C" {
            fn renameat2(
                olddirfd: std::ffi::c_int,
                oldpath: *const std::ffi::c_char,
                newdirfd: std::ffi::c_int,
                newpath: *const std::ffi::c_char,
                flags: std::ffi::c_uint,
            ) -> std::ffi::c_int;
        }
        let source = CString::new(source_name)?;
        let destination = CString::new(destination_name)?;
        let result = unsafe {
            renameat2(
                source_parent.0.as_raw_fd(),
                source.as_ptr(),
                destination_parent.0.as_raw_fd(),
                destination.as_ptr(),
                1,
            )
        };
        if result == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
    #[cfg(target_os = "macos")]
    {
        unsafe extern "C" {
            fn renameatx_np(
                fromfd: std::ffi::c_int,
                from: *const std::ffi::c_char,
                tofd: std::ffi::c_int,
                to: *const std::ffi::c_char,
                flags: std::ffi::c_uint,
            ) -> std::ffi::c_int;
        }
        let source = CString::new(source_name)?;
        let destination = CString::new(destination_name)?;
        let result = unsafe {
            renameatx_np(
                source_parent.0.as_raw_fd(),
                source.as_ptr(),
                destination_parent.0.as_raw_fd(),
                destination.as_ptr(),
                0x4,
            )
        };
        if result == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
    #[cfg(windows)]
    {
        let _ = (
            source_parent,
            source_name,
            destination_parent,
            destination_name,
        );
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "NTFS relative rename adapter is not qualified",
        ))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FileIdentity {
    filesystem: u64,
    file: u64,
    length: u64,
}

#[cfg(unix)]
pub(crate) fn file_identity(file: &File) -> io::Result<FileIdentity> {
    use std::os::unix::fs::MetadataExt;

    let metadata = file.metadata()?;
    Ok(FileIdentity {
        filesystem: metadata.dev(),
        file: metadata.ino(),
        length: metadata.len(),
    })
}

#[cfg(windows)]
pub(crate) fn file_identity(file: &File) -> io::Result<FileIdentity> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };

    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: `file` owns a live Windows handle for the duration of this call,
    // and `information` points to writable storage of the exact API type.
    let result =
        unsafe { GetFileInformationByHandle(file.as_raw_handle().cast(), &mut information) };
    if result == 0 {
        return Err(io::Error::last_os_error());
    }

    Ok(FileIdentity {
        filesystem: u64::from(information.dwVolumeSerialNumber),
        file: u64::from(information.nFileIndexHigh) << 32 | u64::from(information.nFileIndexLow),
        length: u64::from(information.nFileSizeHigh) << 32 | u64::from(information.nFileSizeLow),
    })
}

pub(crate) fn same_file(expected: &FileIdentity, observed: &File) -> io::Result<bool> {
    Ok(*expected == file_identity(observed)?)
}

pub(crate) fn same_entry(expected: &FileIdentity, observed: &File) -> io::Result<bool> {
    let observed = file_identity(observed)?;
    Ok(expected.filesystem == observed.filesystem && expected.file == observed.file)
}

pub(crate) fn same_directory_entry(
    expected: &DirectoryHandle,
    observed: &DirectoryHandle,
) -> io::Result<bool> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let expected = expected.metadata()?;
        let observed = observed.metadata()?;
        Ok(expected.dev() == observed.dev() && expected.ino() == observed.ino())
    }
    #[cfg(windows)]
    {
        let _ = (expected, observed);
        Ok(false)
    }
}

pub(crate) fn regular_single_link(file: &File) -> io::Result<bool> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = file.metadata()?;
        Ok(metadata.file_type().is_file() && metadata.nlink() == 1)
    }
    #[cfg(windows)]
    {
        let _ = file;
        Ok(false)
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::{file_identity, same_file};
    use std::{
        fs::{self, OpenOptions},
        io::Write,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    static TEST_DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn create() -> Self {
            let sequence = TEST_DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "rgit-platform-test-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("create test directory");
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).expect("remove test directory");
        }
    }

    #[test]
    fn handle_identity_matches_clones_but_not_distinct_files() {
        let directory = TestDirectory::create();
        let first_path = directory.0.join("first");
        let second_path = directory.0.join("second");
        let mut first = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(first_path)
            .expect("create first file");
        first.write_all(b"same length").expect("write first file");
        let first_clone = first.try_clone().expect("clone first handle");
        let first_identity = file_identity(&first).expect("inspect first handle");
        let mut second = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(second_path)
            .expect("create second file");
        second.write_all(b"same length").expect("write second file");

        assert!(same_file(&first_identity, &first_clone).expect("compare cloned handle"));
        assert!(!same_file(&first_identity, &second).expect("compare distinct handles"));
        first_clone.set_len(1).expect("truncate cloned handle");
        assert!(!same_file(&first_identity, &first_clone).expect("detect changed length"));
    }
}
