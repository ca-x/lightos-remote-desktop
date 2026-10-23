# 场景 1：远端真实形状与热点
ID: SC-CURSOR-REMOTE-SHAPE
Profile: draft
Gate: required
Given 用户连接支持独立光标的桌面且远端显示输入框、链接、可调整大小的边缘和自定义光标区域
When 用户依次悬停这些区域并进行拖拽
Then 本机光标更新为远端对应的真实图像与热点
And 透明边缘、高分屏缩放及自定义热点保持正确

# 场景 2：光标独立于视频
ID: SC-CURSOR-INDEPENDENT-MOTION
Profile: draft
Gate: required
Given 用户已显示不含光标的视频帧并收到远端光标图像
When 用户移动指针而视频未产生新帧
Then 本机指针立即移动且后续形状更新无需等待视频绘制
And 远端隐藏光标时本机同步隐藏

# 场景 3：切换与回退
ID: SC-CURSOR-SESSION-FALLBACK
Profile: draft
Gate: required
Given 用户正在使用独立光标
When 切换桌面、断开连接或独立光标通道失效
Then 旧图片及旧连接的清理不能覆盖新桌面的光标状态
And 按实际显示的视频帧切换光标显示方式且不出现双光标
