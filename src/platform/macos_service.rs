//! Per-user LaunchAgent installer. The executable lives inside a stable app
//! bundle so macOS can associate TCC grants with a consistent bundle identity.
use std::{
    ffi::CString,
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    os::unix::{
        ffi::OsStrExt,
        fs::{DirBuilderExt, MetadataExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail, ensure};

use crate::install::{HostSettings, ServiceOptions};

const LABEL: &str = "org.lightos.remote-desktop";
const APP: &str = "LightOS Remote Desktop.app";
const EXECUTABLE: &str = "lightos-remoted";
const BUNDLE_ID: &str = "org.lightos.remote-desktop";

struct Layout {
    support: PathBuf,
    bundle: PathBuf,
    config: PathBuf,
    agent: PathBuf,
}
impl Layout {
    fn for_home(home: &Path) -> Self {
        let support = home.join("Library/Application Support/LightOS Remote Desktop");
        Self {
            bundle: support.join(APP),
            config: support.join("config/host.json"),
            agent: home
                .join("Library/LaunchAgents")
                .join(format!("{LABEL}.plist")),
            support,
        }
    }
    fn executable(&self) -> PathBuf {
        self.bundle.join("Contents/MacOS").join(EXECUTABLE)
    }
    fn pending(&self) -> PathBuf {
        self.support.join(".pending-update")
    }
}

pub fn install_service(options: ServiceOptions, rollback: bool) -> Result<()> {
    validate_flags(&options, rollback)?;
    let session = super::GuiSession::current()?;
    ensure!(
        session.active,
        "install-service requires the logged-in, unlocked macOS GUI user"
    );
    let home = dirs::home_dir().context("cannot locate macOS GUI user's home directory")?;
    let metadata = fs::symlink_metadata(&home)?;
    ensure!(
        metadata.is_dir() && metadata.uid() == session.uid && metadata.mode() & 0o022 == 0,
        "home directory must be owned by the GUI user and not group-writable"
    );
    let layout = Layout::for_home(&home);
    let domain = format!("gui/{}", session.uid);
    let service = format!("{domain}/{LABEL}");
    let pending = layout.pending();
    let has_pending = path_exists(&pending)?;
    let loaded_agent = launchctl_state(&service)?;
    let installed = existing_install(&layout)?;
    validate_managed_state(&layout, installed, has_pending, loaded_agent.as_deref())?;
    if has_pending {
        validate_pending(&layout)?;
    }
    if options.remove {
        if !installed {
            println!("LaunchAgent {LABEL} is not installed");
            return Ok(());
        }
        if options.dry_run {
            println!(
                "Would unload {service} and remove {}, {}, {}",
                layout.agent.display(),
                layout.bundle.display(),
                layout.config.display()
            );
            return Ok(());
        }
        if loaded_agent.is_some() {
            launchctl(&["bootout", &service])?;
        }
        remove_file_if_exists(&layout.agent)?;
        remove_bundle_if_exists(&layout.bundle)?;
        remove_file_if_exists(&layout.config)?;
        remove_bundle_if_exists(&layout.pending())?;
        println!("Removed LaunchAgent {LABEL}");
        return Ok(());
    }

    let fresh = if has_pending && !options.stage_only {
        None
    } else {
        let settings = HostSettings::from_environment().context(
            "set LIGHTOS_REMOTE_DESKTOP_AUTH_PASSWORD and other desired LIGHTOS_REMOTE_DESKTOP_* settings before install-service"
        )?;
        settings
            .config()
            .context("invalid installed host configuration")?;
        let source = std::env::current_exe().context("find running lightos-remoted executable")?;
        ensure!(source.is_file(), "running executable is not a regular file");
        Some((source, settings))
    };
    if options.dry_run {
        if let Some((source, _)) = &fresh {
            println!(
                "Would install {} from {}",
                layout.executable().display(),
                source.display()
            );
        } else {
            println!("Would activate staged update at {}", pending.display());
        }
        println!(
            "Would write private config {} and LaunchAgent {}",
            layout.config.display(),
            layout.agent.display()
        );
        println!("Would bootstrap {service}");
        return Ok(());
    }

    private_dir(&layout.support)?;
    private_dir(
        layout
            .config
            .parent()
            .context("config path has no parent")?,
    )?;
    launchagents_dir(
        layout
            .agent
            .parent()
            .context("LaunchAgents path has no parent")?,
    )?;
    let was_loaded = loaded_agent.is_some();
    let prepared = match &fresh {
        Some((source, settings)) => Prepared::fresh(&layout, source, settings)?,
        None => Prepared::pending(&layout)?,
    };
    if options.stage_only && installed {
        store_pending(&layout, &prepared)?;
        println!("Staged macOS update at {}", pending.display());
        return Ok(());
    }
    let mut replacements = prepared.replacements(&layout);
    let mut control = LaunchAgentControl {
        domain: &domain,
        service: &service,
        agent: path_text(&layout.agent)?,
    };
    install_transaction(
        &mut replacements,
        was_loaded,
        !options.stage_only,
        &mut control,
    )?;
    if options.stage_only {
        println!(
            "Staged macOS app and LaunchAgent at {}",
            layout.bundle.display()
        );
        return Ok(());
    }
    if has_pending && let Err(error) = remove_bundle_if_exists(&pending) {
        tracing::warn!(%error, "activated staged update but could not remove pending copy");
    }
    println!(
        "Installed and started LaunchAgent {LABEL}; grant Screen Recording and Accessibility to {APP} in System Settings if needed"
    );
    Ok(())
}

fn validate_flags(options: &ServiceOptions, rollback: bool) -> Result<()> {
    ensure!(
        !rollback,
        "macOS LaunchAgent installer does not support Linux rollback"
    );
    ensure!(
        !options.package_managed
            && !options.snapshot
            && !options.legacy
            && options.print_unit.is_none()
            && options.test_root.is_none(),
        "Linux package, unit, snapshot, legacy, and test-root options are unsupported on macOS"
    );
    ensure!(
        !(options.remove && options.stage_only),
        "--remove cannot be combined with --stage-only"
    );
    Ok(())
}

fn private_dir(path: &Path) -> Result<()> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)?;
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_dir()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0,
        "private service directory must be owned by the GUI user with mode 0700"
    );
    Ok(())
}

fn launchagents_dir(path: &Path) -> Result<()> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)?;
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_dir()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o022 == 0,
        "LaunchAgents directory must be owned by the GUI user and not writable by others"
    );
    Ok(())
}

fn write_bundle(bundle: &Path, source: &Path) -> Result<()> {
    let contents = bundle.join("Contents");
    let macos = contents.join("MacOS");
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o755)
        .create(&macos)?;
    let destination = macos.join(EXECUTABLE);
    fs::copy(source, &destination).context("copy host executable into app bundle")?;
    fs::set_permissions(&destination, fs::Permissions::from_mode(0o755))?;
    File::open(&destination)?.sync_all()?;
    atomic_file(&contents.join("Info.plist"), info_plist().as_bytes(), 0o644)?;
    sync_dir(&macos);
    sync_dir(&contents);
    Ok(())
}

fn path_exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn existing_install(layout: &Layout) -> Result<bool> {
    let paths = [
        (&layout.bundle, true),
        (&layout.config, false),
        (&layout.agent, false),
    ];
    let mut present = 0;
    for (path, directory) in paths {
        if !path_exists(path)? {
            continue;
        }
        owned_entry(path, directory, !directory)?;
        present += 1;
    }
    ensure!(
        present == 0 || present == 3,
        "existing macOS service files are incomplete"
    );
    if present == 0 {
        return Ok(false);
    }
    owned_entry(&layout.support, true, true)?;
    owned_entry(
        layout
            .config
            .parent()
            .context("config path has no parent")?,
        true,
        true,
    )?;
    owned_entry(
        layout.agent.parent().context("agent path has no parent")?,
        true,
        false,
    )?;
    validate_bundle(&layout.bundle)?;
    HostSettings::load(&layout.config)?.config()?;
    validate_agent_plist(&read_plist(&layout.agent)?, layout)?;
    Ok(true)
}

fn validate_managed_state(
    layout: &Layout,
    installed: bool,
    has_pending: bool,
    loaded_agent: Option<&str>,
) -> Result<()> {
    ensure!(
        !has_pending || installed,
        "staged macOS update exists without a complete installed service"
    );
    if let Some(loaded_agent) = loaded_agent {
        ensure!(
            installed,
            "loaded LaunchAgent is missing its managed app, settings, or plist"
        );
        validate_loaded_agent(loaded_agent, layout)?;
    }
    Ok(())
}

fn owned_entry(path: &Path, directory: bool, private: bool) -> Result<fs::Metadata> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.uid() == unsafe { libc::geteuid() }
            && if directory {
                metadata.is_dir()
            } else {
                metadata.is_file()
            }
            && metadata.mode() & (if private { 0o077 } else { 0o022 }) == 0,
        "managed service path has wrong owner, type, or mode: {}",
        path.display()
    );
    Ok(metadata)
}

fn read_plist(path: &Path) -> Result<serde_json::Value> {
    let output = run_bounded(
        "/usr/bin/plutil",
        &["-convert", "json", "-o", "-", path_text(path)?],
    )?;
    ensure!(
        output.status.success(),
        "invalid managed plist: {}",
        path.display()
    );
    serde_json::from_slice(&output.stdout).context("parse managed plist")
}

fn validate_agent_plist(plist: &serde_json::Value, layout: &Layout) -> Result<()> {
    let executable_path = layout.executable();
    let executable = path_text(&executable_path)?;
    let config = path_text(&layout.config)?;
    let expected = [executable, "service-host", "--config-file", config];
    let arguments = plist["ProgramArguments"]
        .as_array()
        .context("managed LaunchAgent has no ProgramArguments")?;
    ensure!(
        plist["Label"].as_str() == Some(LABEL)
            && arguments
                .iter()
                .map(serde_json::Value::as_str)
                .collect::<Vec<_>>()
                == expected.map(Some).to_vec()
            && plist
                .get("Program")
                .is_none_or(|program| program.as_str() == Some(executable)),
        "LaunchAgent label or program arguments do not match this installation"
    );
    Ok(())
}

fn validate_bundle(bundle: &Path) -> Result<()> {
    owned_entry(bundle, true, false)?;
    let contents = bundle.join("Contents");
    owned_entry(&contents, true, false)?;
    owned_entry(&contents.join("MacOS"), true, false)?;
    let executable = contents.join("MacOS").join(EXECUTABLE);
    let metadata = owned_entry(&executable, false, false)?;
    ensure!(
        metadata.len() > 0 && metadata.mode() & 0o111 != 0,
        "managed host executable is invalid"
    );
    let info = contents.join("Info.plist");
    owned_entry(&info, false, false)?;
    let plist = read_plist(&info)?;
    validate_bundle_plist(&plist)
}

fn validate_bundle_plist(plist: &serde_json::Value) -> Result<()> {
    ensure!(
        plist["CFBundleIdentifier"].as_str() == Some(BUNDLE_ID)
            && plist["CFBundleExecutable"].as_str() == Some(EXECUTABLE)
            && plist["CFBundlePackageType"].as_str() == Some("APPL"),
        "app bundle identity does not match this installation"
    );
    Ok(())
}

fn validate_pending(layout: &Layout) -> Result<()> {
    let pending = layout.pending();
    owned_entry(&pending, true, true)?;
    let ready = pending.join("ready");
    owned_entry(&ready, false, true)?;
    ensure!(
        fs::read(ready)? == b"ready",
        "staged macOS update is incomplete"
    );
    let config = pending.join("host.json");
    owned_entry(&config, false, true)?;
    HostSettings::load(&config)?.config()?;
    let agent = pending.join("agent.plist");
    owned_entry(&agent, false, true)?;
    validate_agent_plist(&read_plist(&agent)?, layout)?;
    validate_bundle(&pending.join(APP))?;
    Ok(())
}

fn validate_loaded_agent(print: &str, layout: &Layout) -> Result<()> {
    let lines: Vec<_> = print.lines().map(str::trim).collect();
    let executable_path = layout.executable();
    let executable = path_text(&executable_path)?;
    let expected = [
        executable,
        "service-host",
        "--config-file",
        path_text(&layout.config)?,
    ];
    let agent_path = format!("path = {}", path_text(&layout.agent)?);
    ensure!(
        lines.contains(&agent_path.as_str()),
        "loaded LaunchAgent comes from a foreign plist"
    );
    if let Some(program) = lines
        .iter()
        .find_map(|line| line.strip_prefix("program = "))
    {
        ensure!(
            program == executable,
            "loaded LaunchAgent uses a foreign program"
        );
    }
    let start = lines
        .iter()
        .position(|line| *line == "arguments = {")
        .context("loaded LaunchAgent has no arguments")?;
    let arguments: Vec<_> = lines[start + 1..]
        .iter()
        .take_while(|line| **line != "}")
        .copied()
        .collect();
    ensure!(
        arguments == expected,
        "loaded LaunchAgent has foreign program arguments"
    );
    Ok(())
}

struct Prepared {
    _bundle_dir: tempfile::TempDir,
    _config_dir: tempfile::TempDir,
    _agent_dir: tempfile::TempDir,
    bundle: PathBuf,
    config: PathBuf,
    agent: PathBuf,
}
impl Prepared {
    fn empty(layout: &Layout) -> Result<Self> {
        let bundle_dir = tempfile::Builder::new()
            .prefix(".lightos-app-")
            .permissions(fs::Permissions::from_mode(0o700))
            .tempdir_in(&layout.support)?;
        let config_dir = tempfile::Builder::new()
            .prefix(".lightos-config-")
            .permissions(fs::Permissions::from_mode(0o700))
            .tempdir_in(
                layout
                    .config
                    .parent()
                    .context("config path has no parent")?,
            )?;
        let agent_dir = tempfile::Builder::new()
            .prefix(".lightos-agent-")
            .permissions(fs::Permissions::from_mode(0o700))
            .tempdir_in(layout.agent.parent().context("agent path has no parent")?)?;
        let bundle = bundle_dir.path().join(APP);
        let config = config_dir.path().join("host.json");
        let agent = agent_dir.path().join(format!("{LABEL}.plist"));
        Ok(Self {
            _bundle_dir: bundle_dir,
            _config_dir: config_dir,
            _agent_dir: agent_dir,
            bundle,
            config,
            agent,
        })
    }
    fn fresh(layout: &Layout, source: &Path, settings: &HostSettings) -> Result<Self> {
        let staged = Self::empty(layout)?;
        write_bundle(&staged.bundle, source)?;
        settings.save(&staged.config)?;
        atomic_file(&staged.agent, agent_plist(layout)?.as_bytes(), 0o600)?;
        staged.validate()?;
        Ok(staged)
    }
    fn pending(layout: &Layout) -> Result<Self> {
        let pending = layout.pending();
        let metadata = fs::symlink_metadata(&pending)?;
        ensure!(
            metadata.is_dir()
                && metadata.uid() == unsafe { libc::geteuid() }
                && metadata.mode() & 0o077 == 0,
            "staged macOS update must be an owned private directory"
        );
        ensure!(
            fs::read(pending.join("ready"))? == b"ready",
            "staged macOS update is incomplete"
        );
        let settings = HostSettings::load(&pending.join("host.json"))?;
        settings.config()?;
        let manifest = fs::read(pending.join("agent.plist"))?;
        ensure!(
            manifest == agent_plist(layout)?.as_bytes(),
            "staged LaunchAgent plist differs from the managed layout"
        );
        let staged = Self::empty(layout)?;
        copy_bundle(&pending.join(APP), &staged.bundle)?;
        settings.save(&staged.config)?;
        atomic_file(&staged.agent, &manifest, 0o600)?;
        staged.validate()?;
        Ok(staged)
    }
    fn validate(&self) -> Result<()> {
        let executable = self.bundle.join("Contents/MacOS").join(EXECUTABLE);
        let metadata = fs::symlink_metadata(&executable)?;
        ensure!(
            metadata.is_file() && metadata.len() > 0 && metadata.mode() & 0o111 != 0,
            "staged host executable is missing or not executable"
        );
        HostSettings::load(&self.config)?.config()?;
        for path in [self.bundle.join("Contents/Info.plist"), self.agent.clone()] {
            let path = path_text(&path)?;
            let output = run_bounded("/usr/bin/plutil", &["-lint", path])?;
            ensure!(
                output.status.success(),
                "invalid staged plist {}: {}{}",
                path,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(())
    }
    fn replacements(&self, layout: &Layout) -> [Replacement; 3] {
        [
            Replacement::new(&self.bundle, &layout.bundle),
            Replacement::new(&self.config, &layout.config),
            Replacement::new(&self.agent, &layout.agent),
        ]
    }
}

fn copy_bundle(source: &Path, destination: &Path) -> Result<()> {
    let source_executable = source.join("Contents/MacOS").join(EXECUTABLE);
    let source_info = source.join("Contents/Info.plist");
    for source_file in [&source_executable, &source_info] {
        ensure!(
            fs::symlink_metadata(source_file)?.is_file(),
            "staged app contains a non-file path: {}",
            source_file.display()
        );
    }
    let contents = destination.join("Contents");
    let macos = contents.join("MacOS");
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o755)
        .create(&macos)?;
    let executable = macos.join(EXECUTABLE);
    fs::copy(source_executable, &executable)?;
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))?;
    File::open(&executable)?.sync_all()?;
    atomic_file(&contents.join("Info.plist"), &fs::read(source_info)?, 0o644)?;
    sync_dir(&macos);
    sync_dir(&contents);
    Ok(())
}

fn store_pending(layout: &Layout, prepared: &Prepared) -> Result<()> {
    let temporary = tempfile::Builder::new()
        .prefix(".lightos-pending-")
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir_in(&layout.support)?;
    copy_bundle(&prepared.bundle, &temporary.path().join(APP))?;
    let settings = HostSettings::load(&prepared.config)?;
    settings.save(&temporary.path().join("host.json"))?;
    atomic_file(
        &temporary.path().join("agent.plist"),
        &fs::read(&prepared.agent)?,
        0o600,
    )?;
    atomic_file(&temporary.path().join("ready"), b"ready", 0o600)?;
    let pending = layout.pending();
    if path_exists(&pending)? {
        let metadata = fs::symlink_metadata(&pending)?;
        ensure!(
            metadata.is_dir()
                && metadata.uid() == unsafe { libc::geteuid() }
                && metadata.mode() & 0o077 == 0,
            "existing staged update is not an owned private directory"
        );
        exchange_paths(temporary.path(), &pending)?;
        // The old pending update is now inside the temporary directory.
    } else {
        fs::rename(temporary.path(), &pending)?;
    }
    sync_dir(&layout.support);
    Ok(())
}

fn exchange_paths(first: &Path, second: &Path) -> Result<()> {
    let first = CString::new(first.as_os_str().as_bytes())?;
    let second = CString::new(second.as_os_str().as_bytes())?;
    let status = unsafe {
        renameatx_np(
            libc::AT_FDCWD,
            first.as_ptr(),
            libc::AT_FDCWD,
            second.as_ptr(),
            0x2,
        )
    };
    if status != 0 {
        return Err(std::io::Error::last_os_error()).context("atomically exchange managed paths");
    }
    Ok(())
}

struct Replacement {
    staged: PathBuf,
    installed: PathBuf,
    previous: bool,
    applied: bool,
}
impl Replacement {
    fn new(staged: &Path, installed: &Path) -> Self {
        Self {
            staged: staged.to_path_buf(),
            installed: installed.to_path_buf(),
            previous: false,
            applied: false,
        }
    }
    fn apply(&mut self) -> Result<()> {
        self.previous = path_exists(&self.installed)?;
        if self.previous {
            exchange_paths(&self.staged, &self.installed)?;
        } else {
            fs::rename(&self.staged, &self.installed)?;
        }
        self.applied = true;
        sync_dir(
            self.installed
                .parent()
                .context("managed path has no parent")?,
        );
        Ok(())
    }
    fn restore(&mut self) -> Result<()> {
        if !self.applied {
            return Ok(());
        }
        if self.previous {
            exchange_paths(&self.staged, &self.installed)?;
        } else {
            fs::rename(&self.installed, &self.staged)?;
        }
        self.applied = false;
        sync_dir(
            self.installed
                .parent()
                .context("managed path has no parent")?,
        );
        Ok(())
    }
}

trait AgentControl {
    fn loaded(&mut self) -> Result<bool>;
    fn bootout(&mut self) -> Result<()>;
    fn enable(&mut self) -> Result<()>;
    fn bootstrap(&mut self) -> Result<()>;
}

struct LaunchAgentControl<'a> {
    domain: &'a str,
    service: &'a str,
    agent: &'a str,
}
impl AgentControl for LaunchAgentControl<'_> {
    fn loaded(&mut self) -> Result<bool> {
        launchctl_loaded(self.service)
    }
    fn bootout(&mut self) -> Result<()> {
        launchctl(&["bootout", self.service])
    }
    fn enable(&mut self) -> Result<()> {
        launchctl(&["enable", self.service])
    }
    fn bootstrap(&mut self) -> Result<()> {
        launchctl(&["bootstrap", self.domain, self.agent])
    }
}

fn install_transaction(
    replacements: &mut [Replacement; 3],
    was_loaded: bool,
    activate: bool,
    control: &mut impl AgentControl,
) -> Result<()> {
    let attempt = (|| -> Result<()> {
        if was_loaded {
            control.bootout()?;
        }
        for replacement in replacements.iter_mut() {
            replacement.apply()?;
        }
        if activate {
            control.enable()?;
            control.bootstrap()?;
            ensure!(
                control.loaded()?,
                "LaunchAgent was not loaded after bootstrap"
            );
        }
        Ok(())
    })();
    let Err(error) = attempt else {
        return Ok(());
    };
    let mut recovery_errors = Vec::new();
    if replacements.iter().any(|replacement| replacement.applied) {
        match control.loaded() {
            Ok(true) => {
                if let Err(stop_error) = control.bootout() {
                    recovery_errors.push(format!("stop failed replacement: {stop_error:#}"));
                }
            }
            Ok(false) => {}
            Err(check_error) => {
                recovery_errors.push(format!("check failed replacement: {check_error:#}"))
            }
        }
    }
    for replacement in replacements.iter_mut().rev() {
        if let Err(restore_error) = replacement.restore() {
            recovery_errors.push(format!(
                "restore {}: {restore_error:#}",
                replacement.installed.display()
            ));
        }
    }
    if was_loaded && !replacements.iter().any(|replacement| replacement.applied) {
        match control.loaded() {
            Ok(true) => {}
            Ok(false) => {
                if let Err(restart_error) = control
                    .enable()
                    .and_then(|()| control.bootstrap())
                    .and_then(|()| {
                        ensure!(control.loaded()?, "previous LaunchAgent did not reload");
                        Ok(())
                    })
                {
                    recovery_errors
                        .push(format!("restart previous LaunchAgent: {restart_error:#}"));
                }
            }
            Err(check_error) => {
                recovery_errors.push(format!("check previous LaunchAgent: {check_error:#}"))
            }
        }
    }
    if recovery_errors.is_empty() {
        Err(error.context("macOS service update failed; previous files and loaded agent restored"))
    } else {
        bail!(
            "macOS service update failed: {error:#}; recovery failed: {}",
            recovery_errors.join("; ")
        )
    }
}

fn atomic_file(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    let directory = path.parent().context("file has no parent directory")?;
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    temporary
        .as_file()
        .set_permissions(fs::Permissions::from_mode(mode))?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|e| e.error)?;
    sync_dir(directory);
    Ok(())
}

fn sync_dir(directory: &Path) {
    if let Err(error) = File::open(directory).and_then(|file| file.sync_all()) {
        tracing::warn!(%error, path = %directory.display(), "directory sync failed after file update");
    }
}

fn remove_file_if_exists(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            ensure!(
                meta.is_file(),
                "refusing to remove non-file {}",
                path.display()
            );
            fs::remove_file(path)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}
fn remove_bundle_if_exists(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            ensure!(
                meta.is_dir(),
                "refusing to remove non-directory {}",
                path.display()
            );
            fs::remove_dir_all(path)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn path_text(path: &Path) -> Result<&str> {
    path.to_str()
        .context("LaunchAgent paths must be valid Unicode")
}

fn xml_escape(value: &str) -> Result<String> {
    ensure!(
        !value
            .chars()
            .any(|c| (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r')),
        "path contains invalid XML control characters"
    );
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            other => escaped.push(other),
        }
    }
    Ok(escaped)
}

fn agent_plist(layout: &Layout) -> Result<String> {
    let executable = xml_escape(path_text(&layout.executable())?)?;
    let config = xml_escape(path_text(&layout.config)?)?;
    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>{LABEL}</string>
<key>LimitLoadToSessionType</key><string>Aqua</string>
<key>ProgramArguments</key><array>
<string>{executable}</string><string>service-host</string>
<string>--config-file</string><string>{config}</string>
</array>
<key>RunAtLoad</key><true/>
<key>KeepAlive</key><true/>
</dict></plist>
"#
    ))
}

fn info_plist() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>{BUNDLE_ID}</string>
<key>CFBundleName</key><string>LightOS Remote Desktop</string>
<key>CFBundleDisplayName</key><string>LightOS Remote Desktop</string>
<key>CFBundleExecutable</key><string>{EXECUTABLE}</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleVersion</key><string>{version}</string>
<key>CFBundleShortVersionString</key><string>{version}</string>
<key>LSUIElement</key><true/>
</dict></plist>
"#,
        version = env!("CARGO_PKG_VERSION")
    )
}

fn launchctl_loaded(service: &str) -> Result<bool> {
    Ok(launchctl_state(service)?.is_some())
}

fn launchctl_state(service: &str) -> Result<Option<String>> {
    let output = run_launchctl(&["print", service])?;
    if output.status.success() {
        return Ok(Some(
            String::from_utf8(output.stdout).context("LaunchAgent state is not UTF-8")?,
        ));
    }
    let error = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if output.status.code() == Some(113) || error.contains("Could not find service") {
        return Ok(None);
    }
    bail!(
        "launchctl print {service} failed ({}): {error}",
        output.status
    )
}
fn launchctl(arguments: &[&str]) -> Result<()> {
    let output = run_launchctl(arguments)?;
    ensure!(
        output.status.success(),
        "launchctl {} failed ({}): {}{}",
        arguments.join(" "),
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

fn run_launchctl(arguments: &[&str]) -> Result<Output> {
    run_bounded("/bin/launchctl", arguments)
}

fn run_bounded(program: &str, arguments: &[&str]) -> Result<Output> {
    let mut stdout = tempfile::tempfile()?;
    let mut stderr = tempfile::tempfile()?;
    let mut child = Command::new(program)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout.try_clone()?))
        .stderr(Stdio::from(stderr.try_clone()?))
        .spawn()
        .with_context(|| format!("start {program} {}", arguments.join(" ")))?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("{program} {} timed out", arguments.join(" "));
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    let mut out = Vec::new();
    let mut err = Vec::new();
    stdout.seek(SeekFrom::Start(0))?;
    stderr.seek(SeekFrom::Start(0))?;
    stdout.take(16 * 1024).read_to_end(&mut out)?;
    stderr.take(16 * 1024).read_to_end(&mut err)?;
    Ok(Output {
        status,
        stdout: out,
        stderr: err,
    })
}

#[link(name = "System", kind = "dylib")]
unsafe extern "C" {
    fn renameatx_np(
        from_dir: i32,
        from: *const std::ffi::c_char,
        to_dir: i32,
        to: *const std::ffi::c_char,
        flags: u32,
    ) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;
    struct FakeAgent {
        loaded: bool,
        fail_bootstrap_once: bool,
        calls: Vec<&'static str>,
    }
    impl AgentControl for FakeAgent {
        fn loaded(&mut self) -> Result<bool> {
            self.calls.push("loaded");
            Ok(self.loaded)
        }
        fn bootout(&mut self) -> Result<()> {
            self.calls.push("bootout");
            self.loaded = false;
            Ok(())
        }
        fn enable(&mut self) -> Result<()> {
            self.calls.push("enable");
            Ok(())
        }
        fn bootstrap(&mut self) -> Result<()> {
            self.calls.push("bootstrap");
            if self.fail_bootstrap_once {
                self.fail_bootstrap_once = false;
                bail!("injected bootstrap failure");
            }
            self.loaded = true;
            Ok(())
        }
    }
    fn options() -> ServiceOptions {
        ServiceOptions {
            dry_run: false,
            stage_only: false,
            test_root: None,
            print_unit: None,
            package_managed: false,
            snapshot: false,
            remove: false,
            legacy: false,
        }
    }
    #[test]
    fn launchagent_uses_stable_bundle_identity_and_escaped_paths() {
        let layout = Layout::for_home(Path::new("/Users/A&B <Guest>"));
        let agent = agent_plist(&layout).unwrap();
        assert!(agent.contains("/Users/A&amp;B &lt;Guest&gt;"));
        assert!(agent.contains("<string>service-host</string>"));
        assert!(agent.contains("<string>--config-file</string>"));
        assert!(agent.contains("<key>LimitLoadToSessionType</key><string>Aqua</string>"));
        assert!(!agent.contains("--password"));
        let info = info_plist();
        assert!(info.contains("<string>org.lightos.remote-desktop</string>"));
        assert!(info.contains("<string>lightos-remoted</string>"));
    }
    #[test]
    fn managed_plist_identity_rejects_same_label_with_foreign_program_or_config() {
        let layout = Layout::for_home(Path::new("/Users/Agent Owner"));
        let executable_path = layout.executable();
        let executable = path_text(&executable_path).unwrap();
        let config = path_text(&layout.config).unwrap();
        let mut agent = serde_json::json!({
            "Label": LABEL,
            "ProgramArguments": [executable, "service-host", "--config-file", config]
        });
        assert!(validate_agent_plist(&agent, &layout).is_ok());
        agent["ProgramArguments"][0] = serde_json::json!("/tmp/foreign-host");
        assert!(validate_agent_plist(&agent, &layout).is_err());
        agent["ProgramArguments"][0] = serde_json::json!(executable);
        agent["ProgramArguments"][3] = serde_json::json!("/tmp/foreign-config");
        assert!(validate_agent_plist(&agent, &layout).is_err());

        let mut bundle = serde_json::json!({
            "CFBundleIdentifier": BUNDLE_ID,
            "CFBundleExecutable": EXECUTABLE,
            "CFBundlePackageType": "APPL"
        });
        assert!(validate_bundle_plist(&bundle).is_ok());
        bundle["CFBundleIdentifier"] = serde_json::json!("com.example.other");
        assert!(validate_bundle_plist(&bundle).is_err());
    }
    #[test]
    fn foreign_loaded_same_label_blocks_remove_and_update_preflight() {
        let layout = Layout::for_home(Path::new("/Users/Agent Owner"));
        let executable_path = layout.executable();
        let executable = path_text(&executable_path).unwrap();
        let config = path_text(&layout.config).unwrap();
        let own = format!(
            "gui/501/{LABEL} = {{\n path = {}\n program = {executable}\n arguments = {{\n  {executable}\n  service-host\n  --config-file\n  {config}\n }}\n}}",
            layout.agent.display()
        );
        assert!(validate_managed_state(&layout, true, false, Some(&own)).is_ok());
        assert!(validate_managed_state(&layout, false, false, None).is_ok());
        assert!(validate_managed_state(&layout, false, true, None).is_err());
        assert!(validate_managed_state(&layout, false, false, Some(&own)).is_err());
        let foreign_path = own.replace(
            &format!("path = {}", layout.agent.display()),
            "path = /Users/Agent Owner/Library/LaunchAgents/foreign.plist",
        );
        assert!(validate_managed_state(&layout, true, false, Some(&foreign_path)).is_err());
        let foreign_program = own.replace("  service-host\n", "  foreign-service\n");
        assert!(validate_managed_state(&layout, true, false, Some(&foreign_program)).is_err());
    }
    #[test]
    fn incomplete_fixed_path_collision_is_rejected() {
        let home = tempfile::tempdir().unwrap();
        let layout = Layout::for_home(home.path());
        fs::create_dir_all(&layout.bundle).unwrap();
        assert!(existing_install(&layout).is_err());
    }
    #[test]
    fn unsupported_linux_install_flags_fail() {
        assert!(validate_flags(&options(), true).is_err());
        let mut flags = options();
        flags.package_managed = true;
        assert!(validate_flags(&flags, false).is_err());
        flags = options();
        flags.print_unit = Some(crate::install::ServiceUnit::Transport);
        assert!(validate_flags(&flags, false).is_err());
        flags = options();
        flags.test_root = Some(PathBuf::from("/tmp/test"));
        assert!(validate_flags(&flags, false).is_err());
        flags = options();
        flags.remove = true;
        flags.stage_only = true;
        assert!(validate_flags(&flags, false).is_err());
    }
    #[test]
    fn failed_bootstrap_restores_all_previous_files_and_reloads_agent() {
        let root = tempfile::tempdir().unwrap();
        let mut replacements = std::array::from_fn(|index| {
            let installed = root.path().join(format!("installed-{index}"));
            let staged = root.path().join(format!("staged-{index}"));
            fs::write(&installed, format!("old-{index}")).unwrap();
            fs::write(&staged, format!("new-{index}")).unwrap();
            Replacement::new(&staged, &installed)
        });
        let mut agent = FakeAgent {
            loaded: true,
            fail_bootstrap_once: true,
            calls: Vec::new(),
        };
        let error = install_transaction(&mut replacements, true, true, &mut agent).unwrap_err();
        assert!(format!("{error:#}").contains("injected bootstrap failure"));
        assert!(agent.loaded);
        assert_eq!(
            agent
                .calls
                .iter()
                .filter(|call| **call == "bootstrap")
                .count(),
            2
        );
        for (index, replacement) in replacements.iter().enumerate() {
            assert_eq!(
                fs::read(&replacement.installed).unwrap(),
                format!("old-{index}").as_bytes()
            );
            assert_eq!(
                fs::read(&replacement.staged).unwrap(),
                format!("new-{index}").as_bytes()
            );
        }
    }
    #[test]
    fn failed_second_replacement_restores_first_and_reloads_agent() {
        let root = tempfile::tempdir().unwrap();
        let mut replacements = std::array::from_fn(|index| {
            let installed = root.path().join(format!("installed-{index}"));
            let staged = root.path().join(format!("staged-{index}"));
            fs::write(&installed, format!("old-{index}")).unwrap();
            if index != 1 {
                fs::write(&staged, format!("new-{index}")).unwrap();
            }
            Replacement::new(&staged, &installed)
        });
        let mut agent = FakeAgent {
            loaded: true,
            fail_bootstrap_once: false,
            calls: Vec::new(),
        };
        assert!(install_transaction(&mut replacements, true, true, &mut agent).is_err());
        assert!(agent.loaded);
        assert_eq!(
            agent
                .calls
                .iter()
                .filter(|call| **call == "bootstrap")
                .count(),
            1
        );
        for (index, replacement) in replacements.iter().enumerate() {
            assert_eq!(
                fs::read(&replacement.installed).unwrap(),
                format!("old-{index}").as_bytes()
            );
        }
    }
    #[test]
    fn stage_existing_update_keeps_active_files_unchanged() {
        let root = tempfile::tempdir().unwrap();
        let layout = Layout::for_home(root.path());
        private_dir(&layout.support).unwrap();
        private_dir(layout.config.parent().unwrap()).unwrap();
        launchagents_dir(layout.agent.parent().unwrap()).unwrap();
        let settings = HostSettings {
            arguments: vec!["--password".into(), "service-test-password".into()],
        };
        let prepared =
            Prepared::fresh(&layout, &std::env::current_exe().unwrap(), &settings).unwrap();
        copy_bundle(&prepared.bundle, &layout.bundle).unwrap();
        settings.save(&layout.config).unwrap();
        atomic_file(&layout.agent, b"old plist", 0o600).unwrap();
        fs::write(layout.executable(), b"old executable").unwrap();
        let old_config = fs::read(&layout.config).unwrap();
        store_pending(&layout, &prepared).unwrap();
        assert_eq!(fs::read(layout.executable()).unwrap(), b"old executable");
        assert_eq!(fs::read(&layout.config).unwrap(), old_config);
        assert_eq!(fs::read(&layout.agent).unwrap(), b"old plist");
        assert_eq!(fs::read(layout.pending().join("ready")).unwrap(), b"ready");
        assert_ne!(
            fs::read(
                layout
                    .pending()
                    .join(APP)
                    .join("Contents/MacOS")
                    .join(EXECUTABLE)
            )
            .unwrap(),
            b"old executable"
        );
    }
}
