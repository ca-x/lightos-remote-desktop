# 场景 1：在同一入口查看并连接有权访问的设备
ID: SC-OPENS-AVAILABLE-CONTAINER-AND-PHYSICAL-DESKTOPS
Profile: draft
Gate: required
Given 当前账号拥有可连接的容器和运行着控制程序的在线物理机，容器管理应用保持原版本
When 用户打开懒猫远程
Then 页面以带封面的卡片展示设备名称、系统和运行状态，应用名称为“懒猫远程”并使用新的黑白灰专用图标
And 明暗主题均使用黑白灰界面，在微服沉浸模式中尊重宿主安全区，设备操作不会被状态栏或关闭按钮遮挡
And 物理机在桌面关闭时仍可见，权限状态可查看，已就绪桌面可连接，页面不增加分类、搜索或其他管理应用入口

# 场景 2：从管理页启停默认关闭的物理机桌面
ID: SC-PERSISTS-PHYSICAL-DESKTOP-ENABLEMENT
Profile: draft
Gate: required
Given 用户首次运行包含当前系统运行时的控制程序，已登录微服，“接入 LightOS”保持关闭
When 用户在懒猫远程查看该设备
Then 桌面显示已关闭，控制程序仍在线，PC 设置页没有远程桌面开关
When 用户开启桌面并完成本机必要的系统权限授权
Then 桌面进入可连接状态，用户可查看并操作对应机器
When 用户关闭桌面，再重启控制程序
Then 已打开的连接失效，桌面保持关闭，设备卡片仍可再次开启

# 场景 3：远程桌面与终端分别授权
ID: SC-DESKTOP-INDEPENDENT-OF-TERMINAL
Profile: draft
Gate: required
Given 用户已登录微服，物理机运行独立控制程序且“接入 LightOS”关闭
When 用户从懒猫远程开启并连接该物理机，再开启和关闭“接入 LightOS”
Then 桌面可查看和操作，终端设置变化不改变桌面连接
When 用户切换客户端当前登录账号
Then 旧账号不能继续控制或连接当前设备，新账号首次接入时桌面保持关闭

# 场景 4：离线设备保留身份和预览但不接受控制
ID: SC-KEEPS-OFFLINE-DEVICE-CARDS
Profile: draft
Gate: required
Given 当前账号曾管理一台物理机并保存过连接预览
When 该设备离线或管理应用重启
Then 卡片与封面保留，离线状态明确，不提供当前不可执行的启停与连接操作
And 其他账号不能读取该设备的控制信息或预览
