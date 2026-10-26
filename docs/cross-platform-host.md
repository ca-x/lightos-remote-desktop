# Windows and macOS hosts / Windows 与 macOS 被控主机

Windows and macOS hosts share the browser transport, authentication, software
codecs and scheduling used on Linux. Their first implementation shares the
currently logged-in physical desktop. Native support is experimental until
interactive capture, input, audio and service acceptance has been completed on
each target system; CI builds and unit tests do not replace that acceptance.

Windows/macOS 复用 Linux 的浏览器传输、认证、软件编码和调度。首期共享当前登录
用户的物理桌面；在各系统完成真实采集、输入、音频和服务验收之前，标记为实验性。
原生 CI 的编译和单元测试不代表已完成交互桌面验收。

| Host / 主机 | Desktop / 桌面 | Foreground / 前台 | Service / 服务 |
| --- | --- | --- | --- |
| Linux | Existing virtual or attached physical desktop / 现有虚拟或物理桌面 | `serve` | Existing two systemd services / 保持现有两个 systemd 服务 |
| Windows | Logged-in console user's desktop / 当前控制台登录用户桌面 | `serve` | SCM supervisor starts an interactive user host / SCM 监管用户会话中的 host |
| macOS 13+ | Logged-in GUI user's desktop / 当前 GUI 登录用户桌面 | `serve` | User LaunchAgent / 用户 LaunchAgent |

## Windows

Extract the Windows release archive to a local directory. Windows Graphics
Capture support, the actual display, and the selected interactive session are
checked by the host; a version number alone does not establish availability.

解压 Windows 发布包。程序会检测 Windows Graphics Capture、显示器和实际交互会话；
不能仅凭系统版本判断采集可用。

```powershell
$env:LIGHTOS_REMOTE_DESKTOP_AUTH_PASSWORD = 'replace-with-a-unique-long-password'
.\lightos-remoted.exe doctor
.\lightos-remoted.exe serve
```

Run the installer in an elevated PowerShell belonging to the logged-in console
user. The installer creates the SCM service and stores its configuration with a
private ACL. The supervisor runs as LocalSystem, but capture, input, audio and
HTTP run in the pinned user's interactive session. Other logged-in users do not
inherit this user's credentials. SCM stop/update shuts down only the managed
host and its codec children.

在当前控制台登录用户的管理员 PowerShell 中安装。安装器创建 SCM 服务并用私有
ACL 保存配置。监管进程以 LocalSystem 运行，采集、输入、音频和 HTTP 则在绑定的
用户交互会话中运行。其他登录用户不会继承这份凭据。停止或更新仅关闭受监管的
host 及其编码子进程。

```powershell
.\lightos-remoted.exe install-service --dry-run
.\lightos-remoted.exe install-service
```

`--stage-only` registers a fresh service without starting it. For an existing
service it stages a protected binary/configuration pair and leaves the running
service untouched; run `install-service` without `--stage-only` to apply that
pair. `--remove` validates and removes this product's native service. Keep credentials out of command-line arguments; the installer
reads the existing `LIGHTOS_REMOTE_DESKTOP_*` environment settings into a private
configuration file.

`--stage-only` 首次安装时注册服务但不启动；已有服务时暂存受保护的二进制和配置，
保持运行中的服务，之后再次执行普通 `install-service` 才应用暂存版本。`--remove`
验证并删除本程序的原生服务。安装器从现有
`LIGHTOS_REMOTE_DESKTOP_*` 环境变量生成私密配置，服务进程命令行只携带配置路径。

## macOS

Use the executable inside the supplied application bundle. Foreground and
service execution use the same ScreenCaptureKit backend and bundle identity.

使用发布包 `.app` 内的执行程序。前台和服务模式共用 ScreenCaptureKit 后端及应用标识。

```bash
export LIGHTOS_REMOTE_DESKTOP_AUTH_PASSWORD='replace-with-a-unique-long-password'
"./LightOS Remote Desktop.app/Contents/MacOS/lightos-remoted" doctor
"./LightOS Remote Desktop.app/Contents/MacOS/lightos-remoted" serve
```

Grant Screen Recording and Accessibility in macOS System Settings to the
installed host identity. `doctor` and capability discovery only read permission
state; they do not open consent prompts or grant permissions. The OS can require
authorization again after unsigned/ad-hoc-signed updates. These packages are not
Developer ID signed or notarized.

请在 macOS 系统设置中为安装后的程序授权屏幕录制和辅助功能。`doctor` 与能力探测
只读取授权状态，不自动弹窗或授予权限。未使用 Developer ID 签名、公证的更新可能
需要重新授权；发布包不声称具备这类签名或公证。

```bash
"./LightOS Remote Desktop.app/Contents/MacOS/lightos-remoted" install-service --dry-run
"./LightOS Remote Desktop.app/Contents/MacOS/lightos-remoted" install-service
```

Run this as the logged-in GUI user, without `sudo`. The installer creates a user
LaunchAgent and private settings, then activates it. The stable app path is
`~/Library/Application Support/LightOS Remote Desktop/LightOS Remote Desktop.app`;
settings live alongside it at `config/host.json`, and the manifest is
`~/Library/LaunchAgents/org.lightos.remote-desktop.plist`.
For an existing service, `--stage-only` saves a private pending update and leaves
the active bundle, configuration and agent unchanged. A subsequent normal
`install-service` applies it. On a fresh installation, it writes the files without
activation. `--remove` unloads the agent. A root LaunchDaemon cannot
replace the user's GUI session or TCC authorization.

以当前 GUI 登录用户执行，无需 `sudo`。安装器创建用户 LaunchAgent、私密配置并
激活服务。程序固定安装到上述 Application Support 中的 `.app`，配置保存在同目录
`config/host.json`，服务定义在 `~/Library/LaunchAgents/` 中。
已有服务时，`--stage-only` 只写私密的待更新副本，不修改运行中的文件，之后普通
`install-service` 应用该副本；首次安装时仅写文件而不激活。`--remove` 卸载服务。root LaunchDaemon 无法
替代用户的 GUI 会话和 TCC 授权。

## Boundaries / 能力边界

- Lock or revoked permissions withhold video and release held input. A changed
  login identity ends that host; it does not attach to a different user's desktop.
  锁屏或撤销权限会暂停画面并释放输入；登录身份变化会结束当前 host。
- An unacknowledged input release bars new controllers and ends the host for
  supervised recovery; a timed-out native call cannot overlap the next controller.
  输入释放未获确认时，禁止新的控制端接管并结束 host，交由服务监管恢复。
- Native capture includes the cursor. Display selection and DPI conversion live
  in native adapters. Browser fit scales encoded pixels; it does not change the
  physical display's resolution.
  原生采集包含光标；显示器选择与 DPI 转换由平台适配器处理，浏览器适应窗口不修改
  物理显示器分辨率。
- A static desktop retains genuine cached pixels briefly, then the capture stream
  is reopened to obtain a fresh native snapshot. Queue timeouts never refresh
  screenshot timestamps or establish capture health.
  静止桌面短暂复用真实缓存后，会重新打开采集流取得新的原生画面；空队列或超时
  不会延长旧像素的有效时间。
- Independent virtual desktops, virtual display drivers, Windows UAC/logon
  secure desktops and remote login before a GUI user session are outside this
  implementation. Existing MCP/RDP physical-desktop control restrictions remain.
  本期不创建独立虚拟桌面、不安装虚拟显示驱动、不控制 Windows UAC/登录安全桌面，
  也不代替 GUI 登录。保留现有 MCP/RDP 对物理桌面控制的限制。

## Architecture and validation / 架构与验证

`HostPlatform` opens a `HostSession` containing `DesktopBackend`,
`DisplayController`, `SessionLifecycle` and `AudioCaptureFactory`.
`PermissionProvider`, `ServiceInstaller` and `PrivateStorage` isolate OS APIs.
Linux compositor/session discovery stays in its Linux adapter.

`HostPlatform` 返回包含桌面、显示器、生命周期、音频对象的 `HostSession`。
权限、服务安装、私密存储分别通过 trait 隔离系统 API，Linux 的 compositor 和会话
发现保留在 Linux 实现内。

Native dependencies include enigo/arboard, keycode, windows-capture,
windows-service, wasapi and ScreenCaptureKit bindings. Their notices accompany
release archives. RustDesk and uu-remote-linux inform architecture; GPL/AGPL
implementations are not copied into this MIT project.

原生实现复用上述成熟 Rust crate，并随发布包提供第三方声明。RustDesk、
uu-remote-linux 仅作为架构参考，不将 GPL/AGPL 实现复制到本 MIT 项目。
