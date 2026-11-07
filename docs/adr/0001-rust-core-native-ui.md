# 0001 Rust 内核加原生界面

状态：已采纳

## 决定

全部业务逻辑（数据模型、存储、同步、抓取、提取、AI、搜索）写在 Rust 工作区里，通过 UniFFI 生成 Swift 和 Kotlin 绑定。界面层完全原生：macOS/iOS 用 SwiftUI 加 TextKit 2，Android 用 Kotlin 加 Compose，各平台的交互设计可以不同。

## 理由

- 三端（macOS、iOS、Android）共用同一份内核，Android 阶段不用重写同步、提取、AI 这些逻辑。
- 作者的背景是 C++ 和 Python，Rust 上手成本低于在 Swift 和 Kotlin 里各写一遍。
- UniFFI 能把 Rust 的 async 函数直接生成 Swift `async` 和 Kotlin `suspend`，Element X、Firefox 都在用这种结构。
- 服务端（`eddy-server`）也用同一批 crate，FreshRSS 对接、文档结构、同步协议只有一份实现。

## 代价

- Apple 端要维护 XCFramework 构建（`scripts/build-apple.sh`），跨语言边界调试不如纯 Swift 方便。
- 平台能力（钥匙串、WebView 抓取、网络状态）通过 UniFFI 的外部接口由平台实现，见 `eddy-ffi` 的 `PlatformServices`。
