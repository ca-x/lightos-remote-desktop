# Glass 与 LightOS 的 macOS 视频路径

参考 [Glass](https://github.com/Ntarasiuk/glass) 的 `1311c6b5a6b816963c21e689a2a49516d22aecc3`。其 macOS 捕获直接保留 ScreenCaptureKit 的 CVPixelBuffer，交给 VideoToolbox 异步输出；低延迟硬编避免 B 帧及 lookahead。120fps 是采集与编码目标，仍受显示器刷新率、场景变化、网络和接收端呈现限制。

0.1.37 的 Mac 路径先将 BGRA 复制成 RGBA，必要时 CPU 缩放，再经辅助进程传递整帧、转换到硬件编码输入。VideoToolbox 虽是硬件编码，上游仍有整帧复制、颜色转换和 IPC 成本。

0.1.38 默认保留 ScreenCaptureKit 的原生缓冲，直接提交 VideoToolbox。尺寸一致时不做 CPU 像素读取；缩放由 VTPixelTransferSession 处理。JPEG、RDP、软件或兼容编码需要时才生成 RGBA。异步回调只持有独立帧状态，取消或断线后不会引用已销毁的 Rust 编码器。单连接继续使用现有 ACK、最新帧和拥塞控制。

编码器强制硬件选择并启用 [EnableLowLatencyRateControl](https://developer.apple.com/documentation/videotoolbox/kvtvideoencoderspecification_enablelowlatencyratecontrol)。该模式固定无 B 帧、无 lookahead、无限 GOP 和 High/AutoLevel；不逐帧调用 CompleteFrames。码率可在当前会话更新，分辨率改变重建编码器并从关键帧开始。某些 Apple 编码器不公开可选的硬件状态查询，硬件保证来自创建时的 RequireHardwareAcceleratedVideoEncoder。

原生路径失败时先回退到随包 FFmpeg VideoToolbox 兼容路径，再遵循原有软件回退。可设 `LIGHTOS_MACOS_MEDIA_BACKEND=compat` 对照。`video_metrics.encoder_pipeline` 区分 `native` 与 `compatibility`，帧龄使用原始采集时间。锁屏、用户切换、权限和 privacy epoch 校验继续保留。

真机组件验证：Apple Silicon，原生输入 1920×1080、缩放至 1280×720、目标 120fps，连续 12 帧包含一次运行中调码率，12 帧均可解码，约 99ms。此结果只证明原生硬编和缩放链路工作，不能当作端到端 120fps 或延迟结论。完整体验仍需在同一网络、显示器和客户端下由用户对照测试。

后续可独立评估：单独的光标形状/位置通道以减少光标受视频时延影响；Worker 内直接呈现以减少主线程调度；按平台引入原生 GPU 句柄而不影响已稳定的 Linux/Windows。上述项目尚未在本版实现。
