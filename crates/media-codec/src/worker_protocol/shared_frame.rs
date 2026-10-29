//! One bounded frame per worker. The parent writes only between replies; the
//! worker reads only while handling its single outstanding Frame request.
use anyhow::{Context, Result, ensure};
use memmap2::{Mmap, MmapMut};
use std::{
    fs::File,
    os::fd::{AsRawFd, FromRawFd, RawFd},
};

pub const ENV: &str = "LIGHTOS_CODEC_FRAME_FD";
pub const NEGOTIATE_ENV: &str = "LIGHTOS_CODEC_NEGOTIATE_SHARED_FRAME";
pub const CHILD_FD: RawFd = 3;
const SIZE_SEALS: i32 = libc::F_SEAL_SHRINK | libc::F_SEAL_GROW;
const MAX_PIXELS: usize = 3840 * 2160 * 4;

pub struct SharedFrame {
    file: File,
    pixels: MmapMut,
}
impl SharedFrame {
    pub fn new(length: usize) -> Result<Self> {
        ensure!(
            (1..=MAX_PIXELS).contains(&length),
            "invalid shared frame size"
        );
        // SAFETY: fixed name and flags, ownership transferred exactly once.
        let fd = unsafe {
            libc::memfd_create(
                c"lightos-codec-frame".as_ptr(),
                libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING,
            )
        };
        ensure!(
            fd >= 0,
            "create codec frame: {}",
            std::io::Error::last_os_error()
        );
        let file = unsafe { File::from_raw_fd(fd) };
        file.set_len(length as u64)?;
        // Seals prevent either peer from truncating a live mapping.
        ensure!(
            unsafe { libc::fcntl(fd, libc::F_ADD_SEALS, SIZE_SEALS | libc::F_SEAL_SEAL) } >= 0,
            "seal codec frame: {}",
            std::io::Error::last_os_error()
        );
        let pixels = unsafe { MmapMut::map_mut(&file) }.context("map codec frame")?;
        Ok(Self { file, pixels })
    }
    pub fn fd(&self) -> RawFd {
        self.file.as_raw_fd()
    }
    pub fn write(&mut self, pixels: &[u8]) -> Result<()> {
        ensure!(
            pixels.len() == self.pixels.len(),
            "invalid shared pixel buffer"
        );
        self.pixels.copy_from_slice(pixels);
        Ok(())
    }
}

pub fn inherited() -> Result<Option<Mmap>> {
    let Some(value) = std::env::var_os(ENV) else {
        return Ok(None);
    };
    ensure!(
        value == CHILD_FD.to_string().as_str(),
        "invalid inherited codec descriptor"
    );
    let seals = unsafe { libc::fcntl(CHILD_FD, libc::F_GET_SEALS) };
    ensure!(
        seals >= 0 && seals & SIZE_SEALS == SIZE_SEALS,
        "unsealed inherited codec frame"
    );
    // The launcher transfers this descriptor only to the codec worker. No
    // other code takes ownership of it, and the mapping keeps its own reference.
    let file = unsafe { File::from_raw_fd(CHILD_FD) };
    let length = file.metadata()?.len();
    ensure!(
        (1..=MAX_PIXELS as u64).contains(&length),
        "invalid inherited codec frame size"
    );
    Ok(Some(
        unsafe { Mmap::map(&file) }.context("map inherited codec frame")?,
    ))
}
