# 场景 1：切换后得到目标桌面
ID: SC-SWITCH-TO-TARGET-DESKTOP
Profile: draft
Gate: required
Given 用户连接了虚拟桌面且本机物理桌面可用
When 用户切换到物理桌面，再切回虚拟桌面
Then 每次切换完成后显示所选桌面的画面
And 虚拟桌面原有应用继续运行

# 场景 2：关闭旧连接不会无限等待
ID: SC-RECOVER-FROM-STALLED-CLOSE
Profile: draft
Gate: required
Given 用户已连接桌面且访问链路无法确认旧会话停止控制，也无法完成关闭握手
When 用户切换桌面
Then 用户在三秒内看到旧连接关闭超时的反馈
And 用户可以主动重连或重新选择桌面，迟到的旧连接消息不会改变当前桌面

# 场景 3：失败后停止自动重试
ID: SC-BOUND-FAILED-RECONNECTS
Profile: draft
Gate: required
Given 用户选择的桌面持续连接失败
When 首次连接和两次自动重试均失败
Then 页面停止自动重试并保留错误原因
And 用户可以主动重新连接或切回虚拟桌面

# 场景 4：放弃初始化后释放控制权
ID: SC-CANCEL-PENDING-DESKTOP
Profile: draft
Gate: required
Given 用户发起了尚未就绪的物理桌面连接
When 用户关闭该连接
Then 服务取消该次初始化并允许后续控制端连接
And 两个控制端不能同时向桌面发送输入


# 场景 5：声音跟随当前桌面
ID: SC-AUDIO-FOLLOWS-DESKTOP
Profile: draft
Gate: required
Given 虚拟桌面与物理桌面均有可用音源且播放不同音频
When 用户分别连接这两个桌面并开启声音
Then 每次只播放当前桌面的声音
And 切换桌面、锁屏或关闭连接后停止旧音频，缺失的音源不会使用麦克风或另一桌面的音源替代

# 场景 6：手机恢复被中断的播放
ID: SC-MOBILE-AUDIO-RECOVERY
Profile: draft
Gate: required
Given 用户已开启桌面声音，手机浏览器暂时中断了音频播放
When 用户再次触摸页面或点击声音按钮
Then 浏览器尝试恢复媒体播放并据实际播放状态显示声音开关
And 恢复失败时提示用户重试，画面连接不受影响

# 场景 7：锁屏时由用户明确选择独立桌面
ID: SC-EXPLICIT-VIRTUAL-DESKTOP-WHILE-PHYSICAL-LOCKED
Profile: draft
Gate: required
Given 用户有权访问的设备已开启远程桌面，当前物理桌面已锁定，系统禁止共享锁屏，且设备具备独立虚拟会话能力
When 用户连接物理桌面
Then 页面明确提示物理桌面已锁定，不自动切换或解锁，并提供使用虚拟桌面的选择
When 用户选择使用虚拟桌面并打开应用，再断开并重新连接该虚拟桌面
Then 用户可查看并操作独立桌面，原有虚拟应用仍在运行，物理桌面保持锁定
And 物理桌面解锁后不会自动改变当前所选桌面

# 场景 8：连接物理桌面的锁屏并恢复原会话
ID: SC-UNLOCK-PHYSICAL-DESKTOP-THROUGH-SYSTEM-PROMPT
Profile: draft
Gate: required
Given 用户已在 macOS、Windows 或 Linux 物理机上完成首次远程访问授权，该设备保持在线，物理桌面已登录并锁屏
When 用户在外部网络通过平板打开懒猫远程
Then 设备卡片显示“已锁屏”并提供进入锁屏界面的入口
When 用户点击该入口
Then 连接显示该桌面的系统锁屏界面，桌面音频和剪贴板暂停
When 用户用平板软键盘在系统锁屏界面输入正确的密码
Then 用户可以在同一连接中继续使用原有桌面会话，仍能看到锁屏前打开的窗口
When 被控设备在连接期间再次锁屏
Then 当前连接显示系统锁屏界面，用户可再次输入密码解锁并继续原会话
