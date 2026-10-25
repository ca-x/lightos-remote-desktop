# 场景 1：使用键盘按钮输入文字
ID: SC-TOGGLE-SOFTWARE-KEYBOARD
Profile: draft
Gate: required
Given 手机浏览器已连接桌面并显示默认快捷键栏
When 用户点击第一行最左侧的键盘按钮并输入中文和普通文字
Then 软键盘展开且文字在远端恰好输入一次
And 再次点击键盘按钮收起软键盘
And 键盘收起时快捷键栏固定在可见页面底部并且不遮挡画面
And 悬浮入口隐藏，桌面工具仍可通过底栏菜单打开

# 场景 2：键盘上方操作快捷键
ID: SC-SHORTCUTS-ABOVE-KEYBOARD
Profile: draft
Gate: required
Given 手机软键盘已展开
When 用户操作修饰键、方向键或组合快捷键
Then 快捷键栏保持在可见区域的键盘上方且软键盘保持展开
And 快捷键按预期发送到远端，键盘展开期间不调整远端分辨率

# 场景 3：触摸操作和取消
ID: SC-TOUCH-DESKTOP-CONTROL
Profile: draft
Gate: required
Given 手机浏览器已连接可操作的远端桌面
When 用户点按、单指拖动、长按或双指滚动桌面
Then 远端分别收到点击、拖动、右键或滚动操作
And 手势取消、断线或切换桌面后没有残留按键或鼠标按钮

# 场景 4：高分屏显示清晰虚拟桌面
ID: SC-HIGH-DENSITY-DESKTOP
Profile: draft
Gate: required
Given 像素比为 2 的手机已连接虚拟桌面并启用跟随窗口，目标尺寸未超过编码上限
When 桌面适配手机可用区域
Then 每个逻辑像素对应 2×2 个采集像素
And 桌面的逻辑尺寸保持匹配窗口，文字和控件没有因提高采集分辨率而缩小

# 场景 5：底栏独立固定并避让安全区
ID: SC-MOBILE-DOCK-SAFE-AREA
Profile: draft
Gate: required
Given 用户在普通手机浏览器或懒猫客户端查看远程桌面，页面已获得当前安全区
When 画面的长宽比改变、设备横竖屏切换或宿主安全区更新
Then 快捷键栏始终位于可见区域底部，按键避开手势条和横屏侧边安全区
And 桌面画面在底栏上方的剩余安全区域内按比例居中，既不被底栏遮挡，也不强制贴住底栏上缘
And 懒猫客户端隐藏宿主导航区并保留系统时间、电量，已由宿主处理的安全区不重复留白
And 软键盘展开时底栏移动到键盘上方，键盘收起后恢复底部位置和安全区

# 场景 6：从移动底栏返回管理页
ID: SC-MOBILE-RETURN-TO-LIGHTOS
Profile: draft
Gate: required
Given 用户从 LightOS 管理页进入移动端远程桌面
When 用户点击第二行菜单按钮右侧的主页图标
Then 当前页面返回同一 LightOS 管理入口
And 返回操作不依赖远程桌面连接是否就绪

# 场景 7：只缩放物理桌面画面
ID: SC-PHYSICAL-PICTURE-ZOOM
Profile: draft
Gate: required
Given PC 或手机已连接物理桌面，底栏或工具栏处于当前操作状态
When 用户在画面内双指捏合放大、缩小或用触控板捏合
Then 只改变物理桌面画面的显示比例，页面视口与底栏尺寸和操作状态保持不变
And 放大后可双指平移查看局部，点按对应的远端位置仍然正确
And 捏合不向远端发送点击或滚动，且不改变远端分辨率
When 用户在虚拟桌面、底栏或其他页面区域捏合，或触发页面缩放快捷键
Then 页面保持当前比例，虚拟桌面及底栏不被放大或缩小
When 用户切换桌面或断开连接
Then 原物理画面的局部缩放状态被清理

# 场景 8：移动全屏与自动隐藏退出入口
ID: SC-MOBILE-FULLSCREEN-EXIT
Profile: draft
Gate: required
Given 手机或平板已连接远程桌面，普通快捷栏或全键盘模式可用
When 用户进入全屏
Then 本机软键盘收起，底栏与全键盘模式面板隐藏，画面占用整个可见区域
And 右上角退出全屏图标及其下方的旋转图标显示两秒后一起隐藏
When 用户点击页面任意位置
Then 退出和旋转图标再次显示并从本次点击重新计时两秒，远端点击仍然正常
When 用户点击退出图标，包括远程连接暂不可用的情况
Then 退出全屏并取消自动隐藏计时，进入前的全键盘模式或普通底栏恢复，软键盘保持收起


# 场景 9：全屏横竖方向切换
ID: SC-MOBILE-FULLSCREEN-ROTATION
Profile: draft
Gate: required
Given 手机或平板处于远程桌面的全屏模式
When 用户点击退出按钮下方的旋转按钮
Then 支持系统方向控制时切换屏幕方向，否则在页面内旋转桌面画面以便横屏使用
And 旋转后点击、拖动、滚动及光标位置与远端正确对应，物理桌面仍可双指缩放和平移且分辨率不变
And 退出与旋转按钮位置和大小不随桌面画面旋转或缩放
When 用户再次点击旋转按钮或退出全屏
Then 可以恢复原来的画面方向，退出时清理由页面设置的方向锁定并恢复先前键盘布局
