# 场景 1：PC 高分屏采集
ID: SC-PC-NATIVE-DENSITY
Profile: draft
Gate: required
Given 像素比为 2 的 PC 已连接支持缩放的虚拟桌面并启用跟随窗口
When 可用窗口为 1280×800 个 CSS 像素
Then 远端采集为 2560×1600 像素且桌面缩放为 2
And 文字及控件的逻辑尺寸仍对应 1280×800

# 场景 2：PC 缩放变化与手动分辨率
ID: SC-PC-DENSITY-CHANGE
Profile: draft
Gate: required
Given PC 已启用跟随窗口且远端支持 125% 桌面缩放
When 设备像素比从 2 变为 1.25 而窗口 CSS 尺寸不变
Then 1280×800 窗口重新匹配为 1600×1000 采集像素及 1.25 桌面缩放
And 关闭跟随后不自动覆盖用户的手动分辨率

# 场景 3：物理画面按窗口适配
ID: SC-PC-PHYSICAL-NATIVE-PIXELS
Profile: draft
Gate: required
Given PC 正以适应窗口模式查看 1920×1080 的物理桌面且设备像素比为 2
When 可用窗口为 1280×800 个 CSS 像素
Then 显示区域为 1280×720 个 CSS 像素并保持原画面宽高比
And 物理显示器分辨率及缩放不变

# 场景 4：JPEG 保留彩色细节
ID: SC-JPEG-COLOR-DETAIL
Profile: draft
Gate: required
Given 远端 JPEG 画面包含单像素宽的相邻彩色细节
When 画面在客户端解码
Then 相邻细节的颜色保持可区分且没有因色度抽样合并
And H.264 与用户选择的传输策略保持原有行为
