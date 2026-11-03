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

# 场景 3：浏览器解码压力不误报服务端硬编失效
ID: SC-KEEPS-HARDWARE-ENCODING-THROUGH-BROWSER-DECODE-RECOVERY
Profile: draft
Gate: required
Given 设备硬件编码可用且用户通过浏览器连接，浏览器解码配置或处理出现暂时积压
When 用户持续查看桌面或从后台返回连接页面
Then 传输窗口内的帧按顺序处理，后台暂停不会把有效硬件编码器标记为不可用
And 浏览器硬件解码失败时先尝试同一编码流的兼容解码路径，仍失败才回退，并展示真实失败原因

# 场景 4：发送帧率变化保持画面连贯与每帧预算
ID: SC-AVOIDS-ENCODER-RESTARTS-FOR-PACING-CHANGES
Profile: draft
Gate: required
Given 用户通过可用的硬件编码连接桌面，画面尺寸和质量预算未改变
When 传输调度调整发送帧率
Then 编码器不因单纯的发送节奏变化重复重建，画面保持连贯且每帧编码预算不下降
