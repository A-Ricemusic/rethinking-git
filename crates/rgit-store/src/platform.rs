use std::{ffi::CString, fs::File, io, path::Path};

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
}

#[cfg(windows)]
impl DirectoryHandle {
    pub(crate) fn try_clone(&self) -> io::Result<Self> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "NTFS directory handle adapter is not qualified",
        ))
    }

    pub(crate) fn metadata(&self) -> io::Result<std::fs::Metadata> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "NTFS directory handle adapter is not qualified",
        ))
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

#[cfg(unix)]
pub(crate) fn same_file(left: &std::fs::Metadata, right: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    left.dev() == right.dev() && left.ino() == right.ino() && left.len() == right.len()
}

#[cfg(windows)]
pub(crate) fn same_file(left: &std::fs::Metadata, right: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    left.volume_serial_number() == right.volume_serial_number()
        && left.file_index() == right.file_index()
        && left.file_size() == right.file_size()
}
