//! Administrator-approved access to the current user's Windows lock screen.
use super::*;
use crate::backend::{Frame, InputCommand};
use crate::platform::native::Held;
use display_control::DisplayOutput;
use enigo::{Enigo, Keyboard, Settings};
use std::{
    io::Read,
    sync::{Arc, Mutex, OnceLock},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::windows::named_pipe::{NamedPipeServer, ServerOptions},
    time::timeout,
};
use windows::Win32::{
    Graphics::Gdi::{GetMonitorInfoW, HMONITOR, MONITORINFO},
    Security::{
        DuplicateTokenEx, SecurityImpersonation, SetTokenInformation, TOKEN_ALL_ACCESS,
        TOKEN_DUPLICATE, TOKEN_QUERY, TokenPrimary, TokenSessionId,
    },
    Storage::FileSystem::{
        FILE_GENERIC_READ, FILE_WRITE_DATA, OPEN_EXISTING, SECURITY_IDENTIFICATION,
        SECURITY_SQOS_PRESENT,
    },
    System::{
        Pipes::GetNamedPipeClientSessionId,
        Power::{ES_DISPLAY_REQUIRED, SetThreadExecutionState},
        StationsAndDesktops::{
            CloseDesktop, DESKTOP_READOBJECTS, GetThreadDesktop, GetUserObjectInformationW,
            OpenInputDesktop, UOI_NAME,
        },
        Threading::{GetCurrentProcess, GetCurrentThreadId, OpenProcessToken},
    },
    UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
};
use windows_capture::{
    dxgi_duplication_api::{DxgiDuplicationApi, DxgiDuplicationFormat, Error as DuplicationError},
    monitor::Monitor,
};

const NAME: &str = "LightOSRemoteDesktopLockScreen";
const SERVICE_COMMAND: &str = "lock-screen-service";
const MAX_INPUT: usize = 96 * 1024;
const MAX_FRAME: usize = 64 * 1024 * 1024;
const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(2);
const SYSTEM: &str = "S-1-5-18";

fn data_dir() -> Result<PathBuf> {
    Ok(known_folder(&FOLDERID_ProgramData)?.join(NAME))
}

fn executable_dir() -> Result<PathBuf> {
    Ok(known_folder(&FOLDERID_ProgramFiles)?.join("LightOS Remote Desktop Lock Screen"))
}

fn installed_owner(dir: &Path, sid: &str) -> Result<bool> {
    let owner = dir.join(OWNER_FILE);
    if !managed_exists(&owner)? {
        return Ok(false);
    }
    validate_root_chain(dir)?;
    validate_managed_path(dir, true, sid)?;
    validate_managed_path(&owner, false, sid)?;
    Ok(fs::read_to_string(owner)? == sid)
}

fn console_account_sid(session: u32) -> Result<String> {
    use windows::Win32::{
        Security::{LookupAccountNameW, SID_NAME_USE},
        System::RemoteDesktop::{
            WTS_INFO_CLASS, WTSDomainName, WTSFreeMemory, WTSQuerySessionInformationW, WTSUserName,
        },
    };
    fn information(session: u32, field: WTS_INFO_CLASS) -> Result<String> {
        let mut buffer = PWSTR::null();
        let mut length = 0;
        unsafe { WTSQuerySessionInformationW(None, session, field, &mut buffer, &mut length)? };
        let value = if length >= 2 && !buffer.0.is_null() {
            let chars = unsafe { std::slice::from_raw_parts(buffer.0, length as usize / 2) };
            let end = chars.iter().position(|&ch| ch == 0).unwrap_or(chars.len());
            String::from_utf16(&chars[..end]).map_err(anyhow::Error::from)
        } else {
            Ok(String::new())
        };
        unsafe { WTSFreeMemory(buffer.0.cast()) };
        value
    }
    let user = information(session, WTSUserName)?;
    let domain = information(session, WTSDomainName)?;
    ensure!(!user.is_empty(), "Windows console user is unavailable");
    let account = if domain.is_empty() {
        user
    } else {
        format!("{domain}\\{user}")
    };
    let account = wide(OsStr::new(&account));
    let mut sid = [0u32; 17];
    let mut sid_size = size_of_val(&sid) as u32;
    let mut name = [0u16; 256];
    let mut name_size = name.len() as u32;
    let mut account_type = SID_NAME_USE::default();
    let sid = PSID(sid.as_mut_ptr().cast());
    unsafe {
        LookupAccountNameW(
            PCWSTR::null(),
            PCWSTR(account.as_ptr()),
            Some(sid),
            &mut sid_size,
            Some(PWSTR(name.as_mut_ptr())),
            &mut name_size,
            &mut account_type,
        )?
    };
    sid_string(sid)
}

pub(in crate::platform) fn authorized() -> bool {
    static CHECK: OnceLock<Mutex<Option<(Instant, bool)>>> = OnceLock::new();
    let mut cached = CHECK
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if let Some((checked, allowed)) = *cached
        && checked.elapsed() < Duration::from_millis(500)
    {
        return allowed;
    }
    let valid = || -> Result<bool> {
        let owner = current_user_sid()?;
        let dir = data_dir()?;
        if !installed_owner(&dir, &owner)? {
            return Ok(false);
        }
        let manager = service_manager(ServiceManagerAccess::CONNECT)?;
        let service = manager.open_service(NAME, ServiceAccess::QUERY_STATUS)?;
        Ok(matches!(
            service.query_status()?.current_state,
            ServiceState::StartPending | ServiceState::Running
        ))
    };
    let allowed = valid().unwrap_or(false);
    *cached = Some((Instant::now(), allowed));
    allowed
}

pub(in crate::platform) fn request_setup() -> Result<()> {
    if authorized() {
        return Ok(());
    }
    let session = current_session_id()?;
    ensure!(
        active_console_session_id() == Some(session) && console_session_ready(session)?,
        "Windows lock-screen access must be approved in the active console"
    );
    let source = std::env::current_exe()?;
    let file = wide(source.as_os_str());
    let operation = wide(OsStr::new("runas"));
    let owner = current_user_sid()?;
    let command = command_line(&["install-lock-screen-service".into(), owner.into()]);
    let result = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(operation.as_ptr()),
            PCWSTR(file.as_ptr()),
            PCWSTR(command.as_ptr()),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    ensure!(
        result.0 as isize > 32,
        "Windows administrator approval was not granted"
    );
    Ok(())
}

pub fn install(owner_sid: &str) -> Result<()> {
    let session = current_session_id()?;
    ensure!(
        active_console_session_id() == Some(session) && console_session_ready(session)?,
        "Windows lock-screen access must be installed by the active console user"
    );
    let sid = current_user_sid()?;
    ensure!(
        owner_sid == sid || console_account_sid(session).is_ok_and(|current| current == owner_sid),
        "Windows lock-screen access must belong to the current console user"
    );
    let sid = owner_sid.to_owned();
    let dir = data_dir()?;
    let binaries = executable_dir()?;
    let owner = dir.join(OWNER_FILE);
    let executable = binaries.join(BINARY_FILE);
    validate_root_chain(&dir)?;
    validate_root_chain(&binaries)?;
    let manager =
        service_manager(ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE)?;
    let access = ServiceAccess::QUERY_CONFIG
        | ServiceAccess::QUERY_STATUS
        | ServiceAccess::START
        | ServiceAccess::STOP
        | ServiceAccess::DELETE;
    match manager.open_service(NAME, access) {
        Ok(service) => {
            let info = service.query_config()?;
            let expected = command_line(&[
                executable.as_os_str().to_os_string(),
                OsString::from(SERVICE_COMMAND),
            ]);
            ensure!(
                info.service_type == ServiceType::OWN_PROCESS
                    && info.account_name.as_ref().is_some_and(|account| {
                        let account = account.to_string_lossy();
                        account.eq_ignore_ascii_case("LocalSystem")
                            || account.eq_ignore_ascii_case("NT AUTHORITY\\SYSTEM")
                    })
                    && info.executable_path.as_os_str()
                        == OsString::from_wide(&expected[..expected.len() - 1])
                    && installed_owner(&dir, &sid)?,
                "unexpected existing Windows lock-screen service"
            );
            validate_managed_path(&binaries, true, &sid)?;
            validate_managed_path(&executable, false, &sid)?;
            if service.query_status()?.current_state == ServiceState::Stopped {
                service.start(&[] as &[&OsStr])?;
            }
            return Ok(());
        }
        Err(windows_service::Error::Winapi(error))
            if error.raw_os_error() == Some(ERROR_SERVICE_DOES_NOT_EXIST.0 as i32) => {}
        Err(error) => return Err(error.into()),
    }
    if managed_exists(&owner)? {
        ensure!(
            installed_owner(&dir, &sid)?,
            "Windows lock-screen access belongs to another user"
        );
    }
    prepare_directory(&dir, &settings_sddl(&sid, true), &sid)?;
    prepare_directory(&binaries, &binary_sddl(&sid, true), &sid)?;
    // The executable and the pinned owner are writable only by administrators and SYSTEM.
    let suffix = rand::random::<u128>();
    let staged = binaries.join(format!("host-{suffix:032x}.exe"));
    let staged_owner = dir.join(format!("owner-{suffix:032x}.sid"));
    let result = (|| -> Result<()> {
        write_secure_staging_file(&staged, &binary_sddl(&sid, false), &sid, |file| {
            io::copy(&mut fs::File::open(std::env::current_exe()?)?, file)?;
            Ok(())
        })?;
        write_secure_staging_file(&staged_owner, &settings_sddl(&sid, false), &sid, |file| {
            file.write_all(sid.as_bytes())?;
            Ok(())
        })?;
        replace(&staged, &executable, &sid)?;
        replace(&staged_owner, &owner, &sid)?;
        let info = ServiceInfo {
            name: NAME.into(),
            display_name: "Lazycat Remote — Lock Screen".into(),
            service_type: ServiceType::OWN_PROCESS,
            start_type: ServiceStartType::AutoStart,
            error_control: ServiceErrorControl::Normal,
            executable_path: executable.clone(),
            launch_arguments: vec![SERVICE_COMMAND.into()],
            dependencies: vec![],
            account_name: None,
            account_password: None,
        };
        let service = manager.create_service(&info, access)?;
        if let Err(error) = service.start(&[] as &[&OsStr]) {
            let _ = service.delete();
            return Err(error.into());
        }
        Ok(())
    })();
    for path in [&staged, &staged_owner] {
        let _ = fs::remove_file(path);
    }
    result
}

fn replace(source: &Path, target: &Path, owner: &str) -> Result<()> {
    use windows::Win32::Storage::FileSystem::{MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH};
    if managed_exists(target)? {
        validate_managed_path(target, false, owner)?;
    }
    let source = wide(source.as_os_str());
    let target = wide(target.as_os_str());
    unsafe {
        MoveFileExW(
            PCWSTR(source.as_ptr()),
            PCWSTR(target.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )?
    };
    Ok(())
}

pub fn run_service() -> Result<()> {
    service_dispatcher::start(NAME, lock_screen_main)?;
    Ok(())
}

define_windows_service!(lock_screen_main, service_main);

fn service_main(_: Vec<OsString>) {
    if let Err(error) = supervise() {
        tracing::error!(%error, "Windows lock-screen service stopped");
    }
}

fn supervise() -> Result<()> {
    let (stop_tx, stop_rx) = mpsc::channel();
    let status = service_control_handler::register(NAME, move |control| match control {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            let _ = stop_tx.send(());
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    })?;
    status.set_service_status(service_status(
        ServiceState::StartPending,
        ServiceControlAccept::empty(),
        1,
        ServiceExitCode::Win32(0),
    ))?;
    let result = (|| -> Result<()> {
        ensure!(
            current_user_sid()? == SYSTEM && current_session_id()? == 0,
            "Windows lock-screen supervisor requires the service account"
        );
        enable_console_token_access()?;
        let dir = data_dir()?;
        let owner = fs::read_to_string(dir.join(OWNER_FILE))?;
        ensure!(
            owner.starts_with("S-1-") && owner.len() < 256 && installed_owner(&dir, &owner)?,
            "Windows lock-screen user is not authorized"
        );
        status.set_service_status(service_status(
            ServiceState::Running,
            ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
            0,
            ServiceExitCode::Win32(0),
        ))?;
        let mut child = None::<LockChild>;
        let mut retry_at = Instant::now();
        while matches!(
            stop_rx.recv_timeout(Duration::from_millis(250)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ) {
            let locked = active_console_session_id().and_then(|id| {
                let session = session_details(id).ok()?;
                (session.active
                    && session.locked
                    && session.logon_time > 0
                    && console_user_sid(id).is_ok_and(|sid| sid == owner))
                .then_some((id, session.logon_time))
            });
            if child.as_ref().is_some_and(|worker| {
                Some(worker.scope) != locked
                    || unsafe { WaitForSingleObject(worker.process.raw(), 0) } != WAIT_TIMEOUT
            }) {
                child.take();
            }
            if child.is_none()
                && Instant::now() >= retry_at
                && let Some(scope) = locked
            {
                match LockChild::launch(scope, &owner) {
                    Ok(worker) => child = Some(worker),
                    Err(error) => {
                        tracing::warn!(%error, "Windows lock-screen worker could not start")
                    }
                }
                retry_at = Instant::now() + Duration::from_secs(2);
            }
        }
        Ok(())
    })();
    status.set_service_status(service_status(
        ServiceState::Stopped,
        ServiceControlAccept::empty(),
        0,
        if result.is_ok() {
            ServiceExitCode::Win32(0)
        } else {
            ServiceExitCode::ServiceSpecific(1)
        },
    ))?;
    result
}

fn enable_console_token_access() -> Result<()> {
    use windows::Win32::{
        Foundation::{ERROR_NOT_ALL_ASSIGNED, GetLastError, LUID},
        Security::{
            AdjustTokenPrivileges, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW,
            SE_PRIVILEGE_ENABLED, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES,
        },
    };
    let mut id = LUID::default();
    unsafe { LookupPrivilegeValueW(PCWSTR::null(), windows::core::w!("SeTcbPrivilege"), &mut id)? };
    let mut handle = HANDLE::default();
    unsafe {
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &mut handle,
        )?
    };
    let token = OwnedHandle::new(handle);
    let privileges = TOKEN_PRIVILEGES {
        PrivilegeCount: 1,
        Privileges: [LUID_AND_ATTRIBUTES {
            Luid: id,
            Attributes: SE_PRIVILEGE_ENABLED,
        }],
    };
    unsafe { AdjustTokenPrivileges(token.raw(), false, Some(&privileges), 0, None, None)? };
    ensure!(
        unsafe { GetLastError() } != ERROR_NOT_ALL_ASSIGNED,
        "the Windows lock-screen service lacks console token access"
    );
    Ok(())
}

struct LockChild {
    scope: (u32, i64),
    process: OwnedHandle,
    _job: OwnedHandle,
}

impl LockChild {
    fn launch(scope: (u32, i64), owner: &str) -> Result<Self> {
        let (session, logon) = scope;
        let token = system_token(session)?;
        let executable = std::env::current_exe()?;
        let args = [
            executable.as_os_str().to_os_string(),
            "lock-screen-worker".into(),
            session.to_string().into(),
            logon.to_string().into(),
            owner.into(),
        ];
        let mut command = command_line(&args);
        let path = wide(executable.as_os_str());
        let desktop = wide(OsStr::new("winsta0\\winlogon"));
        let startup = STARTUPINFOW {
            cb: size_of::<STARTUPINFOW>() as u32,
            lpDesktop: PWSTR(desktop.as_ptr().cast_mut()),
            ..Default::default()
        };
        let mut info = PROCESS_INFORMATION::default();
        let job = create_kill_on_close_job()?;
        unsafe {
            CreateProcessAsUserW(
                Some(token.raw()),
                PCWSTR(path.as_ptr()),
                Some(PWSTR(command.as_mut_ptr())),
                None,
                None,
                false,
                CREATE_SUSPENDED,
                None,
                PCWSTR::null(),
                &startup,
                &mut info,
            )?
        };
        let process = OwnedHandle::new(info.hProcess);
        let thread = OwnedHandle::new(info.hThread);
        if let Err(error) = unsafe { AssignProcessToJobObject(job.raw(), process.raw()) } {
            let _ = unsafe { TerminateProcess(process.raw(), 1) };
            return Err(error.into());
        }
        ensure!(
            unsafe { ResumeThread(thread.raw()) } != u32::MAX,
            "start Windows lock-screen worker"
        );
        Ok(Self {
            scope,
            process,
            _job: job,
        })
    }
}

fn system_token(session: u32) -> Result<OwnedHandle> {
    let mut source = HANDLE::default();
    unsafe {
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_QUERY | TOKEN_DUPLICATE,
            &mut source,
        )?
    };
    let source = OwnedHandle::new(source);
    let mut token = HANDLE::default();
    unsafe {
        DuplicateTokenEx(
            source.raw(),
            TOKEN_ALL_ACCESS,
            None,
            SecurityImpersonation,
            TokenPrimary,
            &mut token,
        )?
    };
    let token = OwnedHandle::new(token);
    unsafe {
        SetTokenInformation(
            token.raw(),
            TokenSessionId,
            (&raw const session).cast(),
            size_of::<u32>() as u32,
        )?
    };
    Ok(token)
}

fn pipe_name(session: u32) -> String {
    format!(r"\\.\pipe\LightOSRemoteDesktopLockScreen-{session}")
}

fn desktop_is_winlogon(handle: HANDLE) -> bool {
    let mut name = [0u16; 32];
    let result = unsafe {
        GetUserObjectInformationW(
            handle,
            UOI_NAME,
            Some(name.as_mut_ptr().cast()),
            (name.len() * 2) as u32,
            None,
        )
    };
    let end = name.iter().position(|&c| c == 0).unwrap_or(name.len());
    result.is_ok() && String::from_utf16_lossy(&name[..end]).eq_ignore_ascii_case("winlogon")
}

fn locked_console(session: u32, logon: i64) -> bool {
    if active_console_session_id() != Some(session)
        || !session_details(session)
            .is_ok_and(|info| info.active && info.locked && info.logon_time == logon)
    {
        return false;
    }
    let Ok(input) = (unsafe { OpenInputDesktop(Default::default(), false, DESKTOP_READOBJECTS) })
    else {
        return false;
    };
    let secure = desktop_is_winlogon(HANDLE(input.0));
    let _ = unsafe { CloseDesktop(input) };
    secure
}

pub fn run_worker(session: u32, logon: i64, owner: &str) -> Result<()> {
    ensure!(
        current_user_sid()? == SYSTEM
            && current_session_id()? == session
            && console_user_sid(session)? == owner
            && owner.starts_with("S-1-")
            && owner
                .chars()
                .skip(4)
                .all(|c| c.is_ascii_digit() || c == '-'),
        "Windows lock-screen worker identity is invalid"
    );
    let desk = unsafe { GetThreadDesktop(GetCurrentThreadId()) }?;
    ensure!(
        desktop_is_winlogon(HANDLE(desk.0)) && locked_console(session, logon),
        "Windows lock-screen worker must start on the locked console"
    );
    // The only writing clients are SYSTEM and this console user. A client gets
    // write-data, never FILE_CREATE_PIPE_INSTANCE, so it cannot impersonate the server.
    let policy =
        SecurityDescriptor::from_sddl(&format!("O:SYG:SYD:P(A;;GA;;;SY)(A;;0x12008b;;;{owner})"))?;
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: policy.0.0,
        bInheritHandle: BOOL(0),
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let pipe = unsafe {
            ServerOptions::new()
                .first_pipe_instance(true)
                .reject_remote_clients(true)
                .create_with_security_attributes_raw(
                    pipe_name(session),
                    (&raw mut attributes).cast(),
                )?
        };
        let mut worker = LockWorker::new(session, logon)?;
        worker.serve(pipe).await
    })
}

fn connect() -> Result<fs::File> {
    use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PIPE_BUSY};
    let name = wide(OsStr::new(&pipe_name(current_session_id()?)));
    let deadline = Instant::now() + Duration::from_millis(250);
    let handle = loop {
        match unsafe {
            CreateFileW(
                PCWSTR(name.as_ptr()),
                FILE_GENERIC_READ.0 | FILE_WRITE_DATA.0,
                FILE_SHARE_NONE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                None,
            )
        } {
            Ok(handle) => break handle,
            Err(error)
                if (error.code() == windows::core::HRESULT::from_win32(ERROR_PIPE_BUSY.0)
                    || error.code()
                        == windows::core::HRESULT::from_win32(ERROR_FILE_NOT_FOUND.0))
                    && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(error.into()),
        }
    };
    let file = unsafe { fs::File::from_raw_handle(handle.0) };
    let mut owner = PSID::default();
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    unsafe {
        GetSecurityInfo(
            handle,
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            Some(&mut owner),
            None,
            None,
            None,
            Some(&mut descriptor),
        )
        .ok()?
    };
    let verified = !owner.0.is_null() && sid_string(owner).is_ok_and(|sid| sid == SYSTEM);
    unsafe { LocalFree(Some(HLOCAL(descriptor.0))) };
    ensure!(verified, "Windows lock-screen pipe is not owned by SYSTEM");
    Ok(file)
}

pub(in crate::platform) struct Capture;

impl crate::platform::NativeCapture for Capture {
    fn next_frame(&mut self, _: Duration) -> Result<Option<Frame>> {
        let mut pipe = connect()?;
        pipe.write_all(&[1])?;
        let mut header = [0u8; 9];
        pipe.read_exact(&mut header)?;
        if header[0] == 0 {
            pipe.write_all(&[0])?;
            return Ok(None);
        }
        ensure!(header[0] == 1, "invalid Windows lock-screen frame");
        let width = u32::from_le_bytes(header[1..5].try_into()?);
        let height = u32::from_le_bytes(header[5..9].try_into()?);
        let mut rgba = vec![0; frame_size(width, height)?];
        pipe.read_exact(&mut rgba)?;
        pipe.write_all(&[0])?;
        Ok(Some(Frame::new(width, height, Arc::from(rgba))))
    }
}

pub(in crate::platform) fn send_input(command: &InputCommand) -> Result<()> {
    let data = serde_json::to_vec(command)?;
    ensure!(
        data.len() <= MAX_INPUT,
        "Windows lock-screen input is too large"
    );
    let mut pipe = connect()?;
    pipe.write_all(&[2])?;
    pipe.write_all(&(data.len() as u32).to_le_bytes())?;
    pipe.write_all(&data)?;
    let mut result = [0];
    pipe.read_exact(&mut result)?;
    pipe.write_all(&[0])?;
    ensure!(result[0] == 1, "Windows lock-screen input is unavailable");
    Ok(())
}

pub(in crate::platform) fn wake_display() -> Result<()> {
    let mut pipe = connect()?;
    pipe.write_all(&[3])?;
    let mut result = [0];
    pipe.read_exact(&mut result)?;
    pipe.write_all(&[0])?;
    ensure!(result[0] == 1, "could not wake the Windows lock display");
    Ok(())
}

fn frame_size(width: u32, height: u32) -> Result<usize> {
    ensure!(
        (1..=16384).contains(&width) && (1..=16384).contains(&height),
        "invalid Windows lock-screen size"
    );
    let size = width as usize * height as usize * 4;
    ensure!(size <= MAX_FRAME, "Windows lock-screen frame is too large");
    Ok(size)
}

struct LockWorker {
    session: u32,
    logon: i64,
    capture: Option<DxgiDuplicationApi>,
    frame: Option<(u32, u32, Vec<u8>)>,
    output: DisplayOutput,
    input: Enigo,
    held: Held,
    last_input: Instant,
    last_frame_sent: Option<Instant>,
}

impl LockWorker {
    fn new(session: u32, logon: i64) -> Result<Self> {
        super::super::process_dpi_ready()?;
        let monitor = Monitor::primary()?;
        let mut rect = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        unsafe { GetMonitorInfoW(HMONITOR(monitor.as_raw_hmonitor()), &mut rect).ok()? };
        let output = DisplayOutput {
            name: monitor.device_name()?,
            description: "Windows lock screen".into(),
            width: monitor.width()?,
            height: monitor.height()?,
            x: rect.rcMonitor.left,
            y: rect.rcMonitor.top,
            scale: 1.0,
        };
        let input = Enigo::new(&Settings {
            open_prompt_to_get_permissions: false,
            ..Default::default()
        })?;
        Ok(Self {
            session,
            logon,
            capture: None,
            frame: None,
            output,
            input,
            held: Held::default(),
            last_input: Instant::now(),
            last_frame_sent: None,
        })
    }

    async fn serve(&mut self, mut pipe: NamedPipeServer) -> Result<()> {
        while locked_console(self.session, self.logon) {
            if self.last_input.elapsed() >= EXCHANGE_TIMEOUT {
                self.held.release(&mut self.input)?;
            }
            match timeout(Duration::from_millis(100), pipe.connect()).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => return Err(error.into()),
                Err(_) => continue,
            }
            // A disappearing client or a partial message never carries over to the next connection.
            let mut client_session = u32::MAX;
            if unsafe {
                GetNamedPipeClientSessionId(HANDLE(pipe.as_raw_handle()), &mut client_session)
            }
            .is_ok()
                && client_session == self.session
            {
                let _ = timeout(EXCHANGE_TIMEOUT, self.exchange(&mut pipe)).await;
            }
            pipe.disconnect()?;
        }
        Ok(())
    }

    async fn exchange(&mut self, pipe: &mut NamedPipeServer) -> Result<()> {
        if !locked_console(self.session, self.logon) {
            return Ok(());
        }
        match pipe.read_u8().await? {
            1 => {
                let changed = self.refresh_frame()?;
                if let Some((width, height, pixels)) = self.frame.as_ref()
                    && (changed
                        || self
                            .last_frame_sent
                            .is_none_or(|time| time.elapsed() >= Duration::from_millis(500)))
                    && locked_console(self.session, self.logon)
                {
                    pipe.write_u8(1).await?;
                    pipe.write_u32_le(*width).await?;
                    pipe.write_u32_le(*height).await?;
                    pipe.write_all(pixels).await?;
                    self.last_frame_sent = Some(Instant::now());
                } else {
                    pipe.write_all(&[0; 9]).await?;
                }
            }
            2 => {
                let size = pipe.read_u32_le().await? as usize;
                ensure!(size <= MAX_INPUT, "oversized Windows lock-screen input");
                let mut data = vec![0; size];
                pipe.read_exact(&mut data).await?;
                let command: InputCommand = serde_json::from_slice(&data)?;
                data.fill(0);
                super::super::super::native::validate_command(&command)?;
                let session = self.session;
                let logon = self.logon;
                if (self.frame.is_some() || matches!(&command, InputCommand::Release))
                    && locked_console(session, logon)
                {
                    self.last_input = Instant::now();
                    if let InputCommand::Text { text } = &command {
                        ensure!(
                            text.len() <= 1024 && !text.chars().any(char::is_control),
                            "invalid Windows lock-screen text"
                        );
                        self.held.release(&mut self.input)?;
                        for character in text.chars().take_while(|_| locked_console(session, logon))
                        {
                            self.input.text(character.encode_utf8(&mut [0; 4]))?;
                        }
                    } else {
                        self.held.apply(
                            &super::super::WindowsPlatform,
                            &mut self.input,
                            &command,
                            &self.output,
                            || locked_console(session, logon),
                        )?;
                    }
                    pipe.write_u8(1).await?;
                } else {
                    pipe.write_u8(0).await?;
                }
            }
            3 => {
                let woke = locked_console(self.session, self.logon)
                    && unsafe { SetThreadExecutionState(ES_DISPLAY_REQUIRED) }.0 != 0;
                pipe.write_u8(u8::from(woke)).await?;
            }
            _ => return Ok(()),
        }
        // DisconnectNamedPipe discards unread bytes. The client acknowledges
        // receipt before this pipe instance is made available to the next caller.
        ensure!(
            pipe.read_u8().await? == 0,
            "invalid Windows lock-screen acknowledgement"
        );
        Ok(())
    }

    fn refresh_frame(&mut self) -> Result<bool> {
        if self.capture.is_none() {
            self.capture = Some(DxgiDuplicationApi::new_options(
                Monitor::primary()?,
                &[DxgiDuplicationFormat::Bgra8],
            )?);
        }
        let capture = self
            .capture
            .as_mut()
            .context("Windows lock-screen capture is missing")?;
        let result = (|| -> Result<bool> {
            let mut frame = match capture.acquire_next_frame(30) {
                Ok(frame) => frame,
                Err(DuplicationError::Timeout) => return Ok(false),
                Err(error) => return Err(error.into()),
            };
            let buffer = frame.buffer()?;
            let width = buffer.width();
            let height = buffer.height();
            let size = frame_size(width, height)?;
            ensure!(
                buffer.format() == DxgiDuplicationFormat::Bgra8,
                "unexpected Windows lock-screen pixel format"
            );
            let mut padded = Vec::new();
            let source = buffer.as_nopadding_buffer(&mut padded);
            ensure!(
                source.len() == size,
                "unexpected Windows lock-screen frame size"
            );
            let mut pixels = source.to_vec();
            for pixel in pixels.chunks_exact_mut(4) {
                pixel.swap(0, 2);
                pixel[3] = 255;
            }
            self.frame = Some((width, height, pixels));
            Ok(true)
        })();
        if result.is_err() {
            self.capture = None;
            self.frame = None;
        }
        result
    }
}
