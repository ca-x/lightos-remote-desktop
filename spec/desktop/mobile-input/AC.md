# 场景 1：使用键盘按钮输入文字
ID: SC-TOGGLE-SOFTWARE-KEYBOARD
Profile: draft
Gate: required
Given 手机浏览器已连接桌面并显示默认快捷键栏
When 用户点击第一行最左侧的键盘按钮并输入中文和普通文字
Then 软键盘展开且文字在远端恰好输入一次
And 再次点击键盘按钮收起软键盘
And 键盘收起时快捷键栏位于桌面画面下缘并且不遮挡画面
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
