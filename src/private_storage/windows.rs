use super::*;
use ::windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE, HLOCAL, LocalFree},
        Security::{Authorization::*, *},
        Storage::FileSystem::{
            FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        },
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    },
    core::{PCWSTR, PWSTR},
};
use std::{
    fs::{self, OpenOptions},
    os::windows::{
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    },
};

pub struct WindowsStorage;
struct Descriptor(PSECURITY_DESCRIPTOR);
impl Drop for Descriptor {
    fn drop(&mut self) {
        unsafe {
            let _ = LocalFree(Some(HLOCAL(self.0.0)));
        }
    }
}

fn sid_string(sid: PSID) -> Result<String> {
    let mut text = PWSTR::null();
    unsafe {
        ConvertSidToStringSidW(sid, &mut text)?;
        let result = text.to_string();
        let _ = LocalFree(Some(HLOCAL(text.0.cast())));
        Ok(result?)
    }
}
fn current_sid() -> Result<String> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)?;
        let result = (|| -> Result<String> {
            let mut length = 0;
            let _ = GetTokenInformation(token, TokenUser, None, 0, &mut length);
            ensure!(length > 0, "unable to query current user SID");
            let mut storage =
                vec![0usize; (length as usize).div_ceil(std::mem::size_of::<usize>())];
            GetTokenInformation(
                token,
                TokenUser,
                Some(storage.as_mut_ptr().cast()),
                length,
                &mut length,
            )?;
            sid_string((*(storage.as_ptr().cast::<TOKEN_USER>())).User.Sid)
        })();
        let _ = CloseHandle(token);
        result
    }
}
fn open(path: &Path, directory: bool) -> std::io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(
            FILE_FLAG_OPEN_REPARSE_POINT.0
                | if directory {
                    FILE_FLAG_BACKUP_SEMANTICS.0
                } else {
                    0
                },
        )
        .open(path)
}
fn validate(file: &File, directory: bool) -> Result<()> {
    let meta = file.metadata()?;
    ensure!(
        meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 == 0
            && if directory {
                meta.is_dir()
            } else {
                meta.is_file()
            },
        "private settings must not use reparse points"
    );
    unsafe {
        let mut owner = PSID::default();
        let mut dacl = std::ptr::null_mut();
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        GetSecurityInfo(
            HANDLE(file.as_raw_handle()),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            Some(&mut owner),
            None,
            Some(&mut dacl),
            None,
            Some(&mut descriptor),
        )
        .ok()?;
        let _descriptor = Descriptor(descriptor);
        ensure!(!dacl.is_null(), "private settings require a non-null DACL");
        let user = current_sid()?;
        let allowed = |sid: &str| sid == user || sid == "S-1-5-18" || sid == "S-1-5-32-544";
        ensure!(
            allowed(&sid_string(owner)?),
            "private settings owner mismatch"
        );
        let mut control = 0;
        let mut revision = 0;
        GetSecurityDescriptorControl(descriptor, &mut control, &mut revision)?;
        ensure!(
            control & SE_DACL_PROTECTED.0 != 0,
            "private settings DACL must disable inherited access"
        );
        let mut user_read = false;
        for index in 0..u32::from((*dacl).AceCount) {
            let mut ace = std::ptr::null_mut();
            GetAce(dacl, index, &mut ace)?;
            let header = &*ace.cast::<ACE_HEADER>();
            ensure!(
                header.AceType == 0,
                "private settings contain an unsupported access entry"
            );
            let entry = &*ace.cast::<ACCESS_ALLOWED_ACE>();
            let sid = sid_string(PSID(std::ptr::addr_of!(entry.SidStart).cast_mut().cast()))?;
            ensure!(
                allowed(&sid),
                "private settings grant access to another account"
            );
            if sid == user && entry.Mask & (0x8000_0000 | 0x1000_0000 | 1) != 0 {
                user_read = true;
            }
        }
        ensure!(
            user_read,
            "current user has no explicit private settings access"
        );
    }
    Ok(())
}
fn protect(path: &Path, directory: bool) -> Result<()> {
    let user = current_sid()?;
    let inheritance = if directory { "OICI" } else { "" };
    let sddl: Vec<u16> = format!(
        "D:P(A;{inheritance};FA;;;SY)(A;{inheritance};FA;;;BA)(A;{inheritance};FA;;;{user})"
    )
    .encode_utf16()
    .chain(Some(0))
    .collect();
    unsafe {
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl.as_ptr()),
            1,
            &mut descriptor,
            None,
        )?;
        let _descriptor = Descriptor(descriptor);
        let mut present = ::windows::core::BOOL::default();
        let mut defaulted = ::windows::core::BOOL::default();
        let mut dacl = std::ptr::null_mut();
        GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted)?;
        ensure!(present.as_bool() && !dacl.is_null(), "invalid private DACL");
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        SetNamedSecurityInfoW(
            PCWSTR(wide.as_ptr()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(dacl),
            None,
        )
        .ok()?;
    }
    validate(&open(path, directory)?, directory)
}
use std::os::windows::ffi::OsStrExt;

impl PrivateStorage for WindowsStorage {
    fn read(&self, path: &Path, maximum: u64) -> Result<Option<Vec<u8>>> {
        let file = match open(path, false) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        validate(
            &open(
                path.parent()
                    .context("private settings directory unavailable")?,
                true,
            )?,
            true,
        )?;
        validate(&file, false)?;
        Ok(Some(bounded_read(file, maximum)?))
    }
    fn write(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        let directory = path
            .parent()
            .context("private settings directory unavailable")?;
        if !directory.exists() {
            fs::create_dir_all(directory)?;
            protect(directory, true)?;
        }
        validate(&open(directory, true)?, true)?;
        self.read(path, 64 * 1024)?;
        let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
        protect(temporary.path(), false)?;
        temporary.write_all(bytes)?;
        temporary.as_file().sync_all()?;
        temporary.persist(path).map_err(|e| e.error)?;
        Ok(())
    }
}
