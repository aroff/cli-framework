//! Race-resistant private files for credentials and other user-only data.
//!
//! Creation applies the private security policy to the new inode/Windows file
//! object atomically. Opening validates the already-open handle rather than
//! checking a path and then following it. The policy is deliberately strict:
//! the current user must own the file, the final path component must not be a
//! symlink/reparse point, and no unprivileged principal may read it.
//!
//! Callers must place these files in a directory controlled by the current
//! user. These helpers protect file contents and reject a substituted final
//! component, but they cannot prevent a principal that can rename ancestors
//! from causing denial of service or redirecting the whole parent path.

use std::fs::File;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Create a new user-private regular file for reading and writing.
///
/// This never opens or truncates an existing path. On Unix the file is mode
/// `0600`, owned by the effective user, and opened with `O_NOFOLLOW`. On
/// Windows its protected DACL grants access only to the current user, Local
/// System, and Builtin Administrators.
pub fn create_new_private(path: impl AsRef<Path>) -> io::Result<File> {
    imp::create_new_private(path.as_ref())
}

/// Open an existing user-private regular file for reading.
///
/// Validation is performed on the returned file handle, closing the
/// check/use race that path metadata checks create.
pub fn open_private(path: impl AsRef<Path>) -> io::Result<File> {
    imp::open_private(path.as_ref())
}

/// Atomically replace `path` with a newly-created private file containing
/// `bytes`.
///
/// The temporary file is created in the same directory, flushed before the
/// rename, and removed on failure. The parent directory must be controlled by
/// the current user as described in the module-level security boundary.
pub fn replace_private(path: impl AsRef<Path>, bytes: &[u8]) -> io::Result<()> {
    let path = path.as_ref();
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "private path has no parent"))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("private");

    let mut last_collision = None;
    for _ in 0..128 {
        let attempt = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temp = parent.join(format!(".{name}.tmp.{}.{attempt}", std::process::id()));
        match create_new_private(&temp) {
            Ok(mut file) => {
                use std::io::Write;
                let result = (|| {
                    file.write_all(bytes)?;
                    file.sync_all()?;
                    drop(file);
                    imp::replace(&temp, path)
                })();
                if result.is_err() {
                    let _ = std::fs::remove_file(&temp);
                }
                return result;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                last_collision = Some(error);
            }
            Err(error) => return Err(error),
        }
    }

    Err(last_collision.unwrap_or_else(|| {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not allocate a private temporary file",
        )
    }))
}

fn policy_error(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}

#[cfg(unix)]
mod imp {
    use super::{policy_error, File, Path};
    use std::io;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

    pub(super) fn create_new_private(path: &Path) -> io::Result<File> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(path)?;
        validate(&file)?;
        Ok(file)
    }

    pub(super) fn open_private(path: &Path) -> io::Result<File> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            // O_NONBLOCK prevents a substituted FIFO/device from hanging the
            // process before handle metadata can reject the non-regular file.
            // It does not change ordinary regular-file reads.
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)?;
        validate(&file)?;
        Ok(file)
    }

    fn validate(file: &File) -> io::Result<()> {
        let metadata = file.metadata()?;
        if !metadata.file_type().is_file() {
            return Err(policy_error("private path is not a regular file"));
        }
        // SAFETY: geteuid has no preconditions and does not borrow memory.
        if metadata.uid() != unsafe { libc::geteuid() } {
            return Err(policy_error(
                "private file is not owned by the current user",
            ));
        }
        if metadata.mode() & 0o077 != 0 {
            return Err(policy_error(
                "private file grants permissions to group or other users",
            ));
        }
        Ok(())
    }

    pub(super) fn replace(source: &Path, target: &Path) -> io::Result<()> {
        std::fs::rename(source, target)
    }
}

#[cfg(windows)]
mod imp {
    use super::{policy_error, File, Path};
    use std::ffi::c_void;
    use std::io;
    use std::mem::{size_of, zeroed};
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle};
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, LocalFree, GENERIC_READ, GENERIC_WRITE, HANDLE,
        INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        GetSecurityInfo, SDDL_REVISION_1, SE_FILE_OBJECT,
    };
    use windows_sys::Win32::Security::{
        AclSizeInformation, EqualSid, GetAce, GetAclInformation, GetSecurityDescriptorControl,
        GetTokenInformation, IsValidSid, IsWellKnownSid, TokenUser, WinBuiltinAdministratorsSid,
        WinLocalSystemSid, ACCESS_ALLOWED_ACE, ACL, ACL_SIZE_INFORMATION,
        DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
        SECURITY_ATTRIBUTES, SE_DACL_PROTECTED, TOKEN_QUERY, TOKEN_USER,
    };
    #[cfg(test)]
    use windows_sys::Win32::Security::{GetSecurityDescriptorDacl, GetSecurityDescriptorOwner};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FileAttributeTagInfo, GetFileInformationByHandleEx, GetFileType, MoveFileExW,
        CREATE_NEW, FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TAG_INFO,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_GENERIC_READ, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_TYPE_DISK, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::SystemServices::ACCESS_ALLOWED_ACE_TYPE;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    struct OwnedHandle(HANDLE);

    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            // SAFETY: OwnedHandle is constructed only from a successful Win32
            // call and uniquely owns the handle until this drop.
            unsafe { CloseHandle(self.0) };
        }
    }

    struct LocalAllocation(*mut c_void);

    impl Drop for LocalAllocation {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: the pointer came from a Win32 routine documented to
                // allocate with LocalAlloc and is released exactly once here.
                unsafe { LocalFree(self.0) };
            }
        }
    }

    struct CurrentUser {
        _storage: Vec<usize>,
        sid: PSID,
    }

    pub(super) fn create_new_private(path: &Path) -> io::Result<File> {
        let user = current_user()?;
        let sid = sid_string(user.sid)?;
        let sddl = format!("O:{sid}D:P(A;;FA;;;{sid})(A;;FA;;;SY)(A;;FA;;;BA)");
        let sddl = wide(&sddl)?;
        let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();
        // SAFETY: sddl is NUL-terminated and remains alive for the call;
        // descriptor is an out-pointer released with LocalFree below.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                null_mut(),
            )
        } == 0
        {
            return Err(last_error());
        }
        let descriptor_guard = LocalAllocation(descriptor);
        let attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor_guard.0,
            bInheritHandle: 0,
        };
        let file = create_file(
            path,
            GENERIC_READ | GENERIC_WRITE,
            0,
            CREATE_NEW,
            &attributes,
        )?;
        validate(&file, &user)?;
        Ok(file)
    }

    pub(super) fn open_private(path: &Path) -> io::Result<File> {
        let user = current_user()?;
        let file = create_file(
            path,
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_DELETE,
            OPEN_EXISTING,
            null(),
        )?;
        validate(&file, &user)?;
        Ok(file)
    }

    fn create_file(
        path: &Path,
        access: u32,
        share: u32,
        disposition: u32,
        attributes: *const SECURITY_ATTRIBUTES,
    ) -> io::Result<File> {
        let path = wide(path.as_os_str())?;
        // SAFETY: path is NUL-terminated; attributes is either null or points
        // to a live SECURITY_ATTRIBUTES and descriptor for the duration of
        // the call. A successful handle is transferred exactly once to File.
        let handle = unsafe {
            CreateFileW(
                path.as_ptr(),
                access,
                share,
                attributes,
                disposition,
                FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
                null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(last_error());
        }
        // SAFETY: CreateFileW returned a fresh owned handle. File assumes the
        // sole close responsibility, including on validation failure.
        Ok(unsafe { File::from_raw_handle(handle) })
    }

    fn validate(file: &File, user: &CurrentUser) -> io::Result<()> {
        let handle = file.as_raw_handle();
        // SAFETY: handle belongs to a live File and remains valid throughout
        // this function.
        if unsafe { GetFileType(handle) } != FILE_TYPE_DISK {
            return Err(policy_error("private path is not a disk file"));
        }
        let mut tag: FILE_ATTRIBUTE_TAG_INFO = unsafe { zeroed() };
        // SAFETY: tag is a correctly sized writable buffer and handle is live.
        if unsafe {
            GetFileInformationByHandleEx(
                handle,
                FileAttributeTagInfo,
                (&mut tag as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
                size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
            )
        } == 0
        {
            return Err(last_error());
        }
        if tag.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(policy_error("private path is a reparse point"));
        }

        let mut owner: PSID = null_mut();
        let mut dacl: *mut ACL = null_mut();
        let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();
        // SAFETY: every output pointer is valid for the call and descriptor is
        // released with LocalFree after all borrowed owner/DACL pointers cease
        // to be used.
        let status = unsafe {
            GetSecurityInfo(
                handle,
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                &mut owner,
                null_mut(),
                &mut dacl,
                null_mut(),
                &mut descriptor,
            )
        };
        if status != 0 {
            return Err(io::Error::from_raw_os_error(status as i32));
        }
        let _descriptor_guard = LocalAllocation(descriptor);
        validate_descriptor(descriptor, owner, dacl, user.sid)
    }

    fn validate_descriptor(
        descriptor: PSECURITY_DESCRIPTOR,
        owner: PSID,
        dacl: *mut ACL,
        user_sid: PSID,
    ) -> io::Result<()> {
        if owner.is_null() || unsafe { EqualSid(owner, user_sid) } == 0 {
            return Err(policy_error(
                "private file is not owned by the current user",
            ));
        }
        if dacl.is_null() {
            return Err(policy_error("private file has a null DACL"));
        }
        let mut control = 0u16;
        let mut revision = 0u32;
        // SAFETY: descriptor is live for the guard's scope and both output
        // pointers reference initialized stack values.
        if unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) } == 0 {
            return Err(last_error());
        }
        if control & SE_DACL_PROTECTED == 0 {
            return Err(policy_error("private file DACL permits inheritance"));
        }
        validate_dacl(dacl, user_sid)
    }

    fn validate_dacl(dacl: *mut ACL, user_sid: PSID) -> io::Result<()> {
        let mut info: ACL_SIZE_INFORMATION = unsafe { zeroed() };
        // SAFETY: dacl comes from the live descriptor in validate; info is a
        // correctly sized writable output buffer.
        if unsafe {
            GetAclInformation(
                dacl,
                (&mut info as *mut ACL_SIZE_INFORMATION).cast(),
                size_of::<ACL_SIZE_INFORMATION>() as u32,
                AclSizeInformation,
            )
        } == 0
        {
            return Err(last_error());
        }

        let mut user_can_read = false;
        for index in 0..info.AceCount {
            let mut raw_ace: *mut c_void = null_mut();
            // SAFETY: index is bounded by AceCount reported for this live ACL;
            // raw_ace is a valid out-pointer.
            if unsafe { GetAce(dacl, index, &mut raw_ace) } == 0 {
                return Err(last_error());
            }
            let ace = raw_ace.cast::<ACCESS_ALLOWED_ACE>();
            // SAFETY: every ACE begins with ACE_HEADER. We accept only the
            // fixed ACCESS_ALLOWED_ACE layout and reject all other/future ACE
            // forms conservatively rather than guessing where their SID lies.
            if unsafe { (*ace).Header.AceType } as u32 != ACCESS_ALLOWED_ACE_TYPE {
                return Err(policy_error(
                    "private file DACL contains an unsupported ACE",
                ));
            }
            if (unsafe { (*ace).Header.AceSize } as usize) < size_of::<ACCESS_ALLOWED_ACE>() {
                return Err(policy_error("private file DACL contains a truncated ACE"));
            }
            // SAFETY: ACCESS_ALLOWED_ACE's SID begins at SidStart and remains
            // inside the ACL-owned ACE for the lifetime of descriptor.
            let sid = unsafe { (&mut (*ace).SidStart as *mut u32).cast::<c_void>() };
            // GetSecurityInfo returns a kernel-validated descriptor, but keep
            // this explicit guard before any SID comparison so this parser
            // also fails closed if that contract ever changes.
            if unsafe { IsValidSid(sid) } == 0 {
                return Err(policy_error("private file DACL contains an invalid SID"));
            }
            let is_user = unsafe { EqualSid(sid, user_sid) } != 0;
            let privileged = unsafe {
                IsWellKnownSid(sid, WinLocalSystemSid) != 0
                    || IsWellKnownSid(sid, WinBuiltinAdministratorsSid) != 0
            };
            if !is_user && !privileged {
                return Err(policy_error(
                    "private file DACL grants access to an unprivileged principal",
                ));
            }
            if is_user && unsafe { (*ace).Mask } & FILE_GENERIC_READ == FILE_GENERIC_READ {
                user_can_read = true;
            }
        }
        if !user_can_read {
            return Err(policy_error(
                "private file DACL does not grant current-user read access",
            ));
        }
        Ok(())
    }

    fn current_user() -> io::Result<CurrentUser> {
        let mut token: HANDLE = null_mut();
        // SAFETY: GetCurrentProcess returns a pseudo-handle and token is a
        // valid out-pointer. A successful real token handle is RAII-owned.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err(last_error());
        }
        let token = OwnedHandle(token);
        let mut needed = 0u32;
        // SAFETY: the documented sizing call uses a null buffer and zero size.
        unsafe { GetTokenInformation(token.0, TokenUser, null_mut(), 0, &mut needed) };
        if needed == 0 {
            return Err(last_error());
        }
        let words = (needed as usize).div_ceil(size_of::<usize>());
        let mut storage = vec![0usize; words];
        // SAFETY: storage is aligned and at least `needed` bytes long; the
        // token handle remains live for the call.
        if unsafe {
            GetTokenInformation(
                token.0,
                TokenUser,
                storage.as_mut_ptr().cast(),
                needed,
                &mut needed,
            )
        } == 0
        {
            return Err(last_error());
        }
        // SAFETY: a successful TokenUser query initializes TOKEN_USER at the
        // start of storage; its SID points into the same stable allocation.
        let sid = unsafe { (*(storage.as_ptr().cast::<TOKEN_USER>())).User.Sid };
        Ok(CurrentUser {
            _storage: storage,
            sid,
        })
    }

    fn sid_string(sid: PSID) -> io::Result<String> {
        let mut value = null_mut();
        // SAFETY: sid comes from a live TOKEN_USER and value is an out-pointer
        // for a LocalAlloc-owned NUL-terminated string.
        if unsafe { ConvertSidToStringSidW(sid, &mut value) } == 0 {
            return Err(last_error());
        }
        let guard = LocalAllocation(value.cast());
        let mut len = 0usize;
        // SAFETY: ConvertSidToStringSidW guarantees a NUL-terminated string.
        unsafe {
            while *value.add(len) != 0 {
                len += 1;
            }
        }
        // SAFETY: the preceding scan found the terminator within the
        // allocation; the slice excludes it and remains live under guard.
        let slice = unsafe { std::slice::from_raw_parts(value, len) };
        let result = String::from_utf16(slice)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "current SID is not UTF-16"));
        drop(guard);
        result
    }

    #[cfg(test)]
    pub(super) fn test_current_user_sid_string() -> io::Result<String> {
        let user = current_user()?;
        sid_string(user.sid)
    }

    #[cfg(test)]
    pub(super) fn test_validate_sddl(sddl: &str) -> io::Result<()> {
        let user = current_user()?;
        let sddl = wide(sddl)?;
        let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();
        // SAFETY: the test string is NUL-terminated and descriptor is a valid
        // out-pointer released by LocalAllocation.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                null_mut(),
            )
        } == 0
        {
            return Err(last_error());
        }
        let _guard = LocalAllocation(descriptor);
        let mut owner: PSID = null_mut();
        let mut owner_defaulted = 0;
        let mut dacl: *mut ACL = null_mut();
        let mut dacl_present = 0;
        let mut dacl_defaulted = 0;
        // SAFETY: descriptor is a live parsed descriptor and each output
        // pointer references initialized stack storage.
        if unsafe { GetSecurityDescriptorOwner(descriptor, &mut owner, &mut owner_defaulted) } == 0
            || unsafe {
                GetSecurityDescriptorDacl(
                    descriptor,
                    &mut dacl_present,
                    &mut dacl,
                    &mut dacl_defaulted,
                )
            } == 0
        {
            return Err(last_error());
        }
        if dacl_present == 0 {
            dacl = null_mut();
        }
        validate_descriptor(descriptor, owner, dacl, user.sid)
    }

    fn wide(value: impl AsRef<std::ffi::OsStr>) -> io::Result<Vec<u16>> {
        let mut result: Vec<u16> = value.as_ref().encode_wide().collect();
        if result.contains(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Windows path/security string contains NUL",
            ));
        }
        result.push(0);
        Ok(result)
    }

    fn last_error() -> io::Error {
        // SAFETY: GetLastError has no preconditions and reads thread-local state.
        io::Error::from_raw_os_error(unsafe { GetLastError() } as i32)
    }

    pub(super) fn replace(source: &Path, target: &Path) -> io::Result<()> {
        let source_wide = wide(source.as_os_str())?;
        let target_wide = wide(target.as_os_str())?;
        // MoveFileExW on two paths in the same parent is an atomic rename and
        // keeps the source file's protected DACL. Unlike ReplaceFileW, it does
        // not preserve a potentially unsafe target DACL. The caller-controlled
        // parent boundary is documented at module level.
        // SAFETY: both paths are live, NUL-terminated UTF-16 strings.
        if unsafe {
            MoveFileExW(
                source_wide.as_ptr(),
                target_wide.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        } == 0
        {
            return Err(last_error());
        }
        // Validate the final named object as a defense against filesystem or
        // platform behavior that would fail to retain the source descriptor.
        open_private(target)?;
        Ok(())
    }
}

#[cfg(not(any(unix, windows)))]
mod imp {
    use super::{File, Path};
    use std::io;

    pub(super) fn create_new_private(_path: &Path) -> io::Result<File> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "private files are unsupported on this platform",
        ))
    }

    pub(super) fn open_private(_path: &Path) -> io::Result<File> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "private files are unsupported on this platform",
        ))
    }

    pub(super) fn replace(_source: &Path, _target: &Path) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "private files are unsupported on this platform",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{create_new_private, open_private, replace_private, TEMP_COUNTER};
    use std::io::{Read, Write};
    use tempfile::TempDir;

    #[test]
    fn create_open_and_replace_round_trip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("credential");
        let mut created = create_new_private(&path).unwrap();
        created.write_all(b"old").unwrap();
        created.sync_all().unwrap();
        drop(created);

        assert_eq!(
            create_new_private(&path).unwrap_err().kind(),
            std::io::ErrorKind::AlreadyExists
        );
        let mut opened = open_private(&path).unwrap();
        let mut value = String::new();
        opened.read_to_string(&mut value).unwrap();
        assert_eq!(value, "old");

        replace_private(&path, b"new value").unwrap();
        let mut opened = open_private(&path).unwrap();
        value.clear();
        opened.read_to_string(&mut value).unwrap();
        assert_eq!(value, "new value");
    }

    #[test]
    fn replacement_bounds_precreated_temp_name_collisions() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("collision-target");
        let first = TEMP_COUNTER.load(std::sync::atomic::Ordering::Relaxed);
        for attempt in first..first + 256 {
            let temp = dir.path().join(format!(
                ".collision-target.tmp.{}.{attempt}",
                std::process::id()
            ));
            std::fs::write(temp, b"occupied").unwrap();
        }

        let error = replace_private(&target, b"secret").unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert!(!target.exists());
    }

    #[cfg(unix)]
    #[test]
    fn unix_policy_rejects_permissive_nonfiles_and_symlinks() {
        use std::os::unix::ffi::OsStrExt;
        use std::os::unix::fs::{symlink, PermissionsExt};

        let dir = TempDir::new().unwrap();
        let permissive = dir.path().join("permissive");
        std::fs::write(&permissive, b"secret").unwrap();
        std::fs::set_permissions(&permissive, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(
            open_private(&permissive).unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );

        assert!(open_private(dir.path()).is_err());
        let link = dir.path().join("link");
        symlink(&permissive, &link).unwrap();
        assert!(open_private(&link).is_err());

        let fifo = dir.path().join("fifo");
        let fifo_c = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
        // SAFETY: fifo_c is a valid NUL-terminated path and mkfifo does not
        // retain the pointer after returning.
        assert_eq!(unsafe { libc::mkfifo(fifo_c.as_ptr(), 0o600) }, 0);
        assert_eq!(
            open_private(&fifo).unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
    }

    #[cfg(unix)]
    #[test]
    fn unix_created_and_replaced_files_remain_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("private");
        replace_private(&path, b"first").unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        replace_private(&path, b"second").unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );

        let replace_directory = dir.path().join("directory-target");
        std::fs::create_dir(&replace_directory).unwrap();
        assert!(replace_private(&replace_directory, b"cannot replace directory").is_err());
        assert!(replace_private(dir.path().join("missing").join("value"), b"x").is_err());
    }

    #[cfg(windows)]
    #[test]
    fn windows_rejects_an_ordinary_inherited_dacl() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("inherited");
        std::fs::write(&path, b"not private").unwrap();
        assert_eq!(
            open_private(&path).unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
        replace_private(&path, b"repaired private contents").unwrap();
        let mut repaired = open_private(&path).unwrap();
        let mut repaired_contents = String::new();
        repaired.read_to_string(&mut repaired_contents).unwrap();
        assert_eq!(repaired_contents, "repaired private contents");

        let missing = dir.path().join("missing");
        assert_eq!(
            open_private(&missing).unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );

        use std::os::windows::ffi::OsStringExt;
        let nul_path = std::path::PathBuf::from(std::ffi::OsString::from_wide(&[
            b'n' as u16,
            0,
            b'x' as u16,
        ]));
        assert_eq!(
            create_new_private(nul_path).unwrap_err().kind(),
            std::io::ErrorKind::InvalidInput
        );

        let private = dir.path().join("private");
        replace_private(&private, b"private contents").unwrap();
        let link = dir.path().join("reparse-link");
        std::os::windows::fs::symlink_file(&private, &link).unwrap();
        assert_eq!(
            open_private(&link).unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_descriptor_policy_rejects_every_insecure_shape() {
        let sid = super::imp::test_current_user_sid_string().unwrap();
        let validate = |sddl: String| super::imp::test_validate_sddl(&sddl);

        validate(format!("O:{sid}D:P(A;;FA;;;{sid})(A;;FA;;;SY)(A;;FA;;;BA)")).unwrap();

        let cases = [
            (
                format!("O:BAD:P(A;;FA;;;{sid})"),
                "not owned by the current user",
            ),
            (format!("O:{sid}D:NO_ACCESS_CONTROL"), "null DACL"),
            (format!("O:{sid}D:(A;;FA;;;{sid})"), "permits inheritance"),
            (
                format!("O:{sid}D:P(A;;FA;;;WD)(A;;FA;;;{sid})"),
                "unprivileged principal",
            ),
            (
                format!("O:{sid}D:P(D;;FW;;;WD)(A;;FA;;;{sid})"),
                "unsupported ACE",
            ),
            (
                format!("O:{sid}D:P(A;;FW;;;{sid})"),
                "does not grant current-user read access",
            ),
        ];
        for (sddl, expected) in cases {
            let error = validate(sddl).unwrap_err();
            assert!(
                error.to_string().contains(expected),
                "unexpected policy error: {error}"
            );
        }
    }
}
