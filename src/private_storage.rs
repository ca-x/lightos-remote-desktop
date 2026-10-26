//! Private settings files. Platform checks happen on opened files before reading.
use anyhow::{Context, Result, ensure};
use std::{
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

pub trait PrivateStorage: Send + Sync {
    fn read(&self, path: &Path, maximum: u64) -> Result<Option<Vec<u8>>>;
    fn write(&self, path: &Path, bytes: &[u8]) -> Result<()>;
}

pub fn current() -> &'static dyn PrivateStorage {
    #[cfg(unix)]
    {
        &unix::UnixStorage
    }
    #[cfg(windows)]
    {
        &windows::WindowsStorage
    }
}

pub fn state_directory() -> Option<PathBuf> {
    dirs::state_dir()
        .or_else(dirs::data_local_dir)
        .map(|p| p.join("lightos-remote-desktop"))
}

fn bounded_read(file: File, maximum: u64) -> Result<Vec<u8>> {
    ensure!(
        file.metadata()?.len() <= maximum,
        "private settings file too large"
    );
    let mut bytes = Vec::new();
    file.take(maximum + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= maximum,
        "private settings file too large"
    );
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_settings_roundtrip_and_size_limit() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state/private.json");
        assert!(current().read(&path, 100).unwrap().is_none());
        current().write(&path, b"private value").unwrap();
        assert_eq!(
            current().read(&path, 100).unwrap().unwrap(),
            b"private value"
        );
        assert!(current().read(&path, 3).is_err());
        current().write(&path, b"replacement").unwrap();
        assert_eq!(current().read(&path, 100).unwrap().unwrap(), b"replacement");
    }
    #[cfg(unix)]
    #[test]
    fn refuses_links_and_non_private_files() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("private/settings");
        current().write(&path, b"secret").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(current().read(&path, 100).is_err());
        assert!(current().write(&path, b"new secret").is_err());
        let link = path.with_file_name("link");
        symlink(&path, &link).unwrap();
        assert!(current().read(&link, 100).is_err());
        assert!(current().write(&link, b"new secret").is_err());
    }
}
