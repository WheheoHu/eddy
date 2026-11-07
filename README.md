# Eddy

Eddy 是一个本地优先的个人新闻、收藏和信息流阅读器。订阅来自 FreshRSS，网页文章和教程可以随时保存进来，阅读进度像图书应用一样被记住。

- Rust 内核，三端共用：macOS（SwiftUI + TextKit 2）先行，之后是 iOS 和 Android。
- 不依赖服务端也能完整使用；可选的 `eddy-server` 提供跨设备同步和全量搜索。
- 全文提取、截断检测、抓取策略、AI 翻译/摘要/标签都在客户端执行。

架构决定见 [`docs/adr/`](docs/adr/)。

## 目录

| 路径 | 内容 |
| --- | --- |
| `crates/` | Rust 工作区：模型、文档结构、存储、FreshRSS 对接、提取、抓取、AI、搜索、同步、引擎、FFI、服务端 |
| `apple/` | macOS 应用（XcodeGen 工程）和 `EddyKit` Swift 包 |
| `extensions/chrome/` | 把当前页面交给 Eddy 的 Chrome 扩展 |
| `deploy/` | 服务端的 Dockerfile 和 compose 配置 |
| `scripts/` | 构建和安装脚本 |

## 开发

需要 Rust（rustup）、Xcode 26.3 或更新版本、XcodeGen（`brew install xcodegen`）。

```sh
cargo test --workspace          # Rust 测试
scripts/build-mac-app.sh        # 构建 Rust 内核、生成绑定并构建 Eddy.app
open apple/DerivedData/Build/Products/Debug/Eddy.app
```

Chrome 扩展：在 `chrome://extensions` 打开开发者模式，加载 `extensions/chrome/` 目录，然后在 Eddy 的设置里点"连接 Chrome 扩展"。

服务端（可选）：

```sh
cd deploy
cp .env.example .env            # 填入 EDDY_MASTER_KEY
docker compose up -d --build    # 只监听 127.0.0.1:8787
docker compose exec eddy-server eddy-server pairing-code
```

## 许可证

Apache-2.0 或 MIT，任选其一。
