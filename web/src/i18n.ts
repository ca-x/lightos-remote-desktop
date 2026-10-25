import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import { languageKey, languages, readPreference, resolveLanguage } from "./preferences";

const en = {
  fullKeyboardMode: "Full keyboard mode",
  lightbookDescription: "Desktop above, a dark laptop keyboard and trackpad below. The globe button opens your system input method.",
  lightbookUnavailable: "Available on phones, tablets and foldable touch devices.",
  lightbookKeyboard: "Full keyboard", lightbookTrackpad: "Trackpad", lightbookTrackpadHint: "Trackpad", lightbookSpace: "Space",
  lightbookTrackpadHelp: "Move with one finger; tap to click; tap with two fingers for right-click; scroll with two fingers; double-tap and hold or hold still to drag.",
  desktopNotConnected: "Desktop not connected", desktopRecoveryHint: "Open the tools menu to check the status, reconnect, or select another desktop.",
  rdpTitle: "RDP access",
  rdpEnabled: "Enable RDP",
  rdpHelp: "Connect an RDP client to this virtual desktop using your desktop login. Disconnect browser control first. Video quality is negotiated with the RDP client.",
  rdpPort: "RDP port",
  rdpPortHelp: "Press Enter or leave the field to save. Changing the port disconnects RDP clients; desktop applications keep running.",
  rdpEndpoint: "Server bind address",
  rdpLoopback: "This address is local to the server. For remote access, forward this port over SSH and connect your RDP client to the local forwarded port.",
  rdpNetwork: "Connect to this server using an address reachable from your client. A wildcard bind address is not a client destination.",
  rdpFingerprint: "TLS certificate SHA-256 fingerprint",
  rdpUnsupported: "This server build does not include RDP support.",
  rdpAttached: "RDP is available only for the owned virtual desktop, not an attached physical desktop.",
  rdpUnavailable: "RDP settings are unavailable. Check the server’s private state directory and its permissions.",
  rdpInvalidPort: "Enter a whole port number from 1 to 65535.",
  rdpError: "Could not apply RDP settings. Check your login, the port, TLS configuration and private state permissions. Reopen this tab to refresh.",
  rdpStartupError: "The RDP listener is not running. Check the port and TLS configuration, then enable it again.",
  rdpDisabled: "RDP is off. Enabling it creates a private TLS identity automatically unless the server has an explicit certificate. Verify the fingerprint in your client.",
  rdpSaving: "Loading or applying RDP settings…",

  pictureSize: "Picture scale", customResolution: "Custom resolution…", moreDisplaySettings: "Display settings…",
  keyboardMenu: "Keyboard", clipboardMenu: "Clipboard", releaseCapture: "Release keyboard capture", keyboardHelpTitle: "Keyboard help…", exitFullscreen: "Exit fullscreen",
  about: "About", aboutDescription: "Your desktop, in the browser.", developmentBuild: "Development build", license: "License", projectRepository: "Source code", releaseNotes: "Release notes", reportIssue: "Report an issue",
  toolbar: "Session toolbar", displayMenu: "Display", connectionInfo: "Connection information", dismissMenu: "Close menu", closeSettings: "Close settings", settingsSaved: "Preferences save automatically; apply resolution separately", done: "Done",
  encoderBackend: "H.264 encoder", softwareEncoding: "Software (OpenH264)",
  mcpTitle: "MCP agent access", mcpHelp: "Agents can observe this desktop with a token. Control tokens can also send input to the virtual desktop; Disconnect the browser to hand over control. Physical desktops are read-only.",
  mcpEnabled: "Enable MCP", mcpEndpoint: "Streamable HTTP endpoint", mcpCreateForm: "Create an MCP token", mcpTokenName: "Token name", mcpScope: "Access", mcpObserve: "Observe only", mcpControl: "Observe & control", mcpCreate: "Create token", mcpNewToken: "New MCP token", mcpOnce: "Save this token now. It is shown only once and is cleared when these settings close.", mcpCopy: "Copy token", mcpCopied: "Copied", mcpDone: "Done", mcpTokens: "MCP tokens", mcpNoTokens: "No agent tokens. Create one to connect a client.", mcpRevoke: "Revoke", mcpRevokeNamed: "Revoke {{name}}", mcpError: "Could not update MCP settings. Check your login, connection and the server’s private settings file, then Refresh.", mcpCopyError: "Copy was unavailable. Select and copy the token from the text field.",
  streamSettings: "Video & streaming", videoCodec: "Video encoding", streamStrategy: "Streaming strategy", automatic: "Automatic", lowLatency: "Low latency", smooth: "Smoothness", qualityFirst: "Quality first",
  streamHelp: "Automatic negotiates video support and adjusts buffering from render acknowledgements. Auto tries validated hardware, then software H.264 and JPEG. Your choices are saved.",
  title: "Remote desktop", viewport: "Remote desktop viewport", canvas: "Remote desktop. Focus here to send keyboard and pointer input.",
  open: "Open desktop tools", close: "Close desktop tools", fit: "Fit window", actual: "Actual size", fullscreen: "Fullscreen", audioOn: "Enable audio", audioOff: "Mute audio",
  clipboard: "Text & clipboard", settings: "Settings", source: "Desktop source", monitor: "Physical monitor", refresh: "Refresh", refreshTip: "Refresh sessions and monitors without switching desktops",
  keyboard: "Keyboard & shortcuts", capture: "Capture keyboard in fullscreen", chord: "Send a shortcut to the remote desktop", mainKey: "Main key", sendChord: "Send shortcut",
  keyboardHelp: "Enter fullscreen with the page's fullscreen button to capture keys automatically, then allow the browser's keyboard permission. Browser F11 fullscreen does not enable capture. On Linux, use native Wayland Chrome/Chromium if Super or Alt+Tab still acts on the host. Open the desktop directly over HTTPS or localhost.",
  resolution: "Display & resolution", autoResolution: "Match browser resolution", width: "Width", height: "Height", resize: "Apply resolution",
  input: "Text input / clipboard", sendText: "Send text", getClipboard: "Read remote clipboard", setClipboard: "Write remote clipboard", pasteLocal: "Paste local text or image",
  preferences: "Appearance & language", language: "Language", theme: "Theme", system: "Follow system", browser: "Follow browser", light: "Light", dark: "Dark",
  version: "Software version", waiting: "Waiting for video", connecting: "Connecting. Your browser handles authentication.",
  metrics: "FPS counts frames actually drawn in the last second; idle desktops do not repeat frames. RTT measures an application-level WebSocket round trip, not input-to-display latency.",
  help: "Click the desktop to control it. In the virtual desktop, Super+Space opens apps and Super+Enter opens a terminal. Physical sessions use their own shortcuts. Ctrl+Alt+Shift+Esc releases capture and opens tools. Clipboard access happens only when requested. Switching sources keeps virtual apps running.",
  drag: "Drag to any screen edge. Alt+Arrow moves this toolbar with the keyboard.",
  physicalResize: "Physical monitor resolution cannot be changed here.",
  mobileInput: "Remote desktop keyboard input", mobileShortcuts: "Desktop shortcuts", mobileRow1: "Keyboard and navigation", mobileRow2: "Shortcuts and symbols", mobileMenu: "Desktop menu", mobileHome: "Return to LightOS",
  showKeyboard: "Show keyboard", hideKeyboard: "Hide keyboard", stickyKey: "Sticky {{key}}", arrowUp: "Up arrow", arrowDown: "Down arrow", arrowLeft: "Left arrow", arrowRight: "Right arrow", backspace: "Backspace",
  touchHelp: "Tap to click, drag with one finger, hold to right-click, and scroll with two fingers. Use the leftmost keyboard button to show or hide the keyboard. Highlighted modifiers apply to the next key.",
};
const zh: Record<keyof typeof en, string> = {
  fullKeyboardMode: "全键盘模式",
  lightbookDescription: "上半屏显示桌面，下半屏使用深色键盘和触控板；地球按钮可打开系统输入法。",
  lightbookUnavailable: "仅支持手机、平板和折叠屏触摸设备。",
  lightbookKeyboard: "全键盘", lightbookTrackpad: "触控板", lightbookTrackpadHint: "触控板", lightbookSpace: "空格",
  lightbookTrackpadHelp: "单指移动，轻点左键，双指轻点右键，双指滚动；双击后按住或按住不动可拖拽。",
  desktopNotConnected: "桌面尚未连接", desktopRecoveryHint: "打开工具菜单可查看状态、重新连接或切换桌面。",
  rdpTitle: "RDP 接入",
  rdpEnabled: "启用 RDP",
  rdpHelp: "使用桌面登录凭据，通过 RDP 客户端连接此虚拟桌面。请先断开浏览器控制。画质由 RDP 客户端协商。",
  rdpPort: "RDP 端口",
  rdpPortHelp: "按 Enter 或离开输入框保存。修改端口会断开 RDP 客户端，桌面应用继续运行。",
  rdpEndpoint: "服务器监听地址",
  rdpLoopback: "此地址仅在服务器本机可用。远程连接时，先用 SSH 转发此端口，再让 RDP 客户端连接本地转发端口。",
  rdpNetwork: "请使用客户端可访问的服务器地址连接。通配监听地址不能作为客户端连接目标。",
  rdpFingerprint: "TLS 证书 SHA-256 指纹",
  rdpUnsupported: "此服务器版本未编译 RDP 支持。",
  rdpAttached: "RDP 仅支持本服务拥有的虚拟桌面，不支持附加的物理桌面。",
  rdpUnavailable: "RDP 设置不可用，请检查服务器私有状态目录及权限。",
  rdpInvalidPort: "请输入 1 到 65535 之间的整数端口。",
  rdpError: "无法应用 RDP 设置，请检查登录、端口、TLS 配置及私有状态权限。重新打开此页可刷新。",
  rdpStartupError: "RDP 监听未运行。请检查端口和 TLS 配置后重新启用。",
  rdpDisabled: "RDP 已关闭。启用后会自动创建私有 TLS 身份，除非服务器已指定证书。请在客户端核验证书指纹。",
  rdpSaving: "正在加载或应用 RDP 设置…",

  pictureSize: "画面缩放", customResolution: "自定义分辨率…", moreDisplaySettings: "显示设置…",
  keyboardMenu: "键盘", clipboardMenu: "剪贴板", releaseCapture: "释放键盘捕获", keyboardHelpTitle: "键盘使用帮助…", exitFullscreen: "退出全屏",
  about: "关于", aboutDescription: "在浏览器中，使用你的桌面。", developmentBuild: "开发版本", license: "许可证", projectRepository: "源代码", releaseNotes: "版本记录", reportIssue: "反馈问题",
  toolbar: "会话工具栏", displayMenu: "显示", connectionInfo: "连接信息", dismissMenu: "关闭菜单", closeSettings: "关闭设置", settingsSaved: "偏好自动保存，分辨率需点击应用", done: "完成",
  encoderBackend: "H.264 编码后端", softwareEncoding: "软件编码（OpenH264）",
  mcpTitle: "MCP 智能体接入", mcpHelp: "智能体可凭令牌查看此桌面。控制令牌还可向虚拟桌面发送输入；请先断开浏览器连接以交出控制权。物理桌面仅允许查看。",
  mcpEnabled: "启用 MCP", mcpEndpoint: "Streamable HTTP 地址", mcpCreateForm: "创建 MCP 令牌", mcpTokenName: "令牌名称", mcpScope: "权限", mcpObserve: "仅查看", mcpControl: "查看与控制", mcpCreate: "创建令牌", mcpNewToken: "新 MCP 令牌", mcpOnce: "请立即保存令牌。令牌仅显示一次，关闭此设置后即清除。", mcpCopy: "复制令牌", mcpCopied: "已复制", mcpDone: "完成", mcpTokens: "MCP 令牌", mcpNoTokens: "暂无智能体令牌。创建后即可连接客户端。", mcpRevoke: "撤销", mcpRevokeNamed: "撤销 {{name}}", mcpError: "MCP 设置未能更新。请检查登录、连接和服务的私有设置文件后刷新。", mcpCopyError: "无法自动复制，请在文本框中选中并复制令牌。",
  streamSettings: "视频与传输", videoCodec: "视频编码", streamStrategy: "传输策略", automatic: "自动", lowLatency: "低延迟优先", smooth: "流畅优先", qualityFirst: "清晰优先",
  streamHelp: "自动模式根据画面确认调整缓冲，依次尝试实测可用的硬件编码、软件 H.264 和 JPEG，并保存你的选择。",
  title: "远程桌面", viewport: "远程桌面视口", canvas: "远程桌面，聚焦此处发送键盘和鼠标输入。",
  open: "打开桌面工具栏", close: "收起桌面工具栏", fit: "适应窗口", actual: "原始大小", fullscreen: "全屏", audioOn: "开启声音", audioOff: "关闭声音",
  clipboard: "文字与剪贴板", settings: "设置", source: "桌面来源", monitor: "物理显示器", refresh: "刷新", refreshTip: "刷新会话和显示器列表，不切换桌面",
  keyboard: "键盘与组合键", capture: "全屏捕获键盘", chord: "发送任意组合键到远程", mainKey: "主键", sendChord: "发送组合键",
  keyboardHelp: "点击页面的全屏按钮后自动捕获，并允许浏览器的键盘权限。浏览器 F11 全屏不能开启捕获。Linux 上若 Super 或 Alt+Tab 仍作用于本机，请使用原生 Wayland Chrome/Chromium，并通过 HTTPS 或 localhost 直接打开桌面页面。",
  resolution: "画面与分辨率", autoResolution: "跟随浏览器窗口分辨率", width: "宽", height: "高", resize: "调整分辨率",
  input: "文字输入 / 剪贴板", sendText: "发送文字", getClipboard: "读取远程剪贴板", setClipboard: "写入远程剪贴板", pasteLocal: "粘贴本地文字或图片",
  preferences: "外观与语言", language: "语言", theme: "主题", system: "跟随系统", browser: "跟随浏览器", light: "浅色", dark: "深色",
  version: "软件版本", waiting: "等待画面", connecting: "正在连接，身份验证使用浏览器登录对话框。",
  metrics: "FPS 为最近一秒实际绘制帧数，静止桌面不重复发送。RTT 为 WebSocket 应用层往返时间，包含排队，不等于操作到画面的端到端延迟。",
  help: "点击桌面即可控制。虚拟桌面中 Super+Space 打开应用，Super+Enter 打开终端；物理桌面遵循本机会话的快捷键。Ctrl+Alt+Shift+Esc 释放捕获并打开工具栏。剪贴板只在主动粘贴时读取。切换来源不会关闭虚拟桌面中的应用。",
  drag: "拖动到屏幕任意边缘；Alt+方向键也可调整工具栏位置。", physicalResize: "物理桌面不支持在此更改显示器分辨率。",
  mobileInput: "远程桌面键盘输入", mobileShortcuts: "桌面快捷键", mobileRow1: "键盘与导航", mobileRow2: "组合键与符号", mobileMenu: "桌面菜单", mobileHome: "返回 LightOS 管理页",
  showKeyboard: "展开键盘", hideKeyboard: "收起键盘", stickyKey: "粘滞 {{key}}", arrowUp: "向上", arrowDown: "向下", arrowLeft: "向左", arrowRight: "向右", backspace: "退格",
  touchHelp: "点按点击，单指拖动，长按右键，双指滚动。使用第一行最左侧的键盘按钮展开或收起键盘；高亮的修饰键作用于下一次按键。",
};

const clientPairs: [string, string][] = [
  ["Clipboard image pasted.", "图片已粘贴到远程桌面。"],
  ["Show keyboard", "展开键盘"], ["Hide keyboard", "收起键盘"],
  ["Encode", "编码"], ["Socket send", "连接发送"], ["Decode / draw", "解码 / 绘制"],
  ["Audio", "音频"], ["Buffer", "缓冲"], ["Underruns", "欠载次数"], ["Dropped samples", "丢弃采样数"],
  ["Audio unavailable: Audio connection stopped; click to retry.", "音频不可用：音频连接已断开，点击声音按钮重试。"],
  ["Connecting", "正在连接"], ["Waiting for desktop", "等待桌面就绪"], ["Disconnected", "已断开"], ["Connected", "已连接"],
  ["Waiting for video", "等待画面"], ["Release keyboard capture", "释放键盘捕获"], ["Capture keyboard in fullscreen", "全屏捕获键盘"],
  ["Fit window", "适应窗口"], ["Actual size · 1:1 pixels", "原始大小 · 按设定分辨率 1:1 显示"], ["Fullscreen · Ctrl+Alt+Shift+Esc releases capture", "全屏 · Ctrl+Alt+Shift+Esc 释放键盘"],
  ["Virtual desktop", "虚拟桌面"], ["Physical desktop", "物理桌面"], ["Desktop unavailable", "桌面不可用"],
  ["No keyboard capture. Try fullscreen capture or send a shortcut below for keys intercepted by your system.", "未捕获键盘。全屏可尝试捕获；系统拦截的按键可通过通用组合键发送器发送。"],
  ["Keyboard capture is unsupported. Use the shortcut sender below for system-intercepted keys.", "浏览器不支持键盘捕获。系统截走的组合键可用下方通用发送器发送。"],
  ["All available keys requested. System-reserved keys may still be intercepted. Ctrl+Alt+Shift+Esc releases capture and opens tools.", "已请求捕获所有可用按键。部分系统快捷键仍可能被本机拦截。Ctrl+Alt+Shift+Esc 释放捕获并打开工具栏。"],
  ["Keyboard capture denied. Check browser permissions or use the shortcut sender below.", "键盘捕获被拒绝，请检查浏览器权限或使用下方通用发送器。"],
  ["Connection is too slow; reconnecting to release input safely.", "连接过慢，正在重新连接以安全释放输入。"],
  ["Waiting for network", "等待网络恢复"],
  ["Connection stopped responding; reconnecting.", "连接长时间无响应，正在重新连接。"],
  ["Connection close timed out; retrying.", "连接关闭超时，正在重试。"],
  ["Desktop readiness timed out; reconnecting.", "桌面就绪超时，正在重新连接。"],
  ["Desktop initialization timed out. Reconnect to try again.", "桌面初始化超时，请手动重新连接。"],
  ["Desktop initialization stopped. Reconnect to try again.", "桌面初始化已中断，请手动重新连接。"],
  ["Previous connection did not close within 3 seconds. Reconnect or select another desktop.", "旧连接在 3 秒内未完成关闭，请重新连接或选择其他桌面。"],
  ["Desktop connection failed.", "桌面连接失败。"],
  ["Connected; waiting for server readiness.", "连接已建立，等待服务就绪。"],
  ["Connection failed. Check browser authentication, server availability, or another active controller.", "连接失败，请检查浏览器身份验证、服务状态或是否有其他控制端。"],
  ["Invalid server JSON.", "服务返回了无效的 JSON。"], ["Server supplied an invalid desktop size.", "服务返回了无效的桌面尺寸。"],
  ["Desktop ready. Audio is on.", "桌面已就绪，声音已开启。"], ["Desktop ready. Audio is off until enabled.", "桌面已就绪，点击声音按钮开启音频。"], ["Desktop ready. Server audio is unavailable.", "桌面已就绪，服务音频不可用。"],
  ["Remote clipboard loaded into the text box.", "已将远程剪贴板内容载入文字框。"],
  ["Server supplied invalid JPEG dimensions.", "服务返回了无效的 JPEG 尺寸。"], ["Could not decode a desktop JPEG frame.", "无法解码桌面 JPEG 画面。"],
  ["H.264 is unavailable; using compatible JPEG video.", "H.264 不可用，使用兼容的 JPEG 视频。"], ["Video: JPEG compatibility mode.", "视频：JPEG 兼容模式。"], ["Video: low-latency H.264.", "视频：低延迟 H.264。"],
  ["Text sent.", "文字已发送。"], ["Shortcut sent to the remote desktop.", "组合键已发送到远程桌面。"],
  ["Resolution must be 320–3840 × 200–2160 whole pixels.", "分辨率须为 320–3840 × 200–2160 范围内的整数。"],
  ["Resize requested; waiting for the server.", "已请求调整分辨率，等待服务响应。"], ["Remote clipboard update requested.", "已请求更新远程剪贴板。"],
  ["Clipboard access denied or unavailable. Paste into the text box instead.", "剪贴板访问被拒绝或不可用，请粘贴到文字框。"],
  ["Canvas rendering is unavailable in this browser.", "此浏览器不支持 Canvas 渲染。"],
  ["Enable audio", "开启声音"], ["Mute audio", "关闭声音"], ["Disconnect", "断开连接"], ["Reconnect", "重新连接"],
  ["Fullscreen", "全屏"], ["Exit fullscreen", "退出全屏"], ["Open desktop tools", "打开桌面工具栏"], ["Close desktop tools", "收起桌面工具栏"],
  ["Resize virtual desktop", "调整虚拟桌面分辨率"], ["Physical monitor resolution cannot be changed here", "物理桌面不支持更改显示器分辨率"],
  ["Desktop locked or temporarily unavailable.", "桌面已锁定或暂时不可用。"],
  ["Choose a physical monitor", "选择一块物理显示器"],
];

export function translateClient(text: string): string {
  const pair = clientPairs.find(([english, chinese]) => text === english || text === chinese);
  const chinese = i18n.language.startsWith("zh");
  if (pair) return pair[chinese ? 1 : 0];
  for (const [prefix, chinesePrefix] of [["Server: ", "服务端："], ["Could not connect: ", "无法连接："], ["Audio unavailable: ", "音频不可用："], ["Fullscreen unavailable: ", "全屏不可用："], ["Keyboard capture unavailable: ", "键盘捕获不可用："]]) {
    if (text.startsWith(prefix)) return (chinese ? chinesePrefix : prefix) + text.slice(prefix.length);
  }
  if (chinese) return text
    .replace(/^Automatic reconnect stopped after two retries\. /, "自动重连已尝试两次并停止。")
    .replace(/^Reconnecting in (.+)s$/, "$1 秒后重连")
    .replace(/^Could not connect: /, "无法连接：")
    .replace(/^Audio unavailable: /, "音频不可用：")
    .replace(/^Fullscreen unavailable: /, "全屏不可用：")
    .replace(/^Server: /, "服务端：")
    .replace(/^Text is too large \(maximum (.+) KiB UTF-8\)\.$/, "文字过长（最多 $1 KiB UTF-8）。");
  return text.replaceAll("未发现当前用户的物理 Wayland 会话；请先登录本机桌面", "No physical Wayland session found for this user; sign in on the host desktop first")
    .replaceAll("发现多个 Wayland 会话；请用 --physical-wayland-socket 指定物理桌面", "Multiple Wayland sessions found; configure --physical-wayland-socket")
    .replaceAll("服务以外部桌面模式启动", "Service started in external desktop mode")
    .replaceAll("虚拟桌面", "Virtual desktop").replaceAll("物理桌面", "Physical desktop")
    .replaceAll("已连接", "Connected to ").replaceAll("正在连接", "Connecting to ").replaceAll("已选择", "Selected ")
    .replaceAll("尚未确认", "not yet confirmed").replaceAll("尚未连接", "not connected")
    .replaceAll("连接超时，可切回虚拟桌面", " connection timed out; switch back to Virtual desktop")
    .replaceAll("连接失败，请检查会话是否可用，或切回虚拟桌面", " connection failed; check the session or switch to Virtual desktop")
    .replaceAll("列表已刷新，当前连接尚未确认", "List refreshed; connection not yet confirmed")
    .replaceAll("刷新失败：", "Refresh failed: ").replaceAll("仍可切回虚拟桌面", "You can switch back to Virtual desktop")
    .replaceAll("跟随窗口：", "Following window: ").replaceAll("缩放", "scale").replaceAll("位置", "position")
    .replaceAll("不可用", "unavailable").replaceAll("切换桌面来源", "Switch desktop source").replaceAll("超时", "timeout")
    .replaceAll("。", ". ").replaceAll("；", "; ").replaceAll("，", ", ").replaceAll("：", ": ");
}

void i18n.use(initReactI18next).init({
  resources: { en: { translation: en }, zh: { translation: zh } },
  lng: resolveLanguage(readPreference(languageKey, languages, "system"), navigator.languages ?? [navigator.language]),
  fallbackLng: "en",
  initAsync: false,
  interpolation: { escapeValue: false },
});

export default i18n;
