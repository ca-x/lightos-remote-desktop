# 场景 1：查看并打开有权访问的桌面
ID: SC-OPENS-AVAILABLE-CONTAINER-AND-PHYSICAL-DESKTOPS
Profile: draft
Gate: required
Given 当前账号拥有已安装且启用远程桌面的运行中容器和在线物理机，既有来源已提供有效连接 URL，容器管理应用保持原版本
When 用户打开远程桌面管理并分别进入两个目标
Then 用户在同一列表看到容器和物理机的名称，并可查看和操作各自对应的桌面
And 应用名称为“远程桌面”并使用专用 Logo，页面没有分类、搜索或打开其他管理应用的入口，访问目标继续执行账号和设备授权

# 场景 2：客户端开关控制运行及自动启动
ID: SC-PERSISTS-PHYSICAL-DESKTOP-ENABLEMENT
Profile: draft
Gate: required
Given 用户首次使用包含当前系统匹配运行时的客户端，已登录微服并完成系统桌面权限授权，“接入 LightOS”保持关闭
When 用户查看设置，再关闭远程桌面并重启客户端
Then 开关首次默认开启且自动启动，关闭后本机远程桌面停止，重启后仍保持关闭且不自动启动
When 用户重新开启远程桌面
Then 匹配当前系统的远程桌面恢复启动并恢复登录自动启动

# 场景 3：远程桌面与终端分别授权
ID: SC-DESKTOP-INDEPENDENT-OF-TERMINAL
Profile: draft
Gate: required
Given 用户已登录微服并安装远程桌面管理应用，客户端“接入 LightOS”关闭，系统桌面权限已授权
When 用户开启远程桌面并从管理页连接本机
Then 用户可查看和控制本机桌面，客户端没有启动 PTY 或终端核心，也没有向 LightOS 注册实例
When 用户开启再关闭“接入 LightOS”
Then 远程桌面连接保持可用
When 用户关闭远程桌面
Then 远程桌面连接撤销且不会影响独立的终端开关状态
