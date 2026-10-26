//! SCM supervisor for one pinned console user's interactive host.

use std::{
    ffi::{OsStr, OsString, c_void},
    fs::{self, OpenOptions},
    io::{self, Write},
    mem::size_of,
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::{MetadataExt, OpenOptionsExt},
        io::{AsRawHandle, FromRawHandle},
    },
    path::{Component, Path, PathBuf, Prefix},
    ptr,
    sync::mpsc,
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result, bail, ensure};
use windows::{
    Win32::{
        Foundation::{
            CloseHandle, ERROR_SERVICE_DOES_NOT_EXIST, HANDLE, HLOCAL, LocalFree, WAIT_FAILED,
            WAIT_OBJECT_0, WAIT_TIMEOUT,
        },
        Security::Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            GetSecurityInfo, SDDL_REVISION_1, SE_FILE_OBJECT, SetNamedSecurityInfoW,
        },
        Security::{
            ACCESS_ALLOWED_ACE, ACE_HEADER, DACL_SECURITY_INFORMATION, GetAce,
            GetSecurityDescriptorControl, GetSecurityDescriptorDacl, GetSecurityDescriptorOwner,
            OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
            PSID, SE_DACL_PROTECTED, SECURITY_ATTRIBUTES,
        },
        Storage::FileSystem::{
            CREATE_NEW, CreateDirectoryW, CreateFileW, FILE_ATTRIBUTE_NORMAL,
            FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
            FILE_GENERIC_WRITE, FILE_SHARE_NONE, MOVEFILE_DELAY_UNTIL_REBOOT, MoveFileExW,
        },
        System::{
            Com::CoTaskMemFree,
            Environment::{CreateEnvironmentBlock, DestroyEnvironmentBlock},
            JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
                SetInformationJobObject, TerminateJobObject,
            },
            RemoteDesktop::WTSQueryUserToken,
            Threading::{
                CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateEventW, CreateProcessAsUserW,
                OpenEventW, PROCESS_INFORMATION, ResumeThread, STARTUPINFOW,
                SYNCHRONIZATION_ACCESS_RIGHTS, SetEvent, TerminateProcess, WaitForSingleObject,
            },
        },
        UI::Shell::{
            FOLDERID_ProgramData, FOLDERID_ProgramFiles, KF_FLAG_DEFAULT, SHGetKnownFolderPath,
        },
    },
    core::{BOOL, GUID, PCWSTR, PWSTR},
};
use windows_service::{
    define_windows_service,
    service::{
        Service, ServiceAccess, ServiceControl, ServiceControlAccept, ServiceErrorControl,
        ServiceExitCode, ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
    },
    service_control_handler::{self, ServiceControlHandlerResult},
    service_dispatcher,
    service_manager::{ServiceManager, ServiceManagerAccess},
};

use super::{
    active_console_session_id, console_session_ready, current_session_id, current_user_sid,
    session_details, token_sid,
};

const SERVICE_NAME: &str = "LightOSRemoteDesktop";
const DISPLAY_NAME: &str = "LightOS Remote Desktop";
const STOP_WAIT: Duration = Duration::from_secs(10);
const CONFIG_FILE: &str = "host.json";
const OWNER_FILE: &str = "owner.sid";
const BINARY_FILE: &str = "lightos-remoted.exe";
const PENDING_CONFIG_FILE: &str = "host.pending.json";
const PENDING_BINARY_FILE: &str = "lightos-remoted.pending.exe";
const PENDING_READY_FILE: &str = "stage.ready";
const PENDING_CONFIG_TEMP_FILE: &str = "host.pending.writing.json";
const PENDING_BINARY_TEMP_FILE: &str = "lightos-remoted.pending.writing.exe";
const PENDING_READY_TEMP_FILE: &str = "stage.writing";

#[derive(Debug, PartialEq, Eq)]
enum PendingDisposition {
    None,
    Activate,
    DiscardIncomplete,
}

fn pending_disposition(ready: bool, config: bool, binary: bool) -> Result<PendingDisposition> {
    if ready {
        ensure!(config && binary, "Windows staged update is incomplete");
        Ok(PendingDisposition::Activate)
    } else if config || binary {
        Ok(PendingDisposition::DiscardIncomplete)
    } else {
        Ok(PendingDisposition::None)
    }
}

struct OwnedHandle(HANDLE);
impl OwnedHandle {
    fn new(handle: HANDLE) -> Self {
        Self(handle)
    }
    fn raw(&self) -> HANDLE {
        self.0
    }
}
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            let _ = unsafe { CloseHandle(self.0) };
        }
    }
}

fn service_dir() -> Result<PathBuf> {
    Ok(known_folder(&FOLDERID_ProgramData)?.join("LightOSRemoteDesktop"))
}

fn binary_dir() -> Result<PathBuf> {
    Ok(known_folder(&FOLDERID_ProgramFiles)?.join("LightOS Remote Desktop"))
}

fn known_folder(id: &GUID) -> Result<PathBuf> {
    let raw = unsafe { SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None)? };
    let result = (|| -> Result<PathBuf> {
        ensure!(!raw.0.is_null(), "Windows known-folder path is empty");
        let mut len = 0;
        while len < 32_767 && unsafe { *raw.0.add(len) } != 0 {
            len += 1;
        }
        ensure!(len < 32_767, "Windows known-folder path is too long");
        let path = PathBuf::from(OsString::from_wide(unsafe {
            std::slice::from_raw_parts(raw.0, len)
        }));
        let mut components = path.components();
        ensure!(
            matches!(components.next(), Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)))
                && matches!(components.next(), Some(Component::RootDir)),
            "Windows known-folder path must be on a local absolute drive"
        );
        Ok(path)
    })();
    unsafe { CoTaskMemFree(Some(raw.0.cast())) };
    result
}

fn service_manager(access: ServiceManagerAccess) -> Result<ServiceManager> {
    ServiceManager::local_computer(None::<&str>, access).context("open Windows service manager")
}

fn verify_existing_service(service: &Service, executable: &Path) -> Result<ServiceInfo> {
    let old = service.query_config()?;
    ensure!(
        old.service_type == ServiceType::OWN_PROCESS,
        "existing service is not an own-process supervisor"
    );
    ensure!(
        old.account_name.as_ref().is_none_or(|account| {
            let account = account.to_string_lossy();
            account.eq_ignore_ascii_case("LocalSystem")
                || account.eq_ignore_ascii_case("NT AUTHORITY\\SYSTEM")
        }),
        "existing service does not run as LocalSystem"
    );
    let expected = command_line(&[
        executable.as_os_str().to_os_string(),
        OsString::from("service-run"),
    ]);
    ensure!(
        old.executable_path.as_os_str() == OsString::from_wide(&expected[..expected.len() - 1]),
        "existing service has an unexpected executable or arguments"
    );
    Ok(ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: old.display_name,
        service_type: old.service_type,
        start_type: old.start_type,
        error_control: old.error_control,
        executable_path: executable.to_path_buf(),
        launch_arguments: vec![OsString::from("service-run")],
        dependencies: old.dependencies,
        account_name: None,
        account_password: None,
    })
}

pub fn install_service(options: crate::install::ServiceOptions, rollback: bool) -> Result<()> {
    ensure!(!rollback, "Windows service rollback is unsupported");
    ensure!(
        options.print_unit.is_none(),
        "systemd unit export is unavailable on Windows"
    );
    ensure!(
        !options.package_managed
            && !options.snapshot
            && !options.legacy
            && options.test_root.is_none(),
        "Linux service lifecycle flags are unavailable on Windows"
    );
    let dir = service_dir()?;
    let source = std::env::current_exe().context("locate Windows service executable")?;
    let bin_dir = binary_dir()?;
    let executable = bin_dir.join(BINARY_FILE);
    if options.dry_run {
        println!(
            "service: {SERVICE_NAME}\nsource: {}\nexecutable: {}\nsettings: {}\naction: {}",
            source.display(),
            executable.display(),
            dir.join(CONFIG_FILE).display(),
            if options.remove {
                "remove"
            } else if options.stage_only {
                "stage"
            } else {
                "install and start"
            }
        );
        return Ok(());
    }
    validate_root_chain(&dir)?;
    validate_root_chain(&bin_dir)?;
    if options.remove {
        return remove_service(&dir, &bin_dir);
    }
    let session = active_console_session_id().context("no active console user session")?;
    ensure!(
        session != 0 && session == current_session_id()?,
        "install-service must run as the logged-in console user (elevation is allowed)"
    );
    ensure!(
        console_session_ready(session)?,
        "the console session is locked or inactive"
    );
    let owner_sid = current_user_sid()?;
    let config = dir.join(CONFIG_FILE);
    let owner = dir.join(OWNER_FILE);
    if managed_exists(&dir)? {
        validate_managed_path(&dir, true, &owner_sid)?;
    } else {
        prepare_directory(&dir, &settings_sddl(&owner_sid, true), &owner_sid)?;
    }
    if managed_exists(&bin_dir)? {
        validate_managed_path(&bin_dir, true, &owner_sid)?;
    } else {
        prepare_directory(&bin_dir, &binary_sddl(&owner_sid, true), &owner_sid)?;
    }
    let owner_present = managed_exists(&owner)?;
    let executable_present = managed_exists(&executable)?;
    if owner_present {
        validate_managed_path(&owner, false, &owner_sid)?;
        ensure!(
            fs::read_to_string(&owner)? == owner_sid,
            "this Windows service belongs to a different console user; remove it before reinstalling"
        );
    }
    if executable_present {
        validate_managed_path(&executable, false, &owner_sid)?;
    }
    let manager =
        service_manager(ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE)?;
    let access = ServiceAccess::QUERY_STATUS
        | ServiceAccess::QUERY_CONFIG
        | ServiceAccess::STOP
        | ServiceAccess::START
        | ServiceAccess::CHANGE_CONFIG
        | ServiceAccess::DELETE;
    let mut service = match manager.open_service(SERVICE_NAME, access) {
        Ok(service) => Some(service),
        Err(windows_service::Error::Winapi(error))
            if error.raw_os_error() == Some(ERROR_SERVICE_DOES_NOT_EXIST.0 as i32) =>
        {
            None
        }
        Err(error) => return Err(error).context("open existing Windows desktop service"),
    };
    let previous_info = if let Some(existing) = &service {
        ensure!(
            owner_present,
            "existing Windows service has no pinned owner; remove it before reinstalling"
        );
        Some(verify_existing_service(existing, &executable)?)
    } else {
        None
    };
    let had_config = managed_exists(&config)?;
    if had_config {
        validate_managed_path(&config, false, &owner_sid)?;
    }
    if service.is_some() {
        ensure!(
            executable_present && had_config,
            "existing Windows service is missing its managed executable or settings"
        );
    }
    let pending_config = dir.join(PENDING_CONFIG_FILE);
    let pending_binary = bin_dir.join(PENDING_BINARY_FILE);
    let pending_ready = dir.join(PENDING_READY_FILE);
    let pending_config_temp = dir.join(PENDING_CONFIG_TEMP_FILE);
    let pending_binary_temp = bin_dir.join(PENDING_BINARY_TEMP_FILE);
    let pending_ready_temp = dir.join(PENDING_READY_TEMP_FILE);
    if options.stage_only && service.is_some() {
        let settings = crate::install::HostSettings::from_environment()?;
        return stage_existing_update(
            &source,
            &settings,
            &pending_binary,
            &pending_config,
            &pending_ready,
            &owner_sid,
        );
    }
    let ready = managed_exists(&pending_ready)?;
    let pending_config_exists = managed_exists(&pending_config)?;
    let pending_binary_exists = managed_exists(&pending_binary)?;
    let disposition = pending_disposition(ready, pending_config_exists, pending_binary_exists)?;
    let use_pending = disposition == PendingDisposition::Activate;
    if ready {
        ensure!(
            service.is_some(),
            "Windows staged update has no managed service"
        );
        validate_managed_path(&pending_ready, false, &owner_sid)?;
        ensure!(
            fs::read(&pending_ready)? == b"ready",
            "Windows staged update marker is invalid"
        );
    }
    if pending_config_exists {
        if ready {
            validate_managed_path(&pending_config, false, &owner_sid)?;
        } else {
            validate_incomplete_pending_config(&pending_config, &owner_sid)?;
        }
    }
    if pending_binary_exists {
        validate_managed_path(&pending_binary, false, &owner_sid)?;
    }
    let temporary = [
        &pending_config_temp,
        &pending_binary_temp,
        &pending_ready_temp,
    ];
    for path in temporary {
        if managed_exists(path)? {
            validate_managed_path(path, false, &owner_sid)?;
        }
    }
    if disposition == PendingDisposition::DiscardIncomplete {
        for path in [&pending_config, &pending_binary] {
            if managed_exists(path)? {
                fs::remove_file(path)?;
            }
        }
    }
    for path in temporary {
        if managed_exists(path)? {
            fs::remove_file(path)?;
        }
    }
    let settings = if use_pending {
        None
    } else {
        Some(crate::install::HostSettings::from_environment()?)
    };
    let update_source = if use_pending {
        &pending_binary
    } else {
        &source
    };
    let prior_running = match &service {
        Some(service) => service.query_status()?.current_state != ServiceState::Stopped,
        None => false,
    };
    let in_place = !use_pending
        && executable_present
        && fs::canonicalize(&executable)? == fs::canonicalize(&source)?;
    let nonce = rand::random::<u128>();
    let staged = bin_dir.join(format!("lightos-remoted.{nonce:032x}.staged.exe"));
    let prior_binary = bin_dir.join(format!("lightos-remoted.{nonce:032x}.previous.exe"));
    let prior_config = dir.join(format!("host.{nonce:032x}.previous.json"));
    let prior_owner = dir.join(format!("owner.{nonce:032x}.previous.sid"));
    if !in_place {
        fs::copy(update_source, &staged).context("stage Windows desktop executable")?;
        secure_binary(&staged, &owner_sid, false)?;
    }
    let had_owner = owner_present;
    if had_config {
        fs::copy(&config, &prior_config)?;
        secure_path(&prior_config, &owner_sid, false)?;
    }
    if had_owner {
        fs::copy(&owner, &prior_owner)?;
        secure_path(&prior_owner, &owner_sid, false)?;
    }
    let info = ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: OsString::from(DISPLAY_NAME),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: executable.clone(),
        launch_arguments: vec![OsString::from("service-run")],
        dependencies: vec![],
        account_name: None, // LocalSystem, required for WTSQueryUserToken.
        account_password: None,
    };
    let mut created = false;
    let mut swapped_binary = false;
    let mut changed_service = false;
    let result = (|| -> Result<()> {
        if let Some(existing) = &service {
            stop_service(existing)?;
        }
        if let Some(settings) = &settings {
            settings.save(&config)?;
        } else {
            let bytes = crate::private_storage::current()
                .read(&pending_config, 64 * 1024)?
                .context("staged Windows service settings are missing")?;
            crate::private_storage::current().write(&config, &bytes)?;
        }
        secure_path(&config, &owner_sid, false)?;
        fs::write(&owner, &owner_sid).context("write Windows service owner identity")?;
        secure_path(&owner, &owner_sid, false)?;
        if !in_place {
            if executable_present {
                fs::rename(&executable, &prior_binary)
                    .context("stage previous Windows service executable")?;
            }
            fs::rename(&staged, &executable)
                .context("activate staged Windows service executable")?;
            swapped_binary = true;
            secure_binary(&executable, &owner_sid, false)?;
        }
        if let Some(existing) = &service {
            existing
                .change_config(&info)
                .context("update Windows desktop service")?;
            changed_service = true;
        } else {
            service = Some(
                manager
                    .create_service(&info, access)
                    .context("create Windows desktop service")?,
            );
            created = true;
        }
        let service = service.as_ref().expect("service has been created");
        service.set_description("Supervises the LightOS remote desktop in the installing user's interactive console session")?;
        if !options.stage_only {
            service
                .start(&[] as &[&OsStr])
                .context("start Windows desktop supervisor service")?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        let rollback = rollback_install(
            service.as_ref(),
            created,
            prior_running,
            changed_service,
            previous_info.as_ref(),
            swapped_binary,
            &executable,
            &prior_binary,
            &config,
            &prior_config,
            had_config,
            &owner,
            &prior_owner,
            had_owner,
            &owner_sid,
        );
        let _ = fs::remove_file(&staged);
        return match rollback {
            Ok(()) => Err(error),
            Err(rollback_error) => Err(anyhow::anyhow!(
                "{error:#}; rollback failed: {rollback_error:#}"
            )),
        };
    }
    for path in [&prior_binary, &prior_config, &prior_owner, &staged] {
        if path.exists() {
            let _ = fs::remove_file(path);
        }
    }
    if use_pending {
        match fs::remove_file(&pending_ready) {
            Ok(()) => {
                for path in [&pending_config, &pending_binary] {
                    if let Err(error) = fs::remove_file(path) {
                        tracing::warn!(%error, path = %path.display(), "remove consumed Windows staged update");
                    }
                }
            }
            Err(error) => tracing::warn!(%error, "remove Windows staged update marker"),
        }
    }
    Ok(())
}

fn write_secure_staging_file(
    path: &Path,
    sddl: &str,
    sid: &str,
    write: impl FnOnce(&mut fs::File) -> Result<()>,
) -> Result<()> {
    let descriptor = SecurityDescriptor::from_sddl(sddl)?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0.0,
        bInheritHandle: BOOL(0),
    };
    let path_wide = wide(path.as_os_str());
    let handle = unsafe {
        CreateFileW(
            PCWSTR(path_wide.as_ptr()),
            FILE_GENERIC_WRITE.0,
            FILE_SHARE_NONE,
            Some(&attributes),
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )?
    };
    let mut file = unsafe { fs::File::from_raw_handle(handle.0) };
    let result = write(&mut file).and_then(|()| file.sync_all().map_err(Into::into));
    drop(file);
    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    result?;
    let validation = validate_managed_path(path, false, sid);
    if validation.is_err() {
        let _ = fs::remove_file(path);
    }
    validation
}

fn stage_existing_update(
    source: &Path,
    settings: &crate::install::HostSettings,
    pending_binary: &Path,
    pending_config: &Path,
    ready: &Path,
    sid: &str,
) -> Result<()> {
    let binary_temp = pending_binary.with_file_name(PENDING_BINARY_TEMP_FILE);
    let config_temp = pending_config.with_file_name(PENDING_CONFIG_TEMP_FILE);
    let ready_temp = ready.with_file_name(PENDING_READY_TEMP_FILE);
    let ready_exists = managed_exists(ready)?;
    let config_exists = managed_exists(pending_config)?;
    let binary_exists = managed_exists(pending_binary)?;
    pending_disposition(ready_exists, config_exists, binary_exists)?;
    if ready_exists {
        validate_managed_path(ready, false, sid)?;
    }
    if config_exists {
        if ready_exists {
            validate_managed_path(pending_config, false, sid)?;
        } else {
            validate_incomplete_pending_config(pending_config, sid)?;
        }
    }
    if binary_exists {
        validate_managed_path(pending_binary, false, sid)?;
    }
    for path in [&binary_temp, &config_temp, &ready_temp] {
        if managed_exists(path)? {
            validate_managed_path(path, false, sid)?;
        }
    }
    settings.config()?;
    let config_bytes = serde_json::to_vec(settings)?;
    ensure!(
        config_bytes.len() <= 64 * 1024,
        "Windows staged settings exceed the private storage limit"
    );
    let result = (|| -> Result<()> {
        if ready_exists {
            fs::remove_file(ready)?;
        }
        for path in [&binary_temp, &config_temp, &ready_temp] {
            if managed_exists(path)? {
                fs::remove_file(path)?;
            }
        }
        write_secure_staging_file(&binary_temp, &binary_sddl(sid, false), sid, |output| {
            io::copy(&mut fs::File::open(source)?, output)?;
            Ok(())
        })
        .context("stage pending Windows desktop executable")?;
        write_secure_staging_file(&config_temp, &settings_sddl(sid, false), sid, |output| {
            output.write_all(&config_bytes)?;
            Ok(())
        })?;
        if binary_exists {
            fs::remove_file(pending_binary)?;
        }
        if config_exists {
            fs::remove_file(pending_config)?;
        }
        fs::rename(&binary_temp, pending_binary)?;
        fs::rename(&config_temp, pending_config)?;
        write_secure_staging_file(&ready_temp, &settings_sddl(sid, false), sid, |output| {
            output.write_all(b"ready")?;
            Ok(())
        })?;
        fs::rename(&ready_temp, ready)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(ready);
        for path in [&binary_temp, &config_temp, &ready_temp] {
            let _ = fs::remove_file(path);
        }
    }
    result
}

fn stop_service(service: &Service) -> Result<()> {
    let state = service.query_status()?.current_state;
    if state != ServiceState::Stopped {
        if state != ServiceState::StopPending {
            service.stop().context("stop Windows desktop supervisor")?;
        }
        let start = Instant::now();
        while service.query_status()?.current_state != ServiceState::Stopped {
            ensure!(
                start.elapsed() < STOP_WAIT + Duration::from_secs(5),
                "Windows service did not stop in time"
            );
            std::thread::sleep(Duration::from_millis(250));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn rollback_install(
    service: Option<&Service>,
    created: bool,
    prior_running: bool,
    changed_service: bool,
    previous_info: Option<&ServiceInfo>,
    swapped_binary: bool,
    executable: &Path,
    prior_binary: &Path,
    config: &Path,
    prior_config: &Path,
    had_config: bool,
    owner: &Path,
    prior_owner: &Path,
    had_owner: bool,
    sid: &str,
) -> Result<()> {
    if let Some(service) = service {
        stop_service(service)?;
        if created {
            service.delete()?;
        }
    }
    if prior_binary.exists() {
        if executable.exists() {
            fs::remove_file(executable)?;
        }
        fs::rename(prior_binary, executable)?;
    } else if swapped_binary && executable.exists() {
        fs::remove_file(executable)?;
    }
    restore_backup(config, prior_config, had_config, sid)?;
    restore_backup(owner, prior_owner, had_owner, sid)?;
    if changed_service {
        service
            .context("existing Windows service handle is missing")?
            .change_config(
                previous_info.context("previous Windows service configuration is missing")?,
            )?;
    }
    if prior_running
        && !created
        && let Some(service) = service
    {
        service.start(&[] as &[&OsStr])?;
    }
    Ok(())
}

fn restore_backup(path: &Path, backup: &Path, existed: bool, sid: &str) -> Result<()> {
    if existed {
        ensure!(
            backup.exists(),
            "Windows service rollback backup is missing: {}",
            backup.display()
        );
        if path.exists() {
            fs::remove_file(path)?;
        }
        fs::rename(backup, path)?;
        secure_path(path, sid, false)?;
    } else if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn remove_service(dir: &Path, bin_dir: &Path) -> Result<()> {
    let manager = service_manager(ServiceManagerAccess::CONNECT)?;
    let service = match manager.open_service(
        SERVICE_NAME,
        ServiceAccess::QUERY_STATUS
            | ServiceAccess::QUERY_CONFIG
            | ServiceAccess::STOP
            | ServiceAccess::DELETE,
    ) {
        Ok(service) => Some(service),
        Err(windows_service::Error::Winapi(error))
            if error.raw_os_error() == Some(ERROR_SERVICE_DOES_NOT_EXIST.0 as i32) =>
        {
            None
        }
        Err(error) => return Err(error).context("open Windows desktop supervisor"),
    };
    let dir_present = managed_exists(dir)?;
    let bin_dir_present = managed_exists(bin_dir)?;
    if !dir_present && !bin_dir_present && service.is_none() {
        return Ok(());
    }
    ensure!(
        dir_present && bin_dir_present,
        "Windows service directories are incomplete; refusing removal"
    );
    let sid = current_user_sid()?;
    validate_managed_path(dir, true, &sid)?;
    validate_managed_path(bin_dir, true, &sid)?;
    let owner = dir.join(OWNER_FILE);
    ensure!(
        managed_exists(&owner)?,
        "Windows service has no pinned owner; refusing removal"
    );
    validate_managed_path(&owner, false, &sid)?;
    ensure!(
        fs::read_to_string(&owner)? == sid,
        "Windows service belongs to a different console user; refusing removal"
    );
    let ready_present = managed_exists(&dir.join(PENDING_READY_FILE))?;
    for (directory, names) in [
        (
            dir,
            &[
                CONFIG_FILE,
                OWNER_FILE,
                PENDING_CONFIG_FILE,
                PENDING_READY_FILE,
                PENDING_CONFIG_TEMP_FILE,
                PENDING_READY_TEMP_FILE,
            ][..],
        ),
        (
            bin_dir,
            &[BINARY_FILE, PENDING_BINARY_FILE, PENDING_BINARY_TEMP_FILE][..],
        ),
    ] {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            ensure!(
                names
                    .iter()
                    .any(|name| entry.file_name() == OsStr::new(name)),
                "Windows service directory contains an unrelated entry: {}",
                entry.path().display()
            );
            if entry.file_name() == OsStr::new(PENDING_CONFIG_FILE) && !ready_present {
                validate_incomplete_pending_config(&entry.path(), &sid)?;
            } else {
                validate_managed_path(&entry.path(), false, &sid)?;
            }
        }
    }
    let executable = bin_dir.join(BINARY_FILE);
    let config = dir.join(CONFIG_FILE);
    if let Some(service) = &service {
        ensure!(
            managed_exists(&executable)? && managed_exists(&config)?,
            "Windows service is missing its managed executable or settings"
        );
        verify_existing_service(service, &executable)?;
    }
    if let Some(service) = &service {
        stop_service(service)?;
        service
            .delete()
            .context("delete Windows desktop supervisor")?;
    }
    for name in [
        PENDING_READY_FILE,
        PENDING_READY_TEMP_FILE,
        PENDING_CONFIG_FILE,
        PENDING_CONFIG_TEMP_FILE,
        CONFIG_FILE,
        OWNER_FILE,
    ] {
        let path = dir.join(name);
        if managed_exists(&path)? {
            fs::remove_file(path).context("remove Windows service settings")?;
        }
    }
    for name in [PENDING_BINARY_FILE, PENDING_BINARY_TEMP_FILE] {
        let path = bin_dir.join(name);
        if managed_exists(&path)? {
            fs::remove_file(path).context("remove staged Windows service executable")?;
        }
    }
    if managed_exists(&executable)? && fs::remove_file(&executable).is_err() {
        let wide = wide(executable.as_os_str());
        unsafe {
            MoveFileExW(
                PCWSTR(wide.as_ptr()),
                PCWSTR::null(),
                MOVEFILE_DELAY_UNTIL_REBOOT,
            )?;
        }
    }
    let _ = fs::remove_dir(dir);
    let _ = fs::remove_dir(bin_dir);
    Ok(())
}

define_windows_service!(service_main_ffi, service_main);

pub fn run_service() -> Result<()> {
    service_dispatcher::start(SERVICE_NAME, service_main_ffi).context("run Windows SCM dispatcher")
}

fn service_main(_arguments: Vec<OsString>) {
    if let Err(error) = supervise() {
        tracing::error!(%error, "Windows desktop supervisor failed");
    }
}

fn service_status(
    state: ServiceState,
    accepted: ServiceControlAccept,
    checkpoint: u32,
    code: ServiceExitCode,
) -> ServiceStatus {
    ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted: accepted,
        exit_code: code,
        checkpoint,
        wait_hint: Duration::from_secs(15),
        process_id: None,
    }
}

fn supervise() -> Result<()> {
    let (stop_tx, stop_rx) = mpsc::channel();
    let status = service_control_handler::register(SERVICE_NAME, move |control| match control {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            let _ = stop_tx.send(());
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    })
    .context("register Windows service control handler")?;
    status.set_service_status(service_status(
        ServiceState::StartPending,
        ServiceControlAccept::empty(),
        1,
        ServiceExitCode::Win32(0),
    ))?;
    let result = supervise_loop(&status, stop_rx);
    let code = if result.is_ok() {
        ServiceExitCode::Win32(0)
    } else {
        ServiceExitCode::ServiceSpecific(1)
    };
    status.set_service_status(service_status(
        ServiceState::Stopped,
        ServiceControlAccept::empty(),
        0,
        code,
    ))?;
    result
}

fn supervise_loop(
    status: &service_control_handler::ServiceStatusHandle,
    stop_rx: mpsc::Receiver<()>,
) -> Result<()> {
    let dir = service_dir()?;
    let owner_sid =
        fs::read_to_string(dir.join(OWNER_FILE)).context("read pinned Windows service user")?;
    ensure!(
        owner_sid.starts_with("S-1-") && owner_sid.len() < 256,
        "invalid pinned Windows service user"
    );
    let config = dir.join(CONFIG_FILE);
    ensure!(config.is_file(), "Windows service settings are missing");
    status.set_service_status(service_status(
        ServiceState::Running,
        ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
        0,
        ServiceExitCode::Win32(0),
    ))?;
    let mut child: Option<SessionChild> = None;
    let mut next_launch = Instant::now();
    loop {
        if stop_rx.recv_timeout(Duration::from_secs(1)).is_ok() {
            break;
        }
        let active = active_console_session_id();
        let ready = active.is_some_and(|id| console_session_ready(id).unwrap_or(false));
        if let Some(current) = &child {
            let alive = unsafe { WaitForSingleObject(current.process.raw(), 0) } == WAIT_TIMEOUT;
            let same_logon = active == Some(current.session)
                && session_details(current.session)
                    .is_ok_and(|details| details.logon_time == current.logon_time);
            let same_user =
                same_logon && console_user_sid(current.session).is_ok_and(|sid| sid == owner_sid);
            if !alive || !same_user {
                if let Some(mut old) = child.take() {
                    old.stop();
                }
                next_launch = Instant::now() + Duration::from_secs(2);
            }
        }
        if child.is_none() && ready && Instant::now() >= next_launch {
            let session = active.expect("ready session has an ID");
            match SessionChild::launch(session, &owner_sid, &config) {
                Ok(started) => child = Some(started),
                Err(error) => {
                    tracing::warn!(%error, "interactive Windows desktop host unavailable");
                    next_launch = Instant::now() + Duration::from_secs(10);
                }
            }
        }
    }
    status.set_service_status(service_status(
        ServiceState::StopPending,
        ServiceControlAccept::empty(),
        1,
        ServiceExitCode::Win32(0),
    ))?;
    if let Some(mut child) = child {
        child.stop();
    }
    Ok(())
}

struct SessionChild {
    session: u32,
    logon_time: i64,
    process: OwnedHandle,
    job: OwnedHandle,
    stop_event: OwnedHandle,
}

impl SessionChild {
    fn launch(session: u32, pinned_sid: &str, config: &Path) -> Result<Self> {
        let mut raw_token = HANDLE::default();
        unsafe {
            WTSQueryUserToken(session, &mut raw_token)
                .context("obtain console user's process token")?
        };
        let token = OwnedHandle::new(raw_token);
        ensure!(
            token_sid(token.raw())? == pinned_sid,
            "active console user differs from installed service owner"
        );
        let logon_time = session_details(session)?.logon_time;
        ensure!(logon_time > 0, "console user has no active logon");
        let event_name = format!(
            "Global\\LightOSRemoteDesktopStop-{:032x}",
            rand::random::<u128>()
        );
        let stop_event = secured_event(&event_name, pinned_sid)?;
        let executable = std::env::current_exe()?;
        let args = [
            executable.as_os_str().to_os_string(),
            OsString::from("service-host"),
            OsString::from("--config-file"),
            config.as_os_str().to_os_string(),
            OsString::from("--stop-event"),
            OsString::from(&event_name),
        ];
        let mut command = command_line(&args);
        let executable_wide = wide(executable.as_os_str());
        let desktop = wide(OsStr::new("winsta0\\default"));
        let startup = STARTUPINFOW {
            cb: size_of::<STARTUPINFOW>() as u32,
            lpDesktop: PWSTR(desktop.as_ptr() as *mut u16),
            ..Default::default()
        };
        let mut info = PROCESS_INFORMATION::default();
        let mut environment: *mut c_void = ptr::null_mut();
        unsafe {
            CreateEnvironmentBlock(&mut environment, Some(token.raw()), false)
                .context("build interactive user's environment")?
        };
        let create = unsafe {
            CreateProcessAsUserW(
                Some(token.raw()),
                PCWSTR(executable_wide.as_ptr()),
                Some(PWSTR(command.as_mut_ptr())),
                None,
                None,
                false,
                CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT,
                Some(environment.cast_const()),
                PCWSTR::null(),
                &startup,
                &mut info,
            )
        };
        let _ = unsafe { DestroyEnvironmentBlock(environment) };
        create.context("start host in active console user session")?;
        let process = OwnedHandle::new(info.hProcess);
        let thread = OwnedHandle::new(info.hThread);
        let job = match create_kill_on_close_job() {
            Ok(job) => job,
            Err(error) => {
                unsafe {
                    let _ = TerminateProcess(process.raw(), 1);
                }
                return Err(error);
            }
        };
        if let Err(error) = unsafe { AssignProcessToJobObject(job.raw(), process.raw()) } {
            unsafe {
                let _ = TerminateProcess(process.raw(), 1);
            }
            return Err(error).context("assign Windows host to private job");
        }
        ensure!(
            active_console_session_id() == Some(session)
                && console_session_ready(session)?
                && session_details(session)?.logon_time == logon_time
                && console_user_sid(session)? == pinned_sid,
            "console user changed before Windows host startup"
        );
        ensure!(
            unsafe { ResumeThread(thread.raw()) } != u32::MAX,
            "resume Windows host failed"
        );
        Ok(Self {
            session,
            logon_time,
            process,
            job,
            stop_event,
        })
    }

    fn stop(&mut self) {
        let _ = unsafe { SetEvent(self.stop_event.raw()) };
        if unsafe { WaitForSingleObject(self.process.raw(), STOP_WAIT.as_millis() as u32) }
            != WAIT_OBJECT_0
        {
            // The job contains only the child launched above and any codec children it creates.
            let _ = unsafe { TerminateJobObject(self.job.raw(), 1) };
            let _ = unsafe { WaitForSingleObject(self.process.raw(), 2_000) };
        }
    }
}

fn console_user_sid(session: u32) -> Result<String> {
    let mut raw_token = HANDLE::default();
    unsafe { WTSQueryUserToken(session, &mut raw_token)? };
    let token = OwnedHandle::new(raw_token);
    token_sid(token.raw())
}

fn create_kill_on_close_job() -> Result<OwnedHandle> {
    let job = OwnedHandle::new(unsafe { CreateJobObjectW(None, PCWSTR::null())? });
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    unsafe {
        SetInformationJobObject(
            job.raw(),
            JobObjectExtendedLimitInformation,
            (&raw const limits).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )?;
    }
    Ok(job)
}

fn secured_event(name: &str, sid: &str) -> Result<OwnedHandle> {
    let descriptor = SecurityDescriptor::from_sddl(&format!(
        "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;0x00100002;;;{sid})"
    ))?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0.0,
        bInheritHandle: BOOL(0),
    };
    let name = wide(OsStr::new(name));
    Ok(OwnedHandle::new(unsafe {
        CreateEventW(Some(&attributes), true, false, PCWSTR(name.as_ptr()))?
    }))
}

/// The native host awaits this named event after it has started its regular host loop.
pub async fn wait_for_stop_event(name: String) -> Result<()> {
    tokio::task::spawn_blocking(move || {
        ensure!(
            name.starts_with("Global\\LightOSRemoteDesktopStop-") && name.len() < 128,
            "invalid Windows service stop event name"
        );
        let name = wide(OsStr::new(&name));
        let event = OwnedHandle::new(unsafe {
            OpenEventW(
                SYNCHRONIZATION_ACCESS_RIGHTS(0x0010_0000),
                false,
                PCWSTR(name.as_ptr()),
            )?
        });
        match unsafe { WaitForSingleObject(event.raw(), u32::MAX) } {
            WAIT_OBJECT_0 => Ok(()),
            WAIT_FAILED => Err(windows::core::Error::from_thread())
                .context("wait for Windows service stop event"),
            _ => bail!("Windows service stop event wait was interrupted"),
        }
    })
    .await
    .context("join Windows service stop waiter")?
}

fn settings_sddl(sid: &str, directory: bool) -> String {
    if directory {
        format!("O:BAD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;GR;;;{sid})")
    } else {
        format!("O:BAD:P(A;;FA;;;SY)(A;;FA;;;BA)(A;;FR;;;{sid})")
    }
}

fn binary_sddl(sid: &str, directory: bool) -> String {
    if directory {
        format!("O:BAD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;GRGX;;;{sid})")
    } else {
        format!("O:BAD:P(A;;FA;;;SY)(A;;FA;;;BA)(A;;GRGX;;;{sid})")
    }
}

fn managed_exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            ensure!(
                metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 == 0,
                "Windows service path is a reparse point: {}",
                path.display()
            );
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => {
            Err(error).with_context(|| format!("inspect Windows service path {}", path.display()))
        }
    }
}

fn prepare_directory(path: &Path, sddl: &str, sid: &str) -> Result<()> {
    ensure!(
        sid.starts_with("S-1-") && sid.len() < 256,
        "invalid Windows user SID"
    );
    match fs::symlink_metadata(path) {
        Ok(_) => validate_managed_path(path, true, sid)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let descriptor = SecurityDescriptor::from_sddl(sddl)?;
            let attributes = SECURITY_ATTRIBUTES {
                nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor.0.0,
                bInheritHandle: BOOL(0),
            };
            let path = wide(path.as_os_str());
            unsafe { CreateDirectoryW(PCWSTR(path.as_ptr()), Some(&attributes))? };
        }
        Err(error) => return Err(error).context("inspect Windows service directory"),
    }
    apply_security(path, sddl, true, sid)
}

fn secure_path(path: &Path, sid: &str, directory: bool) -> Result<()> {
    ensure!(
        sid.starts_with("S-1-") && sid.len() < 256,
        "invalid Windows user SID"
    );
    apply_security(path, &settings_sddl(sid, directory), directory, sid)
}

fn secure_binary(path: &Path, sid: &str, directory: bool) -> Result<()> {
    ensure!(
        sid.starts_with("S-1-") && sid.len() < 256,
        "invalid Windows user SID"
    );
    apply_security(path, &binary_sddl(sid, directory), directory, sid)
}

fn sid_string(sid: PSID) -> Result<String> {
    let mut value = PWSTR::null();
    unsafe { ConvertSidToStringSidW(sid, &mut value)? };
    let result = unsafe { value.to_string() };
    unsafe { LocalFree(Some(HLOCAL(value.0.cast()))) };
    Ok(result?)
}

fn trusted_owner(sid: &str) -> bool {
    matches!(
        sid,
        "S-1-5-18"
            | "S-1-5-32-544"
            | "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464"
    )
}

fn validate_root_chain(managed_dir: &Path) -> Result<()> {
    let root = managed_dir
        .parent()
        .context("Windows managed directory has no known-folder root")?;
    for ancestor in root.ancestors() {
        if ancestor.as_os_str().is_empty() {
            break;
        }
        validate_ancestor(ancestor).with_context(|| {
            format!(
                "Windows install ancestor is replaceable or untrusted: {}",
                ancestor.display()
            )
        })?;
    }
    Ok(())
}

fn validate_ancestor(path: &Path) -> Result<()> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0 | FILE_FLAG_BACKUP_SEMANTICS.0)
        .open(path)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_dir() && metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 == 0,
        "Windows install ancestor is not a plain directory"
    );
    let mut owner = PSID::default();
    let mut dacl = ptr::null_mut();
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    unsafe {
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
    }
    let _descriptor = SecurityDescriptor(descriptor);
    ensure!(
        !owner.0.is_null() && !dacl.is_null(),
        "Windows install ancestor has no owner or DACL"
    );
    ensure!(
        trusted_owner(&sid_string(owner)?),
        "Windows install ancestor has an untrusted owner"
    );
    for index in 0..u32::from(unsafe { (*dacl).AceCount }) {
        let mut ace = ptr::null_mut();
        unsafe { GetAce(dacl, index, &mut ace)? };
        let header = unsafe { &*ace.cast::<ACE_HEADER>() };
        if header.AceFlags & 0x08 != 0 || header.AceType == 1 {
            continue;
        }
        ensure!(
            header.AceType == 0,
            "Windows install ancestor has an unsupported access entry"
        );
        let entry = unsafe { &*ace.cast::<ACCESS_ALLOWED_ACE>() };
        let ace_sid = sid_string(PSID(ptr::addr_of!(entry.SidStart).cast_mut().cast()))?;
        if trusted_owner(&ace_sid) || ace_sid == "S-1-3-4" {
            continue;
        }
        // Creation of new siblings is tolerable; deleting/renaming this ancestor or
        // an existing child, or changing its ACL, would replace our SCM image path.
        const REPLACEMENT_RIGHTS: u32 = 0x500d_0150;
        ensure!(
            entry.Mask & REPLACEMENT_RIGHTS == 0,
            "Windows install ancestor grants replacement rights to {ace_sid}"
        );
    }
    Ok(())
}

fn validate_managed_path(path: &Path, directory: bool, sid: &str) -> Result<()> {
    validate_managed_path_policy(path, directory, sid, false)
}

// Only host.pending.json without stage.ready may have the old storage writer's
// intermediate owner/user-write ACL. Its parent and SCM identity are checked first.
fn validate_incomplete_pending_config(path: &Path, sid: &str) -> Result<()> {
    validate_managed_path_policy(path, false, sid, true)
}

fn managed_file_owner_allowed(owner: &str, sid: &str, allow_incomplete_pending: bool) -> bool {
    matches!(owner, "S-1-5-18" | "S-1-5-32-544") || allow_incomplete_pending && owner == sid
}

fn managed_user_ace_allowed(mask: u32, allow_incomplete_pending: bool) -> bool {
    // The old private-storage writer grants full access until the service ACL is applied.
    // This exception is used only for host.pending.json without stage.ready.
    const MUTATING: u32 = 0x500d_0156;
    allow_incomplete_pending || mask & MUTATING == 0
}

fn validate_managed_path_policy(
    path: &Path,
    directory: bool,
    sid: &str,
    allow_incomplete_pending: bool,
) -> Result<()> {
    let flags = FILE_FLAG_OPEN_REPARSE_POINT.0
        | if directory {
            FILE_FLAG_BACKUP_SEMANTICS.0
        } else {
            0
        };
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(flags)
        .open(path)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 == 0
            && if directory {
                metadata.is_dir()
            } else {
                metadata.is_file()
            },
        "Windows service files must not use reparse points or mismatched types"
    );
    let mut owner = PSID::default();
    let mut dacl = ptr::null_mut();
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    unsafe {
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
    }
    let _descriptor = SecurityDescriptor(descriptor);
    ensure!(!owner.0.is_null(), "Windows service file has no owner");
    let owner = sid_string(owner)?;
    ensure!(
        managed_file_owner_allowed(&owner, sid, allow_incomplete_pending),
        "Windows service file has an unexpected owner"
    );
    ensure!(!dacl.is_null(), "Windows service file has a null DACL");
    let mut control = 0;
    let mut revision = 0;
    unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision)? };
    ensure!(
        control & SE_DACL_PROTECTED.0 != 0,
        "Windows service DACL is not protected"
    );
    let mut user_read = false;
    for index in 0..u32::from(unsafe { (*dacl).AceCount }) {
        let mut ace = ptr::null_mut();
        unsafe { GetAce(dacl, index, &mut ace)? };
        let header = unsafe { &*ace.cast::<ACE_HEADER>() };
        ensure!(
            header.AceType == 0,
            "Windows service DACL has an unsupported entry"
        );
        let entry = unsafe { &*ace.cast::<ACCESS_ALLOWED_ACE>() };
        let entry_sid = sid_string(PSID(ptr::addr_of!(entry.SidStart).cast_mut().cast()))?;
        if entry_sid == sid {
            ensure!(
                managed_user_ace_allowed(entry.Mask, allow_incomplete_pending),
                "Windows service owner has write access"
            );
            user_read |= entry.Mask & (0x8000_0000 | 0x1000_0000 | 1) != 0;
        } else {
            ensure!(
                entry_sid == "S-1-5-18" || entry_sid == "S-1-5-32-544",
                "Windows service DACL grants another account access"
            );
        }
    }
    ensure!(
        user_read,
        "installed console user cannot read Windows service file"
    );
    Ok(())
}

fn apply_security(path: &Path, sddl: &str, directory: bool, sid: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 == 0
            && if directory {
                metadata.is_dir()
            } else {
                metadata.is_file()
            },
        "Windows service files must not use reparse points or mismatched types"
    );
    let descriptor = SecurityDescriptor::from_sddl(sddl)?;
    let mut present = BOOL::default();
    let mut defaulted = BOOL::default();
    let mut dacl = ptr::null_mut();
    unsafe { GetSecurityDescriptorDacl(descriptor.0, &mut present, &mut dacl, &mut defaulted)? };
    let mut owner = PSID::default();
    unsafe { GetSecurityDescriptorOwner(descriptor.0, &mut owner, &mut defaulted)? };
    ensure!(
        present.as_bool() && !dacl.is_null() && !owner.0.is_null(),
        "invalid Windows service security descriptor"
    );
    let wide_path = wide(path.as_os_str());
    unsafe {
        SetNamedSecurityInfoW(
            PCWSTR(wide_path.as_ptr()),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION
                | DACL_SECURITY_INFORMATION
                | PROTECTED_DACL_SECURITY_INFORMATION,
            Some(owner),
            None,
            Some(dacl),
            None,
        )
        .ok()?;
    }
    validate_managed_path(path, directory, sid)
}

struct SecurityDescriptor(PSECURITY_DESCRIPTOR);
impl SecurityDescriptor {
    fn from_sddl(sddl: &str) -> Result<Self> {
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        let sddl = wide(OsStr::new(sddl));
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(sddl.as_ptr()),
                SDDL_REVISION_1,
                &mut descriptor,
                None,
            )?
        };
        Ok(Self(descriptor))
    }
}
impl Drop for SecurityDescriptor {
    fn drop(&mut self) {
        unsafe { LocalFree(Some(HLOCAL(self.0.0))) };
    }
}

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}

fn quote_windows_arg(arg: &OsStr) -> Vec<u16> {
    let chars: Vec<u16> = arg.encode_wide().collect();
    if !chars.is_empty()
        && !chars
            .iter()
            .any(|c| matches!(*c, 0x20 | 0x09 | 0x0a | 0x0b | 0x22))
    {
        return chars;
    }
    let mut result = vec![b'"' as u16];
    let mut slashes = 0;
    for ch in chars {
        if ch == b'\\' as u16 {
            slashes += 1;
            continue;
        }
        if ch == b'"' as u16 {
            result.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2 + 1));
        } else {
            result.extend(std::iter::repeat_n(b'\\' as u16, slashes));
        }
        result.push(ch);
        slashes = 0;
    }
    result.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
    result.push(b'"' as u16);
    result
}

fn command_line(args: &[OsString]) -> Vec<u16> {
    let mut result = Vec::new();
    for arg in args {
        if !result.is_empty() {
            result.push(b' ' as u16);
        }
        result.extend(quote_windows_arg(arg));
    }
    result.push(0);
    result
}

#[cfg(test)]
mod tests {
    use super::{
        PendingDisposition, managed_file_owner_allowed, managed_user_ace_allowed,
        pending_disposition, quote_windows_arg,
    };
    use std::{ffi::OsStr, os::windows::ffi::OsStringExt};

    #[test]
    fn interrupted_pending_config_is_discardable_only_without_ready_marker() {
        assert_eq!(
            pending_disposition(false, false, false).unwrap(),
            PendingDisposition::None
        );
        assert_eq!(
            pending_disposition(false, true, false).unwrap(),
            PendingDisposition::DiscardIncomplete
        );
        assert_eq!(
            pending_disposition(false, false, true).unwrap(),
            PendingDisposition::DiscardIncomplete
        );
        assert_eq!(
            pending_disposition(false, true, true).unwrap(),
            PendingDisposition::DiscardIncomplete
        );
        assert_eq!(
            pending_disposition(true, true, true).unwrap(),
            PendingDisposition::Activate
        );
        assert!(pending_disposition(true, true, false).is_err());
        assert!(pending_disposition(true, false, true).is_err());
    }

    #[test]
    fn incomplete_pending_acl_accepts_elevated_owner_without_weakening_live_files() {
        let sid = "S-1-5-21-1234";
        let full_access = 0x001f_01ff;
        for owner in [sid, "S-1-5-32-544", "S-1-5-18"] {
            assert!(managed_file_owner_allowed(owner, sid, true));
            assert!(managed_user_ace_allowed(full_access, true));
        }
        assert!(!managed_file_owner_allowed("S-1-5-21-9999", sid, true));
        assert!(!managed_file_owner_allowed(sid, sid, false));
        assert!(managed_file_owner_allowed("S-1-5-32-544", sid, false));
        assert!(!managed_user_ace_allowed(full_access, false));
        assert!(managed_user_ace_allowed(0x8000_0000, false));
    }

    #[test]
    fn command_quoting_preserves_paths_and_trailing_backslashes() {
        let quoted = quote_windows_arg(OsStr::new(r#"C:\Program Files\remote\"#));
        let text = std::ffi::OsString::from_wide(&quoted);
        assert_eq!(text.to_string_lossy(), r#""C:\Program Files\remote\\""#);
        let embedded = quote_windows_arg(OsStr::new("a\"b"));
        assert_eq!(
            std::ffi::OsString::from_wide(&embedded).to_string_lossy(),
            "\"a\\\"b\""
        );
    }
}
