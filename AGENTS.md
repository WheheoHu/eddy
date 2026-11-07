# Eddy 开发约定

Eddy 是本地优先的个人阅读器：Rust 内核（`crates/`）通过 UniFFI 给原生界面（`apple/`，以后 `android/`）提供全部业务逻辑；`eddy-server` 是可选的同步中枢。架构决定和理由在 `docs/adr/`，改动边界、数据模型或同步协议前先读对应的 ADR。

## 构建与验证

- Rust：`cargo test --workspace`、`cargo clippy --workspace --all-targets`。
- Mac 应用：`scripts/build-mac-app.sh`。它先跑 `scripts/build-apple.sh` 生成 XCFramework 和 Swift 绑定，再用 XcodeGen 生成工程并构建。
- 生成物不进仓库：`apple/Eddy.xcodeproj`、`apple/EddyKit/EddyFFI.xcframework`、`apple/EddyKit/Sources/EddyKit/Generated/`。改了 `eddy-ffi` 的导出接口就重新跑构建脚本。
- 本机的 `xcode-select` 可能指向命令行工具；`scripts/env.sh` 会自动设置 `DEVELOPER_DIR`，手动调用 `xcodebuild` 时也先 source 它。
- 服务端：`deploy/compose.yaml`，端口只映射到 `127.0.0.1`；部署步骤见 `deploy/README.md`。
- 不开界面验证真实网络行为（保存网页、连服务端、搜索）用 `tools/eddy-cli`，它的数据和密钥都在 `build/` 下自己的目录里，和应用数据隔离。
- 调试 Mac 应用时不要往用户的真实数据目录里写测试内容：用 `open --env EDDY_DATA_DIR=$PWD/build/app-test apple/DerivedData/Build/Products/Debug/Eddy.app` 指向 `build/` 下的测试目录。
- 阅读视图用 TextKit 2（`NSTextView`），不要改回 SwiftUI 的 `LazyVStack` 拼文章：UX demo 阶段两次卡死都是它在滚动时的布局反馈循环。
- 界面文字先只做英文（SwiftUI 的字符串字面量就是本地化 key），中文等以后加 `.xcstrings`。

## 分支

- `main` 是稳定分支，只在用户确认后由 `dev` 合并过去。
- `dev` 是集成分支，不直接在上面开发。新功能从 `dev` 拉本地分支 `feature/<名字>`（修复用 `fix/<名字>`），agent 可以在上面自由提交。
- 完成并通过测试后不要自己合并：停在这个分支上，构建好应用，告诉用户分支名并询问是否合并。用户会在 feature/fix 分支上自己测试，确认后才用 `git merge --no-ff` 合并回 `dev`，再删掉本地分支。
- 远端只保留 `main` 和 `dev`，feature/fix 分支不推送。

## 数据与安全

- 本地数据目录是 `~/Library/Application Support/Eddy`，测试一律用临时目录，保持用户数据原样。
- 密码、API Key、完整请求头只进钥匙串或加密存储；日志、报错、测试输出里用 `[已隐藏]` 占位。真实模型或订阅测试同样遵守。
- 私有付费墙规则包只存在用户本机或其服务端配置里，仓库只放通用抓取策略引擎和不含站点规则的示例格式。
- 推送到 GitHub、发布、绑定局域网或公网地址，都要等用户明确要求。

## 内核约定

- 本地修改先写操作日志（带混合逻辑时钟），再更新状态；单机模式和服务端模式走同一条路径。
- 条目 ID 确定性生成：订阅文章由来源账号和上游 ID 推算 UUIDv5，保存的网页和系列用 UUIDv7。
- 文本偏移一律用 UTF-16 码元，和 Swift/Kotlin 字符串对齐。
- 文档结构升级时提升 `eddy_doc::SCHEMA_VERSION`，旧内容从保存的原始内容重新生成。
