//! Kernel-owned lock; an interrupted or crashed controller cannot leave a stale lock.
use anyhow::Result;
use std::{
    fs::{File, OpenOptions},
    path::Path,
};

pub fn acquire(directory: &Path) -> Result<Option<File>> {
    std::fs::create_dir_all(directory)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0);
    }
    let file = match options.open(directory.join("controller.lock")) {
        Ok(file) => file,
        #[cfg(windows)]
        Err(error) if error.raw_os_error() == Some(32) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::WouldBlock {
                return Ok(None);
            }
            return Err(error.into());
        }
    }
    Ok(Some(file))
}
