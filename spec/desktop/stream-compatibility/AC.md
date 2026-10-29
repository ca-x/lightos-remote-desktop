# 场景 1：客户端自动编码保持物理桌面可见
ID: SC-KEEPS-PHYSICAL-DESKTOP-VISIBLE-IN-CLIENT
Profile: draft
Gate: required
Given 物理桌面已解锁且具有可辨认的窗口内容，用户在桌面客户端选择自动视频编码
When 用户切换到物理桌面，等待首帧后的编码协商完成，并持续移动鼠标
Then 物理桌面内容保持可见并能显示后续变化，独立光标保留真实形状与透明区域，不显示为实心方块
And 兼容处理保留锁定保护及当前虚拟桌面中已打开的应用

# 场景 2：主服务升级后继续使用已安装的编码组件
ID: SC-RETAINS-ENCODING-WITH-OLDER-WORKER
Profile: draft
Gate: required
Given 系统安装了可正常编码的旧原生组件，新主服务支持该组件尚未提供的帧传输能力
When 用户升级主服务并开启自动视频编码
Then 双方使用共同支持的帧传输方式完成编码，不出现由空帧引起的像素缓冲错误
And 支持新传输能力的组件组合仍可使用其优化路径
