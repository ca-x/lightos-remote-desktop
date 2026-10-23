# 二进制通信与图片缩放实验（2026-10-23）

实验分支：`experiment/binary-wire`。基线为 `9a9e136`，实验实现提交为 `267de060679266bcd1c552e32db9be71b964d856`。本次只在实验分支验证；默认连接继续使用旧格式，HTTP、MCP、RDP、音频通道与本机安装方式保持现有实现。

**结论**

在与生产相同的 `TCP_NODELAY` 设置下，三种模式都达到设定的约 30 FPS；40 ms 模拟 RTT 下也没有稳定的帧率改善。本样本中，上行控制流量减少约 57%，加上视频后的总通信量减少约 0.19%（JPEG）和 0.60%（H.264）。服务端回放/接收循环省了几毫秒 CPU；浏览器没有一致的 CPU 改善。

优先考虑 frame ID 合并，减少独立消息及状态配对；大范围 Protobuf 迁移暂时缺乏性能依据。现有 JPEG/H.264 压缩字节继续直接发送。Protobuf 的类型契约与跨语言维护价值可单独评估。

图片缩放的候选首选 `fast_image_resize`，本机 AVX2 微基准有明显优势；当前链路没有通用逐帧 CPU resize 阶段，因此这些数字不能称为现有应用的 FPS 提升。

**实验方式与范围**

这次使用真实 Rust WebSocket、TCP、Chromium、JPEG/WebCodecs 解码与 Canvas 绘制，不使用 Rust/Python 客户端替代浏览器。相同预编码画面用于隔离协议变化。完整应用页面另使用合成 DesktopBackend 检查连接、首帧绘制与 ACK；真实桌面采集与操作系统输入未纳入本次测量。

- 机器：13th Gen Intel(R) Core(TM) i5-13500H，Linux x86_64；Chrome/156.0.8078.47；Node v24.22.1；Rust 1.99.1。
- 依赖：prost 0.14.4、Protobuf-ES / protoc-gen-es 2.16.0、protoc 36.2。
- 每轮 1280×720、120 帧、目标 30 FPS；JPEG 每帧 56,581 B，H.264 共 2,090,725 B（x264 ultrafast / zerolatency、4 Mbit/s、GOP 60、无 B 帧）。
- 每帧两次归一化鼠标移动；每 30 帧一次按键按下/释放；绘制后归还 ACK。每轮验证 120 帧、248 个输入、单调 ID 与压缩 payload 总字节一致。
- 每个“链路 × 编码 × 模式”先预热一轮，再运行五轮，轮换模式顺序；共 18 次预热、90 次正式测量。
- 主比较使用生产已有的 `TCP_NODELAY`。另设关闭它的诊断组；生产设置见 `src/main.rs`，本次没有为主程序新增该选项。
- 40 ms RTT 由每方向增加 20 ms 的用户态 TCP 代理模拟，不模拟丢包或带宽上限；代理两侧都显式启用 TCP_NODELAY。
- 一个视频帧在途，用来固定 ACK 契约；这不代表生产调度器的多帧窗口性能。

| 模式 | 视频与 ID | ACK / 高频输入 | 其他控制与 HTTP |
|---|---|---|---|
| legacy（默认） | 现有二进制画面 + 独立 JSON frame ID | JSON | 原样 |
| frames | ID 合入二进制帧头 | JSON | 原样 |
| protobuf | 与 frames 相同 | Protobuf | 原样 |

实验页面参数分别为 `?wire=frames`、`?wire=protobuf`，通过 WebSocket 子协议 `lightos.frames.v1` / `lightos.protobuf.v1` 明确协商。强制实验参数面向本分支服务端；普通连接继续兼容旧格式。JPEG 新头为类型 5 + 8 字节小端 ID；H.264 新头为类型 6 + key 标志 + 8 字节时间戳 + 8 字节 ID。视频数据不套进 Protobuf。

**实际通信量与帧率**

下表为五轮中位数。通信量取代理真实读到的 WebSocket 字节流，包含关闭帧；排除 HTTP 握手、TCP/IP 和 TLS 头。关闭和统计消息造成几个字节的波动，原始数据保留了每轮的值。

| 链路 | 编码 | 模式 | 下行 B/轮 | 上行 B/轮 | 总量减少 | 绘制 FPS |
|---|---|---|---:|---:|---:|---:|
| 本地/生产设置 | JPEG | legacy | 6,794,255 | 17,268 | 0.0000% | 30.001 |
| 本地/生产设置 | JPEG | frames | 6,791,489 | 17,268 | 0.0406% | 30.004 |
| 本地/生产设置 | JPEG | protobuf | 6,791,489 | 7,418 | 0.1852% | 30.003 |
| 本地/生产设置 | H264 | legacy | 2,096,340 | 17,268 | 0.0000% | 30.008 |
| 本地/生产设置 | H264 | frames | 2,093,581 | 17,268 | 0.1305% | 30.007 |
| 本地/生产设置 | H264 | protobuf | 2,093,581 | 7,418 | 0.5966% | 30.007 |
| 模拟 40ms RTT | JPEG | legacy | 6,794,248 | 17,274 | 0.0000% | 22.699 |
| 模拟 40ms RTT | JPEG | frames | 6,791,489 | 17,274 | 0.0405% | 22.619 |
| 模拟 40ms RTT | JPEG | protobuf | 6,791,489 | 7,424 | 0.1851% | 22.589 |
| 模拟 40ms RTT | H264 | legacy | 2,096,341 | 17,274 | 0.0000% | 22.511 |
| 模拟 40ms RTT | H264 | frames | 2,093,581 | 17,274 | 0.1306% | 22.505 |
| 模拟 40ms RTT | H264 | protobuf | 2,093,581 | 7,424 | 0.5966% | 22.668 |

原始压缩视频字节在三个模式中完全相同。总带宽比例取决于画面大小、码率与输入频率；这些比例只适用于这批样本。

**CPU 与绘制耗时**

服务端 CPU 为进程 getrusage，范围是回放发送与命令接收循环，排除读取样本、真实采集和编码。浏览器 CPU 为 CDP 进程 CPU 时间之和，Script/Task 来自 Performance 域；进程计时精度与样本波动使小差异不足以支持普遍提速结论。以下为生产设置的本地链路，单位毫秒/120 帧。

| 编码 | 模式 | 服务端 CPU | 浏览器 CPU | 浏览器 Script | 浏览器 Task | 解码+提交绘制 P95 |
|---|---|---:|---:|---:|---:|---:|
| JPEG | legacy | 28.497 | 980.0 | 54.251 | 241.115 | 4.800 |
| JPEG | frames | 26.760 | 930.0 | 53.288 | 236.574 | 4.800 |
| JPEG | protobuf | 24.658 | 910.0 | 51.659 | 227.343 | 4.900 |
| H264 | legacy | 26.753 | 870.0 | 306.661 | 400.384 | 4.200 |
| H264 | frames | 25.951 | 880.0 | 329.157 | 427.194 | 4.200 |
| H264 | protobuf | 24.298 | 920.0 | 328.913 | 422.334 | 4.300 |

绘制时间终点是 `drawImage` 提交，未测量屏幕实际呈现。回放基准使用同一实验实现来比较三种 wire 格式；没有对整个旧版与新版前端运行时做 CPU A/B。H.264 使用相同的浏览器软件解码配置。

**小包诊断与引入成本**

关闭 TCP_NODELAY 的诊断组中，legacy 约 21 FPS、合并 ID 后约 30 FPS；该设置不对应当前生产程序。早期试跑还受代理自身 Nagle 行为影响，已弃用。不能把这两类差异算作当前软件切换 Protobuf 的收益。

按相同前端工具链重建主分支归档源码，加入实验支持后 `app.js` 从 549,699 B 增至 616,990 B；gzip level 9 从 182,539 B 增至 200,365 B，增加 17,826 B。这个原型静态导入 Protobuf，即使选择 legacy 也有加载成本；后续若保留，需要另评估按需加载或更小生成代码的维护成本。

**缩放路径与 crate 选择**

当前虚拟桌面通过 DisplayController 调整远端分辨率；物理桌面保持显示分辨率，浏览器负责显示大小。JPEG 直接走 TurboJPEG，OpenH264 调用 YUVBuffer::read_rgba8；FFmpeg worker 的 libswscale 目前做相同尺寸的 RGBA→NV12 转换。直接引入 CPU resize 库不会加速已有浏览器缩放。

- `fast_image_resize 6.1.0`：SIMD、运行时 CPU 选择，声明 Rust 1.87，满足仓库声明的 1.88 下限；本机检测为 AVX2。
- `pic-scale 0.7.12`：支持更多颜色空间与滤波；该版本声明 Rust 1.89，本次未引入。不能把版本下限检查当作 Rust 1.88 实际编译验收。
- `yuv 0.8.19`：后续可用于颜色格式转换对照；本次没有测它，也没有替换现有转换器。
- 硬件编码分支已经有 libswscale，增加独立缩放步骤前应比较整条 scale + convert + encode 路径。

独立基准使用不透明 RGBA8 边缘/纹理图，单线程、五次预热与每项 50 次采样；不做线性光颜色空间转换，关闭 alpha 处理。下表比较同类滤波的库 API；两列 alloc 都包含目标缓冲区分配，reuse 复用输出和工作缓冲区。

| 源→目标 | 滤波 | image alloc P50 ms | FIR alloc P50 ms | FIR reuse P50 ms | alloc 加速比 | 相对 image 最大像素分量差 |
|---|---|---:|---:|---:|---:|---:|
| 1280×720 → 640×360 | nearest | 4.332 | 0.087 | 0.068 | 49.97× | 0 |
| 1280×720 → 640×360 | bilinear | 6.244 | 0.412 | 0.386 | 15.15× | 1 |
| 1280×720 → 640×360 | lanczos3 | 10.759 | 0.948 | 0.949 | 11.35× | 1 |
| 1920×1080 → 1280×720 | nearest | 22.859 | 0.399 | 0.226 | 57.23× | 0 |
| 1920×1080 → 1280×720 | bilinear | 25.154 | 1.758 | 1.566 | 14.31× | 1 |
| 1920×1080 → 1280×720 | lanczos3 | 36.718 | 2.802 | 2.643 | 13.11× | 10 |
| 3840×2160 → 1920×1080 | nearest | 128.500 | 1.648 | 1.192 | 77.97× | 0 |
| 3840×2160 → 1920×1080 | bilinear | 93.628 | 4.683 | 4.116 | 19.99× | 1 |
| 3840×2160 → 1920×1080 | lanczos3 | 127.054 | 7.905 | 7.480 | 16.07× | 4 |

两库卷积的舍入/边界行为并非逐像素一致，Lanczos3 的差异尤其需要看真实文字与图标的可读性。Nearest 的低耗时不代表适合所有桌面内容。RGBA alpha=255 的前提必须在接入真实采集时确认。原始结果包含每轮耗时、P95、MSE、标量对照和输出检查。

后续若实现独立的编码分辨率，建议在 media-codec 的编码输入阶段使用 FIR 和复用缓冲区，把桌面逻辑分辨率、编码分辨率、浏览器显示大小分开，再联测缩放、编码、清晰度和网络数据量。本次只交付可复现的对照工具，FIR 没有进入应用运行依赖。

**验证与复现**

- Rust：`cargo test --locked --lib --no-default-features`，120 passed、6 ignored。包括三种格式的锁屏/隐私 epoch 检查、非法二进制输入、过期 ACK 与 Rust/浏览器共享 golden vectors。
- 前端：100 个 Node tests、40 个 Vitest tests 通过；新增协议用例另跑 86 个 frontend tests 通过。`npm run build`、生产库 Clippy `-D warnings`、格式检查通过。
- 真实浏览器：90 次正式回放与 18 次预热完成；完整应用的 legacy / frames / protobuf 三种模式均 Connected，能显示画面并返回 ACK。
- 实现提交 `267de06` 的 GitHub CI（run `39957465828`）已完成，`test`、`native-codec` 与 `build` 均成功；包含 bundled-jpeg / rdp / portal 的 workspace 测试、Clippy、私有 Xvfb / FreeRDP 检查与 release 二进制构建。实验分支不触发版本发布。
- 主程序、AUR、系统服务没有安装/重启/迁移，本实验没有创建发布 tag。

前置工具为可用的 Node、Rust、ffmpeg/ffprobe（包含 libx264）、agent-browser 与它可启动的 Chromium。测试只绑定随机 loopback 端口和临时配置，不使用真实桌面。

```sh
npm ci
npm run build
node scripts/benchmark-wire.mjs --repeats 5
cargo run --release --locked --manifest-path tools/resize-benchmark/Cargo.toml -- 50 > resize-results.json
```

agent-browser 不在 PATH 时可传 `--agent-browser /absolute/path/to/agent-browser`；`--directory` 指定输出目录，`--frames 30 --repeats 1` 可进行短试跑。代码生成需要 protoc：`npm run generate:wire`。

数据文件：[完整 wire 样本](2026-10-23-binary-wire.json)、[缩放样本](2026-10-23-resize.json)、[包体积](2026-10-23-wire-bundles.json)。

一手资料：[fast_image_resize 6.1.0](https://docs.rs/fast_image_resize/6.1.0/fast_image_resize/)、[pic-scale crate 元数据](https://crates.io/api/v1/crates/pic-scale)、[yuv crate 元数据](https://crates.io/api/v1/crates/yuv)、[Protobuf 编码格式](https://protobuf.dev/programming-guides/encoding/)、[Protobuf-ES 源码](https://github.com/bufbuild/protobuf-es/tree/v2.16.0)。
