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

# 场景 3：切换与通道恢复
ID: SC-CURSOR-SESSION-RECOVERY
Profile: draft
Gate: required
Given 用户正在使用独立光标
When 切换桌面、断开连接或独立光标通道失效
Then 旧图片及旧连接的清理不能覆盖新桌面的光标状态
And 采集画面始终不包含光标，通道失效时保持独立光标并自动恢复同步，不出现双光标

# 场景 4：光标变化不制造桌面画面更新
ID: SC-CURSOR-NO-VIDEO-REPAINT
Profile: draft
Gate: required
Given 用户连接的桌面内容保持静止，指针区域没有悬停高亮或其他内容变化
When 用户连续移动指针并切换真实光标形状
Then 光标图层跟随指针并更新形状
And 桌面画面的像素保持不变，光标变化不触发额外画面重绘或编码
