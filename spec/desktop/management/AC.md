# 场景 1：在同一入口查看并连接有权访问的设备
ID: SC-OPENS-AVAILABLE-CONTAINER-AND-PHYSICAL-DESKTOPS
Profile: draft
Gate: required
Given 当前账号拥有可连接的容器和运行着控制程序的在线物理机，容器管理应用保持原版本
When 用户打开懒猫远程
Then 页面以带封面的卡片展示设备名称、系统和运行状态，应用名称为“懒猫远程”并使用新的黑白灰专用图标
And 物理机与 LightOS 实例的权限行及底部连接按钮在同排横向对齐，实例类型显示“LightOS实例桌面”，可连接实例显示屏幕和输入均已授权
And 只有存在未授权权限时才在权限状态右侧显示申请入口，标题下方说明为“懒猫微服网络保障您的隐私和数据安全。”，状态自动更新且没有刷新按钮
And 滚动设备列表时顶部标题和主题操作保持吸顶可见，内容经过顶部栏时不与标题重叠
And 明暗主题均使用黑白灰界面，在微服沉浸模式中尊重宿主安全区，设备操作不会被状态栏或关闭按钮遮挡
And 其他物理机在桌面关闭时仍可见，权限状态可查看，已就绪桌面可连接，页面按在线和离线分组展示

# 场景 2：从管理页启停默认关闭的物理机桌面
ID: SC-PERSISTS-PHYSICAL-DESKTOP-ENABLEMENT
Profile: draft
Gate: required
Given 用户首次运行包含常驻控制模块的客户端，已登录微服，“接入 LightOS”保持关闭
When 用户从另一台设备打开懒猫远程查看该设备
Then 尚未安装桌面时设备仍可见并显示安装入口，安装后桌面显示已关闭，控制模块仍在线，PC 设置页没有远程桌面开关
When 用户开启桌面并完成本机必要的系统权限授权
Then 服务真正就绪后及时启用连接按钮，不额外等待常规设备刷新周期，用户可查看并操作对应机器
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

# 场景 4：保留离线设备并在上线后恢复连接
ID: SC-RETAINS-OFFLINE-DEVICES-AND-RESTORES-CONNECTION
Profile: draft
Gate: required
Given 当前账号曾管理另一台物理机并保存过连接预览
When 该设备离线或管理应用重启
Then 该设备保留在“离线设备”分类及其计数中，连接和远程操作不可用
When 设备重新上线并完成登记
Then 卡片自动移回“在线设备”分类并沿用原有封面，无需手动刷新
And 其他账号不能读取该设备的控制信息或预览

# 场景 5：从微服安装匹配平台的桌面程序
ID: SC-INSTALLS-MATCHING-RUNTIME-FROM-LPK
Profile: draft
Gate: required
Given 当前账号拥有另一台尚未安装远程桌面的在线设备，客户端控制模块已运行，LPK 已构建当前系统和架构的程序
When 用户在设备卡片点击安装远程桌面
Then 卡片显示真实安装进度和状态，设备只从当前微服应用域名下载匹配平台的程序并校验
And 安装完成后显示已安装版本，桌面仍保持关闭，未授权权限可从卡片主动申请
And 未安装或版本不一致时只提供对应的安装或更新入口，不额外展示客户端升级提示，底部操作与同排实例卡片保持对齐

# 场景 6：确认中断后更新正在运行的桌面
ID: SC-CONFIRMS-INTERRUPTION-BEFORE-UPDATE
Profile: draft
Gate: required
Given 设备已开启远程桌面且正在被连接，LPK 提供不同版本
When 用户在卡片点击更新远程桌面
Then 弹窗说明更新会中断当前连接，并提供取消和确认
When 用户取消
Then 桌面与当前连接保持可用，安装没有开始
When 用户再次点击更新并确认
Then 当前连接被结束，服务自动停止并开始下载安装，无需手动断开或停留在等待连接结束的进度
And 完成后恢复原有开启状态、显示新版本、删除旧版本二进制并立即隐藏进度和完成提示

# 场景 7：更新失败时保留可用的旧版本
ID: SC-RETAINS-WORKING-RUNTIME-ON-UPDATE-FAILURE
Profile: draft
Gate: required
Given 当前版本可用，但本次下载包损坏、签名身份不匹配或无法正常运行
When 用户尝试更新远程桌面
Then 卡片明确显示失败原因和可重试状态，原有已安装版本及其二进制仍保留
And 不会把未通过检查的版本用于桌面服务

# 场景 8：直接更新已关闭的桌面
ID: SC-UPDATES-DISABLED-DESKTOP-DIRECTLY
Profile: draft
Gate: required
Given 设备已经关闭远程桌面且 LPK 提供不同版本
When 用户点击更新远程桌面
Then 直接开始更新，不显示中断连接确认弹窗
And 完成后保持关闭、显示新版本并立即隐藏进度和完成提示

# 场景 9：保留本机卡片并阻止循环连接
ID: SC-SHOWS-CURRENT-DEVICE-AND-BLOCKS-SELF-CONNECTION
Profile: draft
Gate: required
Given 微服能够识别当前客户端的设备身份，当前账号已登记该物理机及其他可访问设备
When 用户打开懒猫远程或等待自动刷新
Then 本机保留在列表及对应分组计数中，连接按钮禁用且文案为“当前设备”
And 本机的安装、更新、权限与启停入口按实际状态保持可用，管理服务重启后继续识别本机
And 其他设备与 LightOS 实例保持可见，不因名称或操作系统相同而被隐藏
When 用户打开先前保存的本机桌面连接地址
Then 连接被阻止并提示从另一台设备访问
When 用户从另一台设备打开懒猫远程
Then 原设备仍按正常在线、权限和运行状态显示并可连接

# 场景 10：启动失败后能够退出并恢复
ID: SC-RECOVERS-FROM-MANAGED-STARTUP-FAILURE
Profile: draft
Gate: required
Given 已安装的桌面服务由客户端控制模块管理
When 服务因桌面不可用等实际错误启动失败
Then 服务进程能够退出，控制模块识别失败，不会因父进程监测停留在无响应的启动状态
And 故障解除后用户可重新开启并连接

# 场景 11：在线与离线设备分别展示
ID: SC-GROUPS-ONLINE-AND-OFFLINE-DEVICES
Profile: draft
Gate: required
Given 当前账号登记了在线和离线设备
When 用户打开管理页或设备在线状态变化
Then 在线设备位于“在线设备”组，离线设备保留在下方“离线设备”组，分组数量与卡片一致
And 离线设备不能连接或执行远程操作，上线后自动恢复到在线组，保留原有预览

# 场景 12：Linux 物理机按实际授权方式提供操作
ID: SC-LINUX-PHYSICAL-PERMISSION-GUIDANCE
Profile: draft
Gate: required
Given 用户通过 PC 客户端接入 Linux 物理机
When 用户查看权限状态并开启、连接远程桌面
Then 已可用的设备直接连接当前真实桌面，连接时确认的设备明确提示在被控机批准屏幕与输入访问
And 系统不支持直接申请时没有无效的申请按钮，会话或组件不可用时可查看原因及处理建议并重新检查
And 拒绝或取消授权后不自动重试、不切换到虚拟桌面，用户可主动重连
And 成功授权后卡片显示“已授权”，隐藏授权提示并提供普通连接入口
And 系统提供可恢复授权时，断线、运行时重启和升级后继续使用已批准授权，不重复要求确认；系统撤销或授权失效时重新确认

# 场景 13：物理机授权调整不改变容器实例连接
ID: SC-CONTAINER-DESKTOP-INDEPENDENT-OF-PHYSICAL-PERMISSIONS
Profile: draft
Gate: required
Given 当前账号有可连接的 LightOS 容器实例，其桌面来源保持原版本
When 用户更新远程桌面管理应用并打开该实例
Then 卡片沿用来源提供的权限与连接地址，实例画面和输入继续可用
And 不出现物理机授权诊断或系统权限申请，不改变实例的桌面会话
