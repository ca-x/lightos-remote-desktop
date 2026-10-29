#![cfg(target_os = "linux")]

use std::{
    fs,
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::{fs::PermissionsExt, net::UnixListener},
    },
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[test]
fn cancelling_publisher_terminates_its_direct_activation_helper() {
    assert_eq!(unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1) }, 0);
    let directory = tempfile::tempdir().unwrap();
    let runtime = directory.path().join("runtime");
    let bin = directory.path().join("bin");
    fs::create_dir(&runtime).unwrap();
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
    fs::create_dir(&bin).unwrap();
    let _sockets = ["bus", "wayland-7", "niri.control.sock"]
        .map(|name| UnixListener::bind(runtime.join(name)).unwrap());
    let pid_file = directory.path().join("activator.pid");
    let activator = bin.join("dbus-update-activation-environment");
    fs::write(
        &activator,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$$\" > '{}'\nexec /bin/sleep 30\n",
            pid_file.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&activator, fs::Permissions::from_mode(0o700)).unwrap();
    let mut publisher = Command::new(env!("CARGO_BIN_EXE_lightos-remoted"))
        .env_clear()
        .env("PATH", &bin)
        .env("XDG_RUNTIME_DIR", &runtime)
        .env(
            "DBUS_SESSION_BUS_ADDRESS",
            format!("unix:path={}", runtime.join("bus").display()),
        )
        .env("WAYLAND_DISPLAY", "wayland-7")
        .env("NIRI_SOCKET", runtime.join("niri.control.sock"))
        .args(["niri-session-environment", "--runtime"])
        .arg(&runtime)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let helper_pid = loop {
        if let Ok(contents) = fs::read_to_string(&pid_file)
            && let Ok(pid) = contents.trim().parse::<i32>()
        {
            break Some(pid);
        }
        if Instant::now() >= deadline || publisher.try_wait().unwrap().is_some() {
            break None;
        }
        thread::sleep(Duration::from_millis(10));
    };
    let helper = helper_pid.map(|pid| {
        let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) as i32 };
        assert!(raw >= 0);
        unsafe { OwnedFd::from_raw_fd(raw) }
    });
    publisher.kill().unwrap();
    publisher.wait().unwrap();
    let helper = helper.expect("publisher never started the isolated activation helper");
    let mut poll = libc::pollfd {
        fd: helper.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    let stopped = unsafe { libc::poll(&mut poll, 1, 1500) } == 1
        && poll.revents & libc::POLLIN != 0;
    if !stopped {
        unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                helper.as_raw_fd(),
                libc::SIGKILL,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            );
        }
    }
    unsafe { libc::waitpid(helper_pid.unwrap(), std::ptr::null_mut(), 0) };
    assert!(!runtime.join("niri-environment.json").exists());
    assert!(stopped, "publisher cancellation left its activation helper alive");
}
