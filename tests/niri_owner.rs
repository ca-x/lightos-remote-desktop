#![cfg(target_os = "linux")]

use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    os::{
        fd::AsRawFd,
        unix::{fs::{MetadataExt, PermissionsExt}, net::UnixStream, process::CommandExt},
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};

const PASSWORD: &str = "niri-owner-isolated-fixture";
const APPLICATION: &str = "lightos-niri-owner-fixture";

struct Process(Child);

impl Process {
    fn exited(&self) -> bool {
        let mut status: libc::siginfo_t = unsafe { std::mem::zeroed() };
        assert_eq!(
            unsafe {
                libc::waitid(
                    libc::P_PID,
                    self.0.id(),
                    &mut status,
                    libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                )
            },
            0
        );
        unsafe { status.si_pid() != 0 }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        unsafe { libc::kill(-(self.0.id() as i32), libc::SIGTERM) };
        let deadline = Instant::now() + Duration::from_secs(10);
        while !self.exited() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        unsafe { libc::kill(-(self.0.id() as i32), libc::SIGKILL) };
        let _ = self.0.wait();
    }
}

fn start(action: &str, home: &Path, arguments: &[&str], log: &Path) -> Process {
    let output = fs::File::create(log).unwrap();
    Process(
        Command::new(env!("CARGO_BIN_EXE_lightos-remoted"))
            .env_clear()
            .env("PATH", "/usr/bin")
            .env("HOME", home)
            .env("LIBGL_ALWAYS_SOFTWARE", "1")
            .env("GALLIUM_DRIVER", "llvmpipe")
            .env("WLR_RENDERER", "pixman")
            .env("LIGHTOS_REMOTE_DESKTOP_AUTH_PASSWORD", PASSWORD)
            .arg(action)
            .args(["--password", PASSWORD, "--compositor", "niri", "--audio", "false"])
            .args(arguments)
            .stdin(Stdio::null())
            .stdout(output.try_clone().unwrap())
            .stderr(output)
            .process_group(0)
            .spawn()
            .unwrap(),
    )
}

fn await_condition(process: &Process, log: &Path, ready: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(35);
    while !ready() {
        assert!(
            !process.exited() && Instant::now() < deadline,
            "candidate did not become ready: {}",
            fs::read_to_string(log).unwrap_or_default().chars().take(4096).collect::<String>()
        );
        thread::sleep(Duration::from_millis(50));
    }
}

fn identity(pid: u32) -> (u32, String) {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    let mut fields = stat.rsplit_once(") ").unwrap().1.split_whitespace();
    (pid, fields.nth(19).unwrap().to_owned())
}

fn socket_pid(path: &Path) -> u32 {
    let stream = UnixStream::connect(path).unwrap();
    let mut peer: libc::ucred = unsafe { std::mem::zeroed() };
    let mut length = std::mem::size_of_val(&peer) as libc::socklen_t;
    assert_eq!(
        unsafe {
            libc::getsockopt(
                stream.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&mut peer as *mut libc::ucred).cast(),
                &mut length,
            )
        },
        0
    );
    assert_eq!(peer.uid, unsafe { libc::geteuid() });
    assert!(peer.pid > 1);
    peer.pid as u32
}

fn application(socket: &Path) -> Option<(u64, u32)> {
    let mut stream = UnixStream::connect(socket).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    stream.set_write_timeout(Some(Duration::from_secs(2))).unwrap();
    stream.write_all(b"\"Windows\"\n").unwrap();
    let mut response = String::new();
    BufReader::new(stream).take(1024 * 1024).read_line(&mut response).unwrap();
    let response: Value = serde_json::from_str(&response).unwrap();
    response["Ok"]["Windows"].as_array().unwrap().iter().find_map(|window| {
        (window["app_id"] == APPLICATION && window["is_focused"] == true)
            .then(|| Some((window["id"].as_u64()?, u32::try_from(window["pid"].as_u64()?).ok()?)))?
    })
}

fn healthy(port: u16) -> bool {
    let Ok(mut socket) = TcpStream::connect_timeout(
        &format!("127.0.0.1:{port}").parse().unwrap(),
        Duration::from_millis(200),
    ) else {
        return false;
    };
    socket.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
    socket.set_write_timeout(Some(Duration::from_secs(1))).unwrap();
    if socket.write_all(b"GET /healthz HTTP/1.0\r\nHost: localhost\r\n\r\n").is_err() {
        return false;
    }
    let mut response = String::new();
    socket.read_to_string(&mut response).is_ok()
        && response.starts_with("HTTP/1.0 200")
        && response.contains("\"native\":true")
}

async fn colored_frame(port: u16, red: bool, output: &Path) -> Value {
    let mut request = format!("ws://127.0.0.1:{port}/ws").into_client_request().unwrap();
    request.headers_mut().insert(
        "Authorization",
        format!("Basic {}", STANDARD.encode(format!("lightos:{PASSWORD}"))).parse().unwrap(),
    );
    request.headers_mut().insert("Origin", format!("http://127.0.0.1:{port}").parse().unwrap());
    let (mut socket, _) = tokio::time::timeout(Duration::from_secs(5), connect_async(request))
        .await.unwrap().unwrap();
    let mut frame_id = None;
    let mut ready = false;
    let observed = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let message = socket.next().await.unwrap().unwrap();
            match message {
                Message::Text(text) => {
                    let value: Value = serde_json::from_str(&text).unwrap();
                    assert_ne!(value["type"], "error", "{value}");
                    if value["type"] == "ready" {
                        assert_eq!(value["resize"], false, "{value}");
                        assert!(value["width"].as_u64().unwrap() >= 100);
                        ready = true;
                    }
                    if value["type"] == "frame_id" {
                        frame_id = value["id"].as_u64();
                    }
                }
                Message::Binary(bytes) if bytes.first() == Some(&1) => {
                    assert!(ready);
                    let image = image::load_from_memory(&bytes[1..]).unwrap().to_rgb8();
                    let expected = image.pixels().filter(|pixel| {
                        if red {
                            pixel[0] > 200 && pixel[1] < 60 && pixel[2] < 60
                        } else {
                            pixel[2] > 200 && pixel[0] < 60 && pixel[1] < 60
                        }
                    }).count();
                    if let Some(id) = frame_id.take() {
                        socket.send(Message::Text(json!({"type":"frame_ack","id":id}).to_string().into())).await.unwrap();
                    }
                    if expected > (image.width() as usize * image.height() as usize) / 10 {
                        image.save(output).unwrap();
                        break json!({"width":image.width(),"height":image.height(),"expected_pixels":expected,"red":red});
                    }
                }
                _ => {}
            }
        }
    }).await.expect("fresh application-colored Niri frame was not delivered");
    socket.close(None).await.unwrap();
    observed
}

#[test]
#[ignore = "requires the isolated non-root GitHub Niri/Cage/foot candidate"]
fn persistent_niri_owner_survives_two_transport_generations() {
    assert_ne!(unsafe { libc::geteuid() }, 0);
    let report = PathBuf::from(std::env::var_os("LRD_NIRI_PROBE_REPORT").unwrap());
    let directory = tempfile::Builder::new().prefix("lrd.owner.")
        .permissions(fs::Permissions::from_mode(0o700)).tempdir().unwrap();
    let runtime = directory.path().join("session");
    let home = directory.path().join("home");
    let config_dir = home.join(".config/niri");
    fs::create_dir_all(&config_dir).unwrap();
    let included = config_dir.join("included.kdl");
    fs::write(&included, "input { keyboard { xkb { layout \"us\"; }; }; }\n").unwrap();
    let marker = directory.path().join("color");
    fs::write(&marker, "red\n").unwrap();
    let painter = directory.path().join("paint.sh");
    fs::write(&painter, r#"#!/bin/sh
last=
while :; do
    IFS= read -r color < "$1"
    if [ "$color" != "$last" ]; then
        case "$color" in
            red) printf '\033]11;#ff0000\007\033[2J\033[H\033[?25lNiri owner red\n' ;;
            blue) printf '\033]11;#0000ff\007\033[2J\033[H\033[?25lNiri owner blue\n' ;;
        esac
        last=$color
    fi
    /usr/bin/sleep 0.1
done
"#).unwrap();
    let configuration = format!(
        "include \"included.kdl\"\nspawn-at-startup \"/usr/bin/foot\" \"--app-id={APPLICATION}\" \"/bin/sh\" {} {}\n",
        serde_json::to_string(&painter).unwrap(),
        serde_json::to_string(&marker).unwrap()
    );
    let config = config_dir.join("config.kdl");
    fs::write(&config, &configuration).unwrap();
    let owner_log = report.join("owner.log");
    let owner = start("desktop-session", &home, &[
        "--runtime-dir", runtime.to_str().unwrap(),
        "--desktop-config", config.to_str().unwrap(),
    ], &owner_log);
    await_condition(&owner, &owner_log, || runtime.join("session.json").exists());
    let descriptor: Value = serde_json::from_slice(&fs::read(runtime.join("session.json")).unwrap()).unwrap();
    assert_eq!(descriptor["version"], 1);
    assert!(descriptor["signature"].is_null());
    let socket = PathBuf::from(descriptor["socket"].as_str().unwrap());
    let environment: Value = serde_json::from_slice(&fs::read(runtime.join("niri-environment.json")).unwrap()).unwrap();
    let ipc = PathBuf::from(environment["niri_socket"].as_str().unwrap());
    let niri = socket_pid(&socket);
    assert_eq!(niri, socket_pid(&ipc));
    let executable = fs::metadata(format!("/proc/{niri}/exe")).unwrap();
    let expected = fs::metadata("/usr/bin/niri").unwrap();
    assert_eq!((executable.dev(), executable.ino()), (expected.dev(), expected.ino()));
    await_condition(&owner, &owner_log, || application(&ipc).is_some());
    let window = application(&ipc).unwrap();
    let identities = [identity(owner.0.id()), identity(niri), identity(window.1)];
    let executor = tokio::runtime::Runtime::new().unwrap();
    let mut generations = Vec::new();
    for generation in 1..=2 {
        fs::write(&marker, if generation == 1 { "red\n" } else { "blue\n" }).unwrap();
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let transport_log = report.join(format!("transport-{generation}.log"));
        let transport = start("serve", &home, &[
            "--listen", "127.0.0.1", "--port", &port.to_string(),
            "--session-runtime", runtime.to_str().unwrap(),
        ], &transport_log);
        await_condition(&transport, &transport_log, || healthy(port));
        let frame = executor.block_on(colored_frame(port, generation == 1, &report.join(format!("owner-generation-{generation}.png"))));
        for expected in &identities {
            assert_eq!(&identity(expected.0), expected);
        }
        assert_eq!(application(&ipc), Some(window));
        drop(transport);
        assert!(!owner.exited());
        assert_eq!(application(&ipc), Some(window));
        generations.push(frame);
    }
    assert_eq!(fs::read_to_string(&config).unwrap(), configuration);
    assert_eq!(fs::read_to_string(&included).unwrap(), "input { keyboard { xkb { layout \"us\"; }; }; }\n");
    drop(owner);
    assert!(!runtime.join("session.json").exists());
    for expected in &identities {
        assert!(!Path::new(&format!("/proc/{}", expected.0)).exists());
    }
    let result = json!({"identities":identities,"window":window,"generations":generations,"native_includes_preserved":true,"pond_shell_tested":false,"platform_proxy_tested":false});
    fs::write(report.join("owner.json"), serde_json::to_vec_pretty(&result).unwrap()).unwrap();
    println!("{result}");
}
