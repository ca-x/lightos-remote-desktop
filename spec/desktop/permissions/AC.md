# 场景 1：首次启动请求缺少的权限
ID: SC-REQUESTS-MISSING-MACOS-PERMISSIONS
Profile: draft
Gate: required
Given macOS 用户运行独立控制程序并已登录微服，在已解锁的 GUI 会话中从懒猫远程开启桌面，屏幕录制或辅助功能尚未授权
When 客户端准备启动被控运行程序
Then 系统向用户请求缺少的权限，设备卡片显示对应授权状态，系统提示在被控设备上出现
When 用户在被控设备上完成授权
Then 远程桌面自动重试启动，授权前不会接受桌面控制

# 场景 2：拒绝授权或关闭功能不反复弹窗
ID: SC-RESPECTS-MACOS-PERMISSION-DECISIONS
Profile: draft
Gate: required
Given 用户在本次客户端启动中已收到系统授权请求，并拒绝或暂未完成授权
When 定时状态刷新和重连发生
Then 系统授权请求不会被重复弹出，用户可以从设备卡片主动重新申请权限
When 用户关闭远程桌面
Then 等待中的授权操作被取消，远程桌面保持停止，迟到的授权结果不会重新启用它
