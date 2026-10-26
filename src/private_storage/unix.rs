use super::*;
use std::{
    fs::{self, OpenOptions},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
};

pub struct UnixStorage;
fn private_directory(path: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(path)?;
    ensure!(
        meta.is_dir() && meta.uid() == unsafe { libc::geteuid() } && meta.mode() & 0o077 == 0,
        "private settings directory must be private and owned by the current user"
    );
    Ok(())
}
impl PrivateStorage for UnixStorage {
    fn read(&self, path: &Path, maximum: u64) -> Result<Option<Vec<u8>>> {
        let file = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        private_directory(
            path.parent()
                .context("private settings directory unavailable")?,
        )?;
        let meta = file.metadata()?;
        ensure!(
            meta.is_file() && meta.uid() == unsafe { libc::geteuid() } && meta.mode() & 0o077 == 0,
            "private settings must be a private regular file owned by the current user"
        );
        Ok(Some(bounded_read(file, maximum)?))
    }
    fn write(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        let directory = path
            .parent()
            .context("private settings directory unavailable")?;
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(directory)?;
        private_directory(directory)?;
        self.read(path, 64 * 1024)?;
        let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
        temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))?;
        temporary.write_all(bytes)?;
        temporary.as_file().sync_all()?;
        temporary.persist(path).map_err(|e| e.error)?;
        // Rename commits the update, so a later fsync error must not roll back live state.
        if let Err(error) = File::open(directory).and_then(|f| f.sync_all()) {
            tracing::warn!(%error, "private settings committed but directory sync failed");
        }
        Ok(())
    }
}
