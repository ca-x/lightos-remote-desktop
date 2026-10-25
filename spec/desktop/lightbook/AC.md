# 场景 1：可用设备默认关闭并保留偏好
ID: SC-LIGHTBOOK-AVAILABILITY
Profile: draft
Gate: required
Given 手机、平板或折叠屏首次访问远程桌面
When 用户打开外观与语言设置
Then 主题下方显示默认关闭的 LightBook 开关
When 用户启用后重新打开同一页面
Then LightBook 偏好被保留
And 普通 PC 不能启用 LightBook，缩窄窗口不会改变这一限制

# 场景 2：半屏桌面和深色笔记本面板
ID: SC-LIGHTBOOK-SPLIT-DARK
Profile: draft
Gate: required
Given 可用设备启用了 LightBook
When 用户查看页面、切换横竖屏或改变页面主题
Then 可见安全区域上半部分为桌面，下半部分为键盘与触控板
And 键盘与触控板始终呈现深色，按键分行和居中触控板保持可用
And 原快捷键底栏隐藏，键盘使用 Super 标识，底行从左到右按 Fn、Ctrl、Super、Alt、空格、Alt、Super、方向键排列
And 键盘右上角的菜单和 LightOS 主页按钮持续可用，包括连接失败时
When 用户关闭 LightBook
Then 恢复普通布局与快捷键栏，原远端应用保持运行

# 场景 3：键盘和触控板控制远端
ID: SC-LIGHTBOOK-INPUT
Profile: draft
Gate: required
Given 已就绪的远程桌面启用了 LightBook
When 用户输入文字、使用 Super 或 Ctrl 组合键并操作触控板
Then 按键和组合键送到远端，触控板移动指针、左键与右键点击、滚动和拖拽按对应手势执行
And 不会因为按键触摸默认展开系统软键盘
When 用户取消手势、失去焦点、断线或关闭模式
Then 已按下的键与鼠标按钮被释放

# 场景 4：显式使用系统输入法
ID: SC-LIGHTBOOK-SYSTEM-IME
Profile: draft
Gate: required
Given LightBook 已连接远程桌面
When 用户点击系统输入法入口
Then 本机软键盘展开，LightBook 收起重复的键盘区并保留触控板和菜单
And 仍沿用已确认文字提交一次的输入行为，键盘展开不改变远端分辨率
When 用户收起本机软键盘
Then LightBook 完整键盘和触控板布局恢复
