# uu-remote-linux：Linux 桌面兼容性对照

研究日期：2026-10-23。参考本地 `/home/czyt/code/rust/uu-remote-linux`，commit `1009bdeaf28cc44ab94059ffe721933707283891`。本文只描述该快照，不声称代表之后的上游版本。

LightOS 对照基线为 `e5524ce`（v0.1.13）；兼容性分支中已有但尚未提交的会话校验、Wayland 能力探测、Hyprland 旋转修复单独列为进行中，不能当作已发布支持。

参考项目的 Cargo.toml 标注 GPL-3.0-or-later；LightOS 保持 MIT。以下提炼行为、边界与验收要求，不复制参考实现源码。

## 核心结论

发行版、桌面环境、会话协议和远控能力需要分别建模。发行版只影响初始化与配置；桌面名称只帮助候选排序；最终选用的 backend 必须在明确目标会话上证明可以采集和输入。显示管理支持不等于完整远控支持。

参考项目已经把 X11、原生 Wayland、Portal、KMS 区分开，并在 Portal 路径共享采集、输入和剪贴板会话。LightOS 应借鉴这种职责边界，继续保留独立的 DesktopInitializer / DisplayController，再抽 DesktopBackend；不照搬其桌面名称判断、C/FFI 架构或 KMS 特权路径。

## 逐项对照

| 领域 | 参考位置（相对 uu 仓库） | LightOS 状态与决定 |
| --- | --- | --- |
| 会话与后端选择 | `src/desktop_backend.rs:19`、`:48` | 先分清 Wayland 与 X11，避免 DISPLAY 中的 XWayland 抢占真实 Wayland；桌面名称仅排序，实际协议能力决定可用性。 |
| 发现可信度 | `src/desktop_backend.rs:11` | uu 扫 wayland-* 文件名不足以证明可用。保留同 UID、真实 socket、精确目标和有界连接/协议探测。多个目标无明确选择时应报歧义。 |
| X11 连接 | `c_engine/x11_engine.c:87` | 显式 DISPLAY 打不开就失败值得保留；不采纳缺省猜 :0/:1。采集、输入、剪贴板、RandR 共用同一 display / authority，不能修改整个服务进程的环境。 |
| 原生 Wayland | `src/capture.rs`、`src/input.rs` | LightOS 现有 libwayshot 与 virtual keyboard/pointer 可保留，但必须检查实际要求的协议和版本。GNOME/KDE 不支持这些协议时，应明确不可用或走独立 Portal backend。 |
| Portal 会话 | `src/portal.rs`、`src/desktop_session.rs`、`c_engine/portal_engine.c` | 共享 RemoteDesktop + ScreenCast 授权与 PipeWire remote；关闭、取消、拒绝、重试是不同状态。屏幕选择/授权等待不应阻塞输入共享锁或异步执行器。当前 LightOS 尚无完整 Portal 远控路径。 |
| Portal 输入 | `c_engine/portal_eis.inc` | EIS 需要映射到实际 stream/region；不能把编码后的像素坐标当作任意全局桌面坐标。源码明确 ConnectToEIS 后 legacy Notify 路径不可混用，需先完成能力协商。 |
| 剪贴板 | `src/clipboard.rs`、`c_engine/portal_clipboard.inc` | 归属所选 backend；X11 selection owner 必须保持生命周期，Portal 使用同一授权 session。LightOS server 目前固定 wl-copy/paste，需要迁出并报告独立能力。保留大小限制和超时；不记录内容。 |
| 授权恢复 | `src/portal_permissions.rs:26`、`:38` | 私有目录/文件、拒绝符号链接、原子写入、restore token 使用前消费值得借鉴。授权拒绝不能无休止弹框，不能把恢复 token 当成网络访问认证。 |
| 锁屏、失连、恢复 | `src/desktop_session.rs:138`、`src/portal.rs`、`src/session_lock.rs:38` | Portal 关闭后不得继续 ready 或输出旧画面；先释放输入，再等待明确恢复条件。LightOS 当前缺少物理会话锁状态门禁，需与目标会话关联，不能误用其他会话状态。区分临时故障与终止性授权失败；不移植自动解锁或专用客户端黑色视频协议。 |
| 唤醒与空闲 | `src/desktop_power.rs:40`、`:61` | 远程输入未必被桌面视为活动。连接生命周期内可持有 idle inhibitor；唤醒失败应降级而非终止健康采集；不自动解锁、不改用户锁屏策略。阻塞 D-Bus acquire/drop 应离开异步执行器。 |
| 显示调整 | `src/desktop_mode.rs:24`、`:93`、`:114` | uu 主要依赖 xrandr，不比当前 Mutter/KScreen/wlroots/Hyprland/RandR 多后端更完整。继续复用 DisplayController，不退回无超时的命令调用；物理桌面保持只读，owned 会话才允许 resize。 |
| X11 采集性能 | `src/damage_capture.rs:29`、`:79` | XDamage 驱动变化通知、MIT-SHM 降低拷贝可借鉴；必须探测扩展、验证像素布局与 stride，支持不满足优化条件时的明确降级，处理 resize 后资源重建。不要直接照搬固定 32-bit BGRX 假设。 |
| 服务与权限 | `packaging/deb/uuremote-native.service`、`contrib/virtual-desktop/ACCEPTANCE.md` | user graphical-session unit 对附着用户桌面有参考价值，但不能替换现有独立用户 owned session 服务。禁止扫描其他用户进程环境、放宽桌面权限或提升网络服务身份。 |
| 虚拟桌面与 handoff | `contrib/virtual-desktop/uu-virtual-desktop:20` | 参考认证后 ready、退出清理、禁用 X11 TCP 等验收。当前 LightOS 的 owner PID/start、私有 descriptor/lock/socket 活性验证保留，不用参考脚本替代 handoff。 |
| doctor 与依赖 | `src/doctor.rs:49`、`src/engine_plugin.rs:45`、`build.rs:73` | 按实际启动模式与 backend 报必需/可选依赖；程序存在只是证据之一。参考可选插件隔离动态库的设计，优先沿用现有 Rust 依赖，确有 ABI/运行库需要时再加 worker/plugin。 |

## 显示与输入专项补充

- uu 的原生 Wayland 引擎与输出枚举拒绝旋转输出（`c_engine/wayland_engine.c:547`、`src/wayland.rs:237`），并不存在可直接借鉴的 Hyprland Legacy / Lua 旋转处理。LightOS 的旋转修复应根据自身协议与回查测试完成，不应以“对齐 uu”替代验证。
- LightOS 的 RandR 调整目前拒绝负坐标布局（`crates/display-control/src/x11.rs:154`）；应将“读取、采集与点击负坐标输出”和“调整输出布局”分别测试，不能笼统声称全覆盖。
- 参考的 X11 根窗口尺寸改变后重建截图缓冲区（`c_engine/x11_engine.c:125`）值得加入新 backend 生命周期要求。Portal 则需区分帧像素、逻辑输出区域、缩放与 EIS mapping，不能复用某个桌面的固定倍率换算。
- 参考的 native Wayland 在虚拟指针不足时可能用 uinput（`src/input.rs:327`）。本次不以自动提升权限、访问 /dev/uinput 替代授权过的桌面输入；输入能力不足应如实报告或在同一会话选择合法候选。
- 参考包含持续剪贴板同步；LightOS 目前是按请求读写。先实现后端一致的文本读写，持续同步属于额外能力，不能混称已具备。

## 当前必须如实保留的支持边界

- Hyprland / wlroots 原生远控：仍取决于 libwayshot 捕获协议、virtual keyboard、virtual pointer 等能力和版本，不能按桌面名无条件判定支持。
- GNOME / KDE Wayland：已有显示查询或调整后端不意味着可采集、输入；完整远控需要 Portal / PipeWire / 输入协商补齐及实际验收。
- X11：已有 RandR DisplayController；采集、输入、文本、剪贴板 backend 尚待本次实现。
- Omarchy：沿用独立初始化策略，4.x 优先系统 Lua，找不到再 conf；不由 backend 覆盖系统桌面配置。
- KMS：不作为本次通用桌面兼容性的默认回退。

## 自动检测与选择（用户确认的默认行为）

Backend 拆分是内部架构，正常使用无需用户手动选 backend。独立 selector 基于目标会话和真实能力生成候选并自动选择；手动覆盖若保留，仅用于诊断，不作为使用前提。

- 先锁定目标会话，再选择该会话内可用的 backend。Owned session 使用已创建的会话；attach 模式使用配置/可信会话上下文中的目标，不能借自动选择去访问其他用户或随意跳到另一个桌面。
- Wayland 会话按实际捕获、键盘、指针能力选择原生路径或 Portal；同时存在 DISPLAY 不能令 XWayland 抢占真实桌面。原生协议不足时可在同一会话内自动选择 Portal 候选。
- X11 会话在同一 display / authority 上探测捕获与输入，满足条件后自动选择 X11 backend。
- 后端可探测、需要授权、初始化中、可用、拒绝授权、暂时失效和不支持需分别表达。无副作用 discovery 不弹框；Portal 的实际授权在用户发起连接时执行，不能把“有 Portal 服务”直接报成 ready。
- 用户拒绝或取消 Portal 授权后停止该次打开，不以另一输入路径绕过授权，也不反复弹框。
- 区分必需能力与可选剪贴板等能力：缺少剪贴板不排除可用的采集/输入 backend；只读观看是否允许则由连接权限和产品模式明确决定。
- 无可用 backend 时返回具体缺少的协议/权限/依赖，不能静默更换目标桌面；诊断同时显示自动选择结果和原因。

## Rust 依赖选型核对

以下为已检查的本地 crate 源码版本，不是“永久最新版本”承诺。尚未改动 Cargo 依赖。

| 候选 | 核对结果 | 决定 |
| --- | --- | --- |
| x11rb 0.14.0 / 已有 x11rb-async | 已有 RandR 依赖；x11rb RustConnection 提供显式 display，以及带显式 auth 的 stream 连接入口 | 优先复用处理原生 X11 协议；扩展必须实际协商，不能假设 SHM / Damage / XTest 存在。 |
| enigo 0.6.1（MIT） | 支持显式 x11_display，文本与按键管理已有实现；其 x11rb 路径内部 connect 仍读取默认认证环境 | 优先评估复用输入；多会话显式 Xauthority 尚需 worker 隔离或可注入连接方案验证。未验证前不作为已选定实现。 |
| arboard 3.7.0（MIT OR Apache-2.0） | 文本可关闭图片默认 feature，但 X11 初始化走 ambient DISPLAY | 不能直接放入多会话服务进程；需隔离会话环境或可传连接的方案。 |
| x11-clipboard 0.9.3（MIT） | Context::new 可传 display；完整 Clipboard::new 内部仍创建两个 Context::new(None) | 不因低层 Context 支持显式 display 就宣称完整 Clipboard 支持。优先现成协议实现，评估 worker 隔离，避免重写 selection / INCR。 |
| xcap 0.9.8（Apache-2.0） | Linux 使用 ambient display / D-Bus；Wayland screenshot 路径不等于共享 RemoteDesktop 的持续 PipeWire 采集 | 不直接作为串流 backend 替代品。 |
| ashpd / pipewire / reis | 是后续 Portal 的候选组合，本次尚未完成版本、API 与统一 session 的最小验证 | 保留为候选；不能用分别自动开授权会话的库拼成重复弹框路径。 |

X11 authority 不能套用 Wayland runtime 父目录必须 0700 的规则；/tmp/.X11-unix 是公共目录，权限应由本地连接、目标身份及 Xauthority 验证。当前 display-control 还调用 xauth，后续需明确检测或改用成熟的 Rust 认证读取能力，不能默认该程序存在。

## 后续实施顺序与验收

1. 固定 SessionTarget 与自动 selector：区分 owned / attach、Wayland / X11，传播 display、authority、socket、bus；显式目标失败不静默切换其他桌面。
2. 抽 DesktopBackend：统一帧订阅、输入队列、输入释放屏障、freshness、stop、独立能力与剪贴板接口。把 Frame / InputCommand / InputGuard 移到通用层；Web / MCP / RDP 改依赖接口，保留 deadline、取消和 ReleaseSync 语义。
3. 现有 Wayland 接入 trait，完成实际协议门禁；修正 doctor 中终端、timeout、gtk-launch 的必需/可选分类。
4. 本次实现 X11 backend：优先现成 Rust crate；先验证显式 authority 和会话隔离，再决定是否需要专用 worker。复用现有 RandR，处理输出区域、像素格式、鼠标坐标与 resize。SHM / Damage 为能力优化，不能掩盖基础采集失败。
5. GNOME / KDE 的 Portal 作为独立能力路径，按共享会话实现，不用桌面名称冒充完成；在实现与真实验收前明确标为未支持。锁屏、权限拒绝、取消、会话关闭、PipeWire 断流均有状态与输入释放验证。
6. 通过隔离测试后再集成审查、提交与发布；由用户使用 AUR 更新生产服务。

验收需要覆盖：Wayland + XWayland 共存；两个同 UID 会话不串桌面；无效 display / 错误 authority / 不回应 socket 有界失败；缺失扩展和缺失工具；输出旋转/缩放/负坐标/边缘；X11 24-depth 32-bpp 与其他布局；静态画面仍能区分健康与停止采集；断开时所有按键和按钮释放；剪贴板 UTF-8、上限、超时、owner 生命周期；owned resize 成功后读回确认。

X11 集成测试应使用临时 Xvfb 与私有 authority，禁止依赖开发机正在登录的桌面。当前 PATH 未发现 Xvfb / xclip / xrandr，因此不能声称已经完成真实 X11 验证。性能结论必须通过同分辨率、同编码设置、同变化场景的测量给出，采用 XDamage / SHM 设计本身不能证明性能已提升。


## 实施与验证更新（2026-10-23）

上面的候选 crate、未实现能力和“PATH 未发现 Xvfb”描述是调研时的状态。
后续按用户批准的计划实现了 `DesktopBackend`、精确会话选择、原生 X11 和
可选 Portal 后端；最终支持边界以 [runtime platforms](../runtime-platforms.md)
和 [实施计划](../superpowers/plans/2026-10-23-linux-compatibility.md) 为准。

X11 已使用单独解包的 Xvfb，在两个临时认证 display 上验证采集、XTest、
Unicode、剪贴板、多会话隔离、排队写入取消和停止清理，没有访问当前登录桌面。
当前路径使用 core GetImage，尚未加入 SHM/Damage 优化；本机 Xvfb 不提供测试所需的
RandR resize，不能将相应跳过项记作真机调整通过。Portal 已有编译和单元测试，
GNOME/KDE 真机、分数缩放与性能比较仍未验收。没有从 GPL 参考项目复制源码。
