//! Kernel proof for a compositor that inherited its listening Wayland socket.
//! No launcher, desktop, distribution or image names participate in admission.
use anyhow::{Context, Result, ensure};
use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::Read,
    os::unix::{ffi::OsStrExt, fs::FileTypeExt, fs::MetadataExt},
    path::{Path, PathBuf},
};

pub(crate) fn verify_listener(socket: &Path, pid: u32, uid: u32) -> Result<()> {
    ensure!(pid > 1, "invalid Wayland server process");
    let process = PathBuf::from(format!("/proc/{pid}"));
    ensure!(
        fs::metadata(&process)?.uid() == uid,
        "Wayland server belongs to another user"
    );
    let node = fs::symlink_metadata(socket)?;
    let server_node = fs::symlink_metadata(process.join("root").join(socket.strip_prefix("/")?))?;
    ensure!(
        node.file_type().is_socket()
            && node.uid() == uid
            && (node.dev(), node.ino()) == (server_node.dev(), server_node.ino()),
        "Wayland server sees a different socket at the selected path"
    );
    let started = process_start(&process)?;
    let inode = listener_inode(&process, socket)?;
    let mut held = false;
    for (index, entry) in fs::read_dir(process.join("fd"))?.enumerate() {
        ensure!(index < 16384, "Wayland server descriptor limit exceeded");
        let entry = entry?;
        let metadata = match fs::metadata(entry.path()) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error).context("read Wayland server descriptor"),
        };
        if metadata.file_type().is_socket() && metadata.ino() == inode {
            held = true;
            break;
        }
    }
    ensure!(
        held,
        "process {pid} does not hold the selected Wayland listening socket"
    );
    let current_node = fs::symlink_metadata(socket)?;
    ensure!(
        fs::metadata(&process)?.uid() == uid
            && process_start(&process)? == started
            && listener_inode(&process, socket)? == inode
            && (node.dev(), node.ino()) == (current_node.dev(), current_node.ino()),
        "Wayland server identity changed during discovery"
    );
    Ok(())
}

fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= limit,
        "Wayland identity evidence exceeds the read limit"
    );
    Ok(bytes)
}

fn process_start(process: &Path) -> Result<u64> {
    let bytes = read_bounded(&process.join("stat"), 4096)?;
    let end = bytes
        .iter()
        .rposition(|byte| *byte == b')')
        .context("Wayland server stat has no process name")?;
    // The remaining fields begin with state (field 3); starttime is field 22.
    Ok(std::str::from_utf8(&bytes[end + 1..])?
        .split_whitespace()
        .nth(19)
        .context("Wayland server stat has no start time")?
        .parse()?)
}

fn listener_inode(process: &Path, socket: &Path) -> Result<u64> {
    let bytes = read_bounded(&process.join("net/unix"), 2 * 1024 * 1024)?;
    let name = socket.as_os_str().as_bytes();
    let mut inodes = BTreeSet::new();
    for line in bytes.split(|byte| *byte == b'\n') {
        let Some(prefix) = line.strip_suffix(name) else {
            continue;
        };
        if !prefix.last().is_some_and(u8::is_ascii_whitespace) {
            continue;
        }
        let fields: Vec<_> = prefix
            .split(u8::is_ascii_whitespace)
            .filter(|field| !field.is_empty())
            .collect();
        if fields.len() != 7 || fields[4] != b"0001" || fields[5] != b"01" {
            continue;
        }
        let flags = u32::from_str_radix(std::str::from_utf8(fields[3])?, 16)?;
        // SOCK_STREAM, listening (SO_ACCEPTCON), not an accepted client socket
        // carrying the same address in /proc/net/unix.
        if flags & 0x10000 != 0 {
            inodes.insert(std::str::from_utf8(fields[6])?.parse::<u64>()?);
        }
    }
    ensure!(
        inodes.len() == 1,
        "selected Wayland listening socket is unavailable or ambiguous"
    );
    Ok(*inodes.first().unwrap())
}
