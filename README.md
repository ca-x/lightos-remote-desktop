<p align="center">
  <img src="docs/assets/logo.png" width="160" height="160" alt="LightOS Remote Desktop logo" />
</p>

# LightOS Remote Desktop

简体中文 | [English](README.en.md)

面向 Linux / LightOS 的 Rust 原生浏览器远程桌面服务。运行时不内嵌 CPython，也不调用原版 Selkies 的 Python 启动器。

## 当前功能

- 内嵌 React / TypeScript 前端，使用 Radix UI 菜单、Headless UI 设置窗口和 Lucide React 图标。紧凑工具栏提供显示、键盘、剪贴板和连接信息菜单；独立的设置窗口管理分辨率、编码、外观、MCP 和 RDP；「设置 → 关于」显示版本、MIT 许可证及项目链接。界面支持中文、英文或跟随浏览器，以及浅色、深色或跟随系统主题。偏好保存在本地，修改时无需重连；主题只影响控制界面，不改变远程桌面的画面。
- 通过同源认证 WebSocket 传输画面，在 Canvas 中显示，并支持自动重连。
- 在「设置 → RDP」中启用或关闭 TLS RDP，并设置端口，与浏览器共用同一个虚拟桌面。默认关闭，监听地址默认是回环地址；远程连接可通过 SSH 转发，具体地址和 TLS 指纹会显示在设置中。没有指定证书时自动生成并保存私有 TLS 身份，也支持显式配置证书。启停或更换端口不重启 HTTP 服务和桌面 owner；RDP 画质由客户端能力和协议协商决定，不受浏览器编码或画质设置影响。参见 [RDP 配置与限制](docs/rdp.md)。
- 可选 MCP Streamable HTTP 接口，使用独立的观察或控制 token。在「设置 → MCP」中创建或撤销 token，通过截图、鼠标和键盘工具操作现有桌面。默认关闭，参见 [MCP 接入与 Cua 兼容性](docs/mcp.md)。
- 拖动悬浮工具栏手柄可将其停靠在屏幕任意边缘，位置保存在当前浏览器。菜单支持悬停切换、方向键导航和二级选项；全屏按钮位于工具栏右侧。点击手柄展开工具栏，也可聚焦手柄后用 Alt + 方向键停靠。菜单向屏幕内侧展开，在边缘自动避让，小窗口中可滚动；菜单和设置在全屏模式下仍可使用。
- 点击页面的全屏按钮后，会通过浏览器 Keyboard Lock API 请求捕获所有可用按键，不依赖应用专用快捷键映射。此功能需要浏览器支持和授权，通常还需要 HTTPS 等安全上下文；系统或合成器保留的快捷键仍可能无法到达页面。失去焦点、断开连接或退出全屏时，会释放捕获和按住的按键。Ctrl + Alt + Shift + Esc 可释放捕获并打开工具栏，浏览器长按 Escape 退出全屏的功能也保留。
  键盘菜单可组合任意受支持的物理按键与 Ctrl / Alt / Shift / Super，直接发送到远程机器，绕过本地快捷键拦截，无需修改宿主机绑定。窗口模式不会自动进入全屏；浏览器 F11 全屏不会启用 Keyboard Lock API。每次通过页面进入全屏都会重新尝试捕获，包含上级元素覆盖桌面的情况；授权失败后，点击全屏桌面可重试。捕获状态下 Ctrl + V 使用远程剪贴板，窗口模式下粘贴会传入本地文本。
  如果 Super 或 Alt + Tab 仍作用于本机 Omarchy / Hyprland，可用 `--ozone-platform=wayland` 让 Chrome / Chromium 原生运行在 Wayland 上，以便请求合成器的快捷键抑制协议。以顶层 HTTPS 或 localhost 页面打开桌面，并允许键盘权限。参见 [Chrome Keyboard Lock 要求](https://developer.chrome.com/docs/capabilities/web-apis/keyboard-lock)和 [Chromium Wayland 实现](https://chromium.googlesource.com/chromium/src/+/main/ui/ozone/platform/wayland/host/wayland_keyboard.cc)。
- 工具栏的连接信息菜单显示实际绘制 FPS、编码和分辨率，以及应用层 WebSocket RTT，包括最新值和最近 30 次响应的 P95。RTT 包含排队时间，不等于输入到画面的延迟。静态桌面会跳过重复帧，空闲时 FPS 较低属于正常现象；测量使用浏览器单调时钟。
- 通过 `DesktopBackend` trait 统一采集、键鼠输入和文本剪贴板，并在可信目标会话上自动选择原生 Wayland、Portal 或 X11。原生 Wayland 使用 libwayshot 与虚拟输入协议；GNOME / KDE 在能核实唯一 Portal 实现及目标 compositor 身份时，可使用共享 RemoteDesktop / ScreenCast 授权与 PipeWire；X11 使用 x11rb、enigo 和 arboard，并隔离目标 display / authority。发现阶段不弹授权窗口，实际连接才请求 Portal 授权；拒绝后不会切换路径绕过授权。X11 已有独立 Xvfb 验收；Portal 尚未在 GNOME / KDE 真机和分数缩放环境验收，限制见[运行平台说明](docs/runtime-platforms.md)。
- 独立光标通过单独连接同步远端真实形状、热点、缩放与隐藏状态，本地鼠标移动无需等待视频。输入框、链接、窗口边缘、拖拽及应用自定义光标均沿用远端图像；不支持独立通道时回退到画面内光标。
- 在隔离的 Cage / wlroots 环境中运行嵌套 Hyprland，使用私有 D-Bus，并清理所拥有的进程组，不修改 UWSM 源码。
- 保存编码选择（自动 / H.264 / JPEG）和传输调度策略（自动 / 低延迟 / 流畅 / 清晰优先）。显示控制、媒体编码和传输调度已拆成独立 crate，桌面后端、实际能力探测和平台限制参见[运行平台说明](docs/runtime-platforms.md)。
- 适应窗口和全屏时，按可见区域的设备像素自动缩小编码画面，使用 FIR SIMD 并复用缓冲；清晰优先和原始大小保持原生尺寸。编码缩放独立于桌面分辨率、输入与光标，参见[编码前缩放与实测](docs/encoder-scaling.md)。
- 支持动态分辨率、文本剪贴板，以及可选的私有 PipeWire / Pulse 音频 sink 和浏览器音频播放。
- 音频使用独立 WebSocket 和发送任务，不受视频编码或画面 ACK 阻塞，生命周期与限制参见[音频通道说明](docs/audio-channel.md)。
- 桌面启动的应用会收到实际的 Xwayland 显示环境；D-Bus 激活只在私有总线上使用合成器最终环境，不会将远程环境导入宿主用户的 systemd manager。
- 提供受监督的健康检查、就绪检查，以及现有 `lightos-remote-desktop.service` 的事务迁移和回退。

**当前原生实现尚未覆盖 Selkies 的全部功能。** 视频优先通过进程内 OpenH264 与浏览器 WebCodecs 协商 H.264；WebCodecs 或编码不可用时回退到 SIMD JPEG。WebCodecs 要求 HTTPS 或 localhost 等安全上下文，H.264 要求宽高为偶数，默认截图上限为 30 FPS。软件 H.264 已内置，可选原生 worker 可探测并使用 VA-API / NVENC / AMF / VideoToolbox 硬件 H.264，失败时回退到软件编码。截图仍使用 CPU 缓冲区，尚无 DMA-BUF 零拷贝。音频使用双声道 Opus，并保留 PCM 回退。麦克风上行、图片剪贴板、文件传输、手柄和多显示器控制尚未实现，带宽和延迟需要在实际部署环境中测量。

截图连接最多复用 8 帧，或在两次截图间隔达到 250 ms 后刷新，以减少 AF_UNIX 和 Wayland 初始化成本，同时限制 libwayshot 0.9 的协议对象保留并更新输出信息。只有请求 JPEG 时才生成 JPEG；H.264 worker 共用截图缓冲区。画面不变时不会重复编码或传输，客户端对已绘制帧发送 ACK，输入处理独立于视频写入。带序号的客户端使用由绘制 ACK 驱动的 1 至 3 帧窗口，切换编码或创建 H.264 编码器后，先等首帧完成绘制再扩展窗口。

H.264 只在有发送额度时读取最新原始帧，不会丢弃已编码的预测帧。重连或分辨率变化会重建编码器并生成关键帧。自动、低延迟、流畅和清晰优先策略通过有界 ACK 反馈调整发送节奏及受支持编码器的码率。GPU 截图和独立的远程光标形状流尚未实现。

## AUR 安装与更新

包名为 `lightos-remoted-bin`，命令为 `lightos-remoted`，可执行文件安装到 `/usr/lib/lightos-remoted/lightos-remoted`。GitHub 二进制压缩包名为 `lightos-remoted-linux-x86_64.tar.gz`。可选 AUR 包 `lightos-codec-ffmpeg-bin` 提供原生 FFmpeg 编码 worker，可探测硬件 H.264 编码。它在 Arch 构建环境中从同版本源码预编译，链接对应 ABI 的系统 FFmpeg，安装到 `/usr/lib/lightos-remoted/lightos-codec-ffmpeg`，主程序会自动发现。无需安装该包即可使用内置软件 H.264 和 JPEG；硬件编码是否可用仍取决于 GPU、驱动和 FFmpeg 能力。

```sh
yay -S lightos-remoted-bin
yay -Syu
```

需要可选硬件编码时，安装扩展包：

```sh
yay -S lightos-codec-ffmpeg-bin
```

全新安装会自动初始化认证并启动服务，无需手动安装 unit 或执行 `enable`。配置和认证信息保存在 `/etc/lightos/remote-desktop.env`，已有值会保留。LightOS 平台脚本依赖原有服务名，因此继续使用 `lightos-remote-desktop.service` 和 `lightos-remote-desktop-session.service`。服务定义和初始化逻辑以二进制内置版本为准：AUR 打包时从二进制导出两个 unit，安装钩子调用程序完成激活，文件由 pacman 管理。常规包升级只重启 HTTP 传输服务，保留常驻桌面及其应用。参见 [AUR 服务与打包说明](docs/aur.md)。

### 老用户迁移与清理（旧版手动安装）

AUR 检测到旧版手动安装时，会保留当前服务和桌面，不自动切换。以下是你决定迁移时手动执行的步骤，普通 AUR 新安装不需要这些操作。

**先保存桌面应用中的工作。停止 `lightos-remote-desktop-session.service` 会结束它管理的桌面应用，包括终端和 v2rayN。**

1. 备份旧配置、unit 和回滚记录。备份目录仅 root 可读，不要把其中的认证信息贴到日志或 issue。

   ```sh
   backup="/root/lightos-remote-desktop-backup-$(date +%Y%m%d-%H%M%S)"
   sudo install -d -m 0700 "$backup"
   for path in /etc/lightos \
     /etc/systemd/system/lightos-remote-desktop.service \
     /etc/systemd/system/lightos-remote-desktop.service.d \
     /etc/systemd/system/lightos-remote-desktop-session.service \
     /var/lib/lightos-remote-desktop; do
     if sudo test -e "$path"; then
       sudo cp -a --parents "$path" "$backup/"
     fi
   done
   ```

   如使用自定义的 MCP 设置文件、证书或其他状态目录，也先单独备份。

2. 保存工作后停止旧桌面，再移除旧的本机 unit 覆盖，让 AUR 提供的同名 unit 生效。

   ```sh
   sudo systemctl disable --now lightos-remote-desktop.service
   sudo systemctl stop lightos-remote-desktop-session.service
   sudo rm -f -- /etc/systemd/system/lightos-remote-desktop.service \
     /etc/systemd/system/lightos-remote-desktop-session.service
   sudo rm -rf -- /etc/systemd/system/lightos-remote-desktop.service.d
   sudo systemctl daemon-reload
   sudo lightos-remoted install-service --package-managed
   ```

   旧的 `90-native.conf`、`95-session.conf` 随上述旧 drop-in 目录一起移除；AUR 的完整 unit 已包含程序路径和 session 依赖，无需重新安装这两个文件。pacman 不会自动删除这些旧手动安装的文件。若目录内还有自定义配置，先备份并逐项迁移。

   `/etc/lightos/remote-desktop.env` 要保留：LightOS 仍通过它管理认证和开关。新服务沿用这个文件中有效的非 root 用户和认证配置。

3. 登录浏览器确认画面、输入和应用启动正常，并检查服务。

   ```sh
   lightos-remoted --version
   systemctl status lightos-remote-desktop.service --no-pager
   systemctl status lightos-remote-desktop-session.service --no-pager
   ```

4. 确认新版本正常、无需回退后，才清理旧 Rust 安装目录。先确认服务已使用 `/usr/lib/lightos-remoted/lightos-remoted`，并检查保留的 `/etc/lightos/remote-desktop-native.env`：若它仍引用旧安装目录中的兼容库，先按 [AUR 文档](docs/aur.md)切换到包提供的兼容库并验证重启正常，再执行删除。

   ```sh
   sudo rm -rf -- /usr/local/lib/lightos-remote-desktop
   sudo rm -rf -- /var/lib/lightos-remote-desktop
   ```

   **不要删除** `/etc/lightos`、`/opt/lightos/remote-desktop` 或 `/usr/local/bin/lightos-remote-desktop-*.sh`：它们仍属于 LightOS 平台集成。备份目录可在确认不再需要回退后自行清理。

## 构建与测试

从源码构建时需要 Node.js 24.15+（24 LTS）或 26+。先构建锁定版本的前端，再编译 Rust，程序会内嵌 HTML、JavaScript、CSS 和 gzip 资源：

```sh
npm ci
npm run build
npm test
cargo test --locked --features bundled-jpeg,rdp,portal
cargo build --release --locked --features bundled-jpeg,rdp,portal
```

安装后的程序不需要 Node.js，也不需要单独的前端目录或服务器。`npm run dev` 只预览界面，认证桌面和 WebSocket 集成应使用 Rust 服务验证。资源使用相对 URL，兼容已有路径前缀。

工具栏的桌面来源选择器可以在常驻虚拟桌面与同一用户已有的 Wayland 桌面之间切换。物理模式列出活动输出，只捕获并控制选定显示器，不改变它的分辨率。没有物理会话时，该选项显示为不可用，不会创建新的物理登录；暂不支持物理桌面的音频。发现方式、非标准 socket 配置与协议要求参见[桌面切换说明](docs/desktop-switching.md)。

GitHub Actions 先运行 `npm ci`、前端类型检查、构建和测试，再运行 Rust 与安装器检查。测试和发布构建任务都会先构建前端，再编译 Rust，随后生成 Linux x86_64 压缩包与 SHA256 校验文件，也可通过 `workflow_dispatch` 手动触发。推送 `v*` tag 会将产物发布到 GitHub Releases。发布构建使用 Ubuntu 22.04，静态内置 SIMD JPEG 库；Wayland、xkbcommon 和 glibc 仍由系统提供。

### 安装下载的可执行文件

```sh
./lightos-remoted install-service --dry-run
sudo ./lightos-remoted install-service
sudo ./lightos-remoted rollback-service
```

`install-service` 默认按新约定安装程序、命令入口和两个完整 systemd unit，自动初始化非 root 账户与认证，再启用服务。配置仍使用 `/etc/lightos/remote-desktop.env`，已有值会保留。无需已有的 Python service，也无需源码目录；桌面依赖和所需的 Aquamarine 兼容库仍须预先安装，程序不下载软件包。

`--dry-run` 只显示安装计划，`--stage-only` 只部署文件，不创建账户、认证或启动服务。常规更新只重启传输服务，保留桌面 owner。`rollback-service` 恢复上一次独立安装事务，保留认证；若回退首次安装会移除服务文件，须先保存工作并主动停止两个服务，再用 `--stage-only` 回退。AUR 已管理的文件不会被独立安装器覆盖，请通过包管理器升级。

检测到旧版或自定义的 `/etc/systemd/system` unit 时，不会直接覆盖或停止旧桌面，应按上文迁移。若要继续使用旧路径的事务迁移流程，可显式指定 `install-service --legacy` 和 `rollback-service --legacy`。

### 桌面操作

触屏设备有与 WebShell 尺寸对齐的两行快捷键栏，左侧显示键盘开关及返回 LightOS 管理页的主页图标。快捷栏固定在可见页面底部，画面在上方剩余区域按比例显示；键盘打开时底栏移动到键盘上方，并适配懒猫客户端沉浸式与自动安全区。支持点击、拖动、长按右键和双指滚动，参见[移动端输入](docs/mobile-input.md)。

桌面占满页面，右侧图标打开悬浮工具栏，通过图标提示和焦点标签识别操作。显示模式包括适应窗口、按配置分辨率以 1:1 显示并滚动，以及浏览器全屏。全屏时保留工具栏，Escape 恢复此前的窗口模式。默认跟随浏览器分辨率，在 250 ms 防抖后按有界、偶数的 CSS 像素尺寸调整桌面。手动指定分辨率或切换到 1:1 模式会关闭跟随；关闭后，只改变显示缩放不会调整远程分辨率。剪贴板与分辨率输入位于相应菜单或设置中。

请使用当前稳定版 Rust。原生链接需要 Wayland、libxkbcommon、libpulse、libturbojpeg 3+，以及用于从源码构建 OpenH264 和 libopus 的 C / C++ 工具链与 NASM。`bundled-jpeg` feature 可静态内置 libjpeg-turbo，需要 CMake 和 NASM。正式二进制启用 `bundled-jpeg,rdp,portal`；`portal` 编译还需要 PipeWire / SPA 开发包、pkg-config 和 libclang，运行时动态链接 `libpipewire-0.3.so.0`。不启用此 feature 的自编译版本会明确报告 Portal 不可用。

Ubuntu 22.04 的发布构建使用带 SHA-256 校验的 SPA 1.0.5 头文件，放在私有目录中，仍链接发行版自带的 PipeWire 库。系统自带的 SPA 0.3.48 头文件无法编译当前 Rust 绑定。参见[构建辅助脚本](.github/scripts/prepare-spa-build-sdk.sh)，通过导出的 `PKG_CONFIG_PATH` 选择构建头文件，无需更换运行库；发布包包含 SDK 来源、校验值和版权声明。

owned 虚拟桌面需要 Cage、Hyprland 和 D-Bus；附着现有桌面不要求启动这些组件。原生 Wayland 文本剪贴板使用可选的 wl-clipboard；X11 需要 xauth 和可用的 XTest。私有音频需要 libpulse、pactl、pipewire、pipewire-pulse 和 WirePlumber 0.5+。`doctor` 按实际启动模式区分必需和可选依赖，安装了某个命令不代表采集和输入已可用。

每个桌面创建私有音频图，不修改宿主 PulseAudio cookie 或默认 sink；WirePlumber 配置禁用物理音频、蓝牙和视频监视器。视频编码与音频捕获、编码不启动 FFmpeg 子进程。音频使用可取消的原生 Pulse 流，兼容浏览器协商为 128 kbit/s Opus，在 Worker 中解码，再通过 AudioWorklet 播放，PCM 作为兼容路径。原生四包队列将积压限制为 PCM 40 ms 或 Opus 80 ms，浏览器传输与播放队列也有界。悬停 FPS / RTT 可查看视频阶段 P95 和音频缓冲诊断，协议、回退与测量限制参见[音频通道说明](docs/audio-channel.md)。当前独立桌面后端需要访问 `/dev/dri/renderD128` 以进行 GLES 渲染。

```sh
npm ci
npm run build
cargo build --release --locked
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
bash tests/installer.sh
bash scripts/test-install-compat.sh
npm test
```

10 秒启动超时测试需显式运行：

```sh
cargo test --locked unresponsive_compositor -- --ignored
```

## 运行独立候选版本

私下设置 `LIGHTOS_REMOTE_DESKTOP_AUTH_PASSWORD`，使用至少 12 字节、非占位的密码。不要提交环境文件，也不要通过命令行参数传入生产凭据。

```sh
target/release/lightos-remoted doctor
target/release/lightos-remoted serve --listen 127.0.0.1 --port 18766
```

连接已有合成器时，使用 `--wayland-socket /absolute/socket`，目标必须提供所需的捕获、seat 和虚拟输入协议。附着模式不拥有该桌面，也不能调整其分辨率。

默认的 `--desktop-shell auto` 会独立于发行版检测桌面环境。在 Omarchy 4.x 中，优先加载用户的 `~/.config/hypr/hyprland.lua`，用户没有该文件时使用系统已安装的 Lua 入口；只有找不到 Lua 入口时才尝试原生 `.conf` 配置。Lua 校验错误会直接报告，不会切换到其他配置方案。快捷键、主题和窗口规则来自原生配置，不依赖仓库内的配置子集。生成的私有入口跳过标准物理显示器、自动启动和 NVIDIA 模块，隔离运行目录、状态及音频，并在加载原生配置后恢复会话自己的 PATH。不会重写宿主配置；原生 Lua 中直接嵌入的自定义命令仍是由用户控制的代码。

`--desktop-shell minimal` 使用通用终端桌面，`--desktop-shell omarchy` 则要求 Omarchy。通用方案检查已安装的终端启动器，缺少前置条件时会报告。显式指定 `--desktop-config FILE` 或附着到 Wayland socket 会绕过自动 shell 启动。这是独立桌面，但同一用户共用主题与应用数据。远程应用通过私有的 `uwsm-app -- command [args...]` 适配器与浏览器启动器运行，不会将远程环境导入宿主用户的 systemd manager。

安装后的服务将常驻桌面 `lightos-remote-desktop-session.service`（Hyprland、Shell、D-Bus、Xwayland 和音频）与传输服务 `lightos-remote-desktop.service`（HTTP、截图和输入）分开。更新或重启传输服务时会重新连接私有、带版本号的会话描述文件，保留 GUI 窗口。首次从旧的合并式服务升级时，桌面需要重建一次；显式重启 session 服务、合成器崩溃、注销或重启系统仍会关闭应用。参见[安装与会话生命周期](docs/install.md#persistent-desktop-session)。

### Aquamarine 兼容性

已测试的 LightOS 镜像使用 Aquamarine 0.15.1，其嵌套 Wayland 后端存在协议版本绑定过高、初始输出提交未 flush 的问题。固定版本、可审查的私有构建修复这些问题，不替换 `/usr/lib/libaquamarine.so.14`：

```sh
bash scripts/build-aquamarine.sh
target/release/lightos-remoted serve --port 18766 \
  --aquamarine-dir "$PWD/.local/aquamarine/build"
```

构建脚本会拒绝其他已安装的 Aquamarine 版本，系统更新后应重新验证兼容性，不要绕过检查。构建工具包括 CMake、Ninja 和匹配的原生开发依赖。

## 安装与回退

修改运行中的服务前，请先阅读[安装说明](docs/install.md)。安装器保留 `/etc/lightos/remote-desktop.env`、原始 unit 和 Python 平台文件，原生专用设置放在独立、可选的环境文件中。旧迁移安装器要求已有服务，不负责发行版初始化，不会安装软件包或下载可执行依赖。

```sh
sudo bash scripts/install-compat.sh --check
sudo bash scripts/install-compat.sh
sudo bash scripts/install.sh --check
sudo bash scripts/install.sh --activate
sudo bash scripts/install.sh --rollback --activate
```

先在独立端口验证真实浏览器的画面、输入和所需功能，再激活版本。激活检查要求原生健康响应匹配 systemd MainPID，并检查重启计数稳定；失败会恢复之前的版本。

## 配置

运行 `serve --help` 查看准确的命令行和环境变量接口。已有认证、端口、开关、画面和 URL 子目录变量会被读取，不执行其中的 shell 代码。`LIGHTOS_REMOTE_DESKTOP_SESSION` 和 `SESSION_ID` 不是任意 shell 执行入口，本后端明确启动 Hyprland。

`GET /healthz` 公开返回就绪状态、PID 和传输信息，不包含凭据。桌面、静态资源和 WebSocket 路由均要求 Basic Auth。在可信网关使用 HTTPS / WSS，原始 HTTP 用于 localhost 或受保护网络。代理应保留 Host 和配置的 URL 前缀，不接受跨源 WebSocket 连接。

## 来源与依赖

实现使用 libwayshot、Wayland client protocols、xkbcommon、Tokio 和 Axum。独立 Aquamarine 补丁基于官方固定提交 `f31c47a1b9d300847d8fc3108ad959448103dfc1`（v0.15.1），该库遵循上游许可证，参见[第三方声明](docs/third-party.md)。它不内嵌在 Rust 可执行文件中。本仓库没有复制 pixelflux / pcmflux 源码或 Python wheel。

可选的 VA-API / NVENC / AMF / VideoToolbox 硬件 H.264 编码通过独立、与 ABI 绑定的原生 worker 实现。可移植核心保留软件回退，不链接 FFmpeg。构建方式、运行依赖与设备验证限制参见[硬件编码说明](docs/hardware-encoding.md)。

## 许可证

LightOS Remote Desktop 使用 [MIT 许可证](LICENSE)。第三方组件保留各自许可证，参见[第三方声明](docs/third-party.md)。

### 独立光标适配器

本机 Hyprland 可通过一个可选的只读适配器导出真实光标。使用与正在运行的 Hyprland 匹配的开发头文件及 C++23 编译器构建：

```sh
./scripts/build-hyprland-cursor.sh ./target/release/lightos-cursor.so
```

将适配器放在待安装的 `lightos-remoted` 旁，安装器会一起复制并纳入回滚快照；已有旧式安装沿用 `--legacy` 更新方式。服务首次建立独立光标连接时，通过已验证归属的合成器 IPC 加载适配器。运行 ABI 不匹配或图像不可用时保留画面内光标。Hyprland 升级后需使用匹配头文件重新构建适配器。

光标位置与视频帧率分别衡量；鼠标在静止桌面上移动时，低视频 FPS 属于正常情况。
