# 场景 1：已有物理浏览器时虚拟网页应用仍在虚拟桌面
ID: SC-APPLICATION-LAUNCH-DESKTOP-OWNERSHIP
Profile: draft
Gate: required
Given 同一用户的物理桌面和虚拟桌面都在运行，物理桌面已打开浏览器
When 用户在虚拟桌面通过网页应用入口打开页面
Then 新窗口只出现在虚拟桌面，不交给物理浏览器
When 用户再次在虚拟桌面打开页面，并在物理桌面打开另一页面
Then 各次启动均留在发起它的桌面
And 物理桌面的原有窗口及浏览器数据保持不变

# 场景 2：启动入口与持久浏览器配置
ID: SC-APPLICATION-LAUNCH-ENTRYPOINTS
Profile: draft
Gate: required
Given 用户已登录虚拟桌面并安装了 Chromium 系默认浏览器
When 用户从应用菜单、浏览器入口、网页应用快捷键或打开链接入口启动页面
Then 各入口都使用本虚拟桌面的浏览器配置及显示连接
And 宿主强制 Chrome 使用 X11 且继承的 X11 显示不可用时，普通虚拟桌面启动入口仍使用本会话的 Wayland
And 安装的桌面入口文件与物理会话的环境不被改写
When 本工具更新或同一虚拟桌面重新启动
Then 该桌面的浏览器配置路径保持一致
And 不同虚拟桌面的配置路径不同

# 场景 3：更新不中断已有会话，错误会话不回退
ID: SC-APPLICATION-LAUNCH-UPGRADE-GUARD
Profile: draft
Gate: required
Given 本工具正在使用旧版本拥有的虚拟桌面
When 新版本传输服务连接该桌面
Then 本地启动桥接获得更新，原桌面和应用进程继续运行
When 启动桥接收到不属于本桌面的环境或本桌面显示端点已消失
Then 启动失败且不调用目标程序，不在其他桌面创建窗口
