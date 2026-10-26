# 场景 1：中文输入和普通按键交替使用
ID: SC-DESKTOP-LOCAL-IME
Profile: draft
Gate: required
Given PC 浏览器或 Electron 网页已连接桌面且远端文本输入框已获得焦点
When 用户使用本机输入法连续确认中文候选并与普通键盘输入交替操作
Then 每次确认的中文恰好出现在远端一次且没有拼音按键泄漏，不被替换成数字或触发 Tab 补全等控制操作
And 远端中文输入法已开启时，手机系统键盘确认的英文、空格、标点和中文仍原样进入输入框或终端，不等待远端候选确认
And 普通按键与修饰键继续按预期工作

# 场景 2：连续使用快捷键后继续输入
ID: SC-DESKTOP-REPEATED-SHORTCUTS
Profile: draft
Gate: required
Given 用户已连接桌面并能控制远端应用
When 用户连续使用组合键发送器和键盘快捷键后继续输入文字
Then 每次组合键均生效且后续输入仍送到远端
And 失焦或断线后没有残留的修饰键或主键

# 场景 3：将本机图片粘贴到远端
ID: SC-DESKTOP-PASTE-IMAGE
Profile: draft
Gate: required
Given 本机剪贴板包含不超过传输上限的图片且远端应用支持图片粘贴
When 用户按本机粘贴快捷键或点击粘贴本地内容按钮
Then 远端应用收到同一图片且纯文本粘贴继续可用
And 传输失败时显示原因且不触发旧剪贴板内容的粘贴
And 断线或切换桌面后没有将旧操作内容传给新连接


# 场景 4：虚拟桌面使用本机已配置的输入法
ID: SC-DESKTOP-VIRTUAL-INPUT-METHOD
Profile: draft
Gate: required
Given 本机已配置 Fcitx5 中文输入法，用户进入本工具拥有的虚拟桌面
When 用户使用 Ctrl＋空格或键盘菜单的“切换远端输入法”入口
Then 在该虚拟桌面内切换可用输入法并能输入中文，使用本机的输入方案与静态词库
And 物理桌面及其他虚拟会话的语言状态不随之切换
And 本工具更新或重新连接后继续可用，原有虚拟桌面应用保持运行
