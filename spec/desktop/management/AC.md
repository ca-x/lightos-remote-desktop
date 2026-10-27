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
Given 用户首次使用包含当前系统匹配运行时的客户端，并已启用 LightOS 接入和完成系统桌面权限授权
When 用户查看设置，再关闭远程桌面并重启客户端
Then 开关首次默认开启且自动启动，关闭后本机远程桌面停止，重启后仍保持关闭且不自动启动
When 用户重新开启远程桌面
Then 匹配当前系统的远程桌面恢复启动并恢复登录自动启动
