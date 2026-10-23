# 编码前缩放

浏览器在“适应窗口”和全屏模式下，自动按可见区域 CSS 尺寸 × DPR 提交编码尺寸上限。服务器只在捕获尺寸超出该范围时等比缩小，极小视口仍受编码器 320×200 最小边长限制；清晰优先和原始大小使用原生编码尺寸。该优化适用于 JPEG 与 H.264，不改变物理显示器、虚拟桌面的捕获分辨率、compositor scale、MCP/RDP 截图或归一化输入。虚拟桌面的“跟随窗口”仍是独立的桌面分辨率功能。

缩放使用 `fast_image_resize` 6.1 的 bilinear/SIMD 路径，在 blocking worker 执行，复用内部与输出缓冲。输出通过 Arc 共享给编码器；仍在使用的输出保持不可变。同尺寸直接透传捕获 Arc，JPEG 保留原来的缓存和 4:4:4。FIR 是编译进二进制的 Rust 依赖，不增加需用户安装的运行库。

三种尺寸分别负责不同的事情：

| 尺寸 | 用途 |
| --- | --- |
| 捕获尺寸 | 桌面分辨率菜单、自动分辨率去重、原始大小布局、光标坐标单位 |
| 编码尺寸 | JPEG/H.264 输入、WebCodecs 配置、canvas backing store、ACK 窗口重置 |
| 显示尺寸 | CSS 布局、DPR 和浏览器中实际占用的区域 |

协议通过 `ready.video_scaling` 协商。客户端发送 `{"type":"video_scale","max_size":{"width":1920,"height":1080}}`，或以 `max_size:null` 恢复原生。仅 sequenced frame 会话可缩放，每条 `frame_id` 关联 `source_width` / `source_height`。未发送上限的旧客户端保持原生，新客户端遇到旧服务器不发送新命令；无损 tile 更新会话不启用这条路径。输出尺寸变化会重建 H.264 编码器并先发新配置与关键帧，静态桌面也会刷新。锁屏、capture 暂停和 privacy epoch 变化清理缩放缓冲并丢弃旧结果。

缩小会损失文字细节；需要保真时选择清晰优先或原始大小。网络波动不会直接触发分辨率升降，现有 QoS 继续控制质量、码率和帧率。H.264 的实际流量受运动、插值相位、码率和画面内容影响，不能把像素减少视为固定的带宽收益。硬件编码目前复用同一 CPU 缩放路径，尚未和其 libswscale/GPU 路径做性能对比。

## 接入后的定向验证（2026-10-23）

`media-codec` 的 `scaling-pipeline` 示例通过实际 `RgbaScaler`、blocking task、Arc 传递及 OpenH264 编码 worker 测量。JPEG 使用当前的 4:4:4。输入来自实验分支的浏览器合成桌面 fixture，逐帧循环滚动 3 个源像素；2/3 缩放对应 2 个目标像素。每轮每种方式 8 帧预热、40 帧计时，3 轮交替原生/缩放顺序。以下为每轮 P50 的中位数，单位毫秒：

| 捕获 → 编码 | 编码 | 原生 | FIR 缩放后 | 耗时减少 |
| --- | --- | ---: | ---: | ---: |
| 1920×1080 → 1280×720 | JPEG 4:4:4 q75 | 6.391 | 5.297 | 17.1% |
| 1920×1080 → 1280×720 | 软件 H.264 默认码率 | 9.869 | 7.533 | 23.7% |
| 3840×2160 → 2560×1440 | JPEG 4:4:4 q75 | 21.413 | 18.026 | 15.8% |
| 3840×2160 → 2560×1440 | 软件 H.264 默认码率 | 33.288 | 25.067 | 24.7% |

机器：i5-13500H、Linux x86_64、FIR AVX2、Rust 1.99.1。共 1,152 次输入，其中 960 次计时，编码器未跳帧。原始 P50/P95、输出数和字节数见 [1080p CSV](benchmarks/2026-10-23-production-scaling-1080.csv) 和 [4K CSV](benchmarks/2026-10-23-production-scaling-4k.csv)。这些是编码准备与编码 API 耗时，排除了捕获、fixture 生成、网络、ACK 调度和浏览器，不能换算为应用端到端 FPS。此合成滚动场景的字节数不代表其他运动或弱网的流量收益。

复测使用任意桌面 PNG/JPEG：

```sh
cargo run --release -p media-codec --example scaling-pipeline -- desktop.png
```

相同 fixture 可从已推送的实验提交 `f62d76a` 的 `tools/resize-benchmark/fixtures/desktop-{1920x1080,3840x2160}.png` 取得。本次没有合并该分支的 Protobuf 实验。

真实 Chromium 也通过了隔离的只读 synthetic backend 验证：1280×800 捕获在 800×600 窗口中通过真实 WebSocket/WebCodecs 显示为 H.264 800×500，桌面菜单仍为 1280×800；清晰优先和原始大小恢复 1280×800，回到适应窗口后 H.264/JPEG 均恢复 800×500。此验证未连接或更改宿主桌面。
