import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import { languageKey, languages, readPreference, resolveLanguage } from "./preferences";

const en = {
  title: "Remote desktop", viewport: "Remote desktop viewport", canvas: "Remote desktop. Focus here to send keyboard and pointer input.",
  open: "Open desktop tools", close: "Close desktop tools", fit: "Fit window", actual: "Actual size", fullscreen: "Fullscreen", audioOn: "Enable audio", audioOff: "Mute audio",
  clipboard: "Text & clipboard", settings: "Display settings", source: "Desktop source", monitor: "Physical monitor", refresh: "Refresh", refreshTip: "Refresh sessions and monitors without switching desktops",
  keyboard: "Keyboard & shortcuts", capture: "Capture keyboard in fullscreen", chord: "Send a shortcut to the remote desktop", mainKey: "Main key", sendChord: "Send shortcut",
  resolution: "Display & resolution", autoResolution: "Match browser resolution", width: "Width", height: "Height", resize: "Apply resolution",
  input: "Text input / clipboard", sendText: "Send text", getClipboard: "Read remote clipboard", setClipboard: "Write remote clipboard", pasteLocal: "Paste local text",
  preferences: "Appearance & language", language: "Language", theme: "Theme", system: "Follow system", browser: "Follow browser", light: "Light", dark: "Dark",
  version: "Software version", waiting: "Waiting for video", connecting: "Connecting. Your browser handles authentication.",
  metrics: "FPS counts frames actually drawn in the last second; idle desktops do not repeat frames. RTT measures an application-level WebSocket round trip, not input-to-display latency.",
  help: "Click the desktop to control it. In the virtual desktop, Super+Space opens apps and Super+Enter opens a terminal. Physical sessions use their own shortcuts. Ctrl+Alt+Shift+Esc releases capture and opens tools. Clipboard access happens only when requested. Switching sources keeps virtual apps running.",
  drag: "Drag to any screen edge. Alt+Arrow moves this toolbar with the keyboard.",
  physicalResize: "Physical monitor resolution cannot be changed here.",
};
const zh: Record<keyof typeof en, string> = {
  title: "远程桌面", viewport: "远程桌面视口", canvas: "远程桌面，聚焦此处发送键盘和鼠标输入。",
  open: "打开桌面工具栏", close: "收起桌面工具栏", fit: "适应窗口", actual: "原始大小", fullscreen: "全屏", audioOn: "开启声音", audioOff: "关闭声音",
  clipboard: "文字与剪贴板", settings: "画面设置", source: "桌面来源", monitor: "物理显示器", refresh: "刷新", refreshTip: "刷新会话和显示器列表，不切换桌面",
  keyboard: "键盘与组合键", capture: "全屏捕获键盘", chord: "发送任意组合键到远程", mainKey: "主键", sendChord: "发送组合键",
  resolution: "画面与分辨率", autoResolution: "跟随浏览器窗口分辨率", width: "宽", height: "高", resize: "调整分辨率",
  input: "文字输入 / 剪贴板", sendText: "发送文字", getClipboard: "读取远程剪贴板", setClipboard: "写入远程剪贴板", pasteLocal: "粘贴本地文字",
  preferences: "外观与语言", language: "语言", theme: "主题", system: "跟随系统", browser: "跟随浏览器", light: "浅色", dark: "深色",
  version: "软件版本", waiting: "等待画面", connecting: "正在连接，身份验证使用浏览器登录对话框。",
  metrics: "FPS 为最近一秒实际绘制帧数，静止桌面不重复发送。RTT 为 WebSocket 应用层往返时间，包含排队，不等于操作到画面的端到端延迟。",
  help: "点击桌面即可控制。虚拟桌面中 Super+Space 打开应用，Super+Enter 打开终端；物理桌面遵循本机会话的快捷键。Ctrl+Alt+Shift+Esc 释放捕获并打开工具栏。剪贴板只在点击时读取。切换来源不会关闭虚拟桌面中的应用。",
  drag: "拖动到屏幕任意边缘；Alt+方向键也可调整工具栏位置。", physicalResize: "物理桌面不支持在此更改显示器分辨率。",
};

const clientPairs: [string, string][] = [
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
  ["Desktop readiness timed out; reconnecting.", "桌面就绪超时，正在重新连接。"],
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
  ["Choose a physical monitor", "选择一块物理显示器"],
];

export function translateClient(text: string): string {
  const pair = clientPairs.find(([english, chinese]) => text === english || text === chinese);
  const chinese = i18n.language.startsWith("zh");
  if (pair) return pair[chinese ? 1 : 0];
  for (const [prefix, chinesePrefix] of [["Server: ", "服务端："], ["Could not connect: ", "无法连接："], ["Audio unavailable: ", "音频不可用："], ["Fullscreen unavailable: ", "全屏不可用："]]) {
    if (text.startsWith(prefix)) return (chinese ? chinesePrefix : prefix) + text.slice(prefix.length);
  }
  if (chinese) return text
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
