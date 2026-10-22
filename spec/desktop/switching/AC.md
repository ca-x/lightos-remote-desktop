# 场景 1：切换后得到目标桌面
ID: SC-SWITCH-TO-TARGET-DESKTOP
Profile: draft
Gate: required
Given 用户连接了虚拟桌面且本机物理桌面可用
When 用户切换到物理桌面，再切回虚拟桌面
Then 每次切换完成后显示所选桌面的画面
And 虚拟桌面原有应用继续运行

# 场景 2：关闭旧连接不会无限等待
ID: SC-RECOVER-FROM-STALLED-CLOSE
Profile: draft
Gate: required
Given 用户已连接桌面且访问链路无法完成旧连接的关闭握手
When 用户切换桌面
Then 用户在三秒内看到旧连接关闭超时的反馈
And 用户可以主动重连或重新选择桌面，迟到的旧连接消息不会改变当前桌面

# 场景 3：失败后停止自动重试
ID: SC-BOUND-FAILED-RECONNECTS
Profile: draft
Gate: required
Given 用户选择的桌面持续连接失败
When 首次连接和两次自动重试均失败
Then 页面停止自动重试并保留错误原因
And 用户可以主动重新连接或切回虚拟桌面

# 场景 4：放弃初始化后释放控制权
ID: SC-CANCEL-PENDING-DESKTOP
Profile: draft
Gate: required
Given 用户发起了尚未就绪的物理桌面连接
When 用户关闭该连接
Then 服务取消该次初始化并允许后续控制端连接
And 两个控制端不能同时向桌面发送输入
