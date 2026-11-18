# Eddy 开发约定

Eddy 是本地优先的个人阅读器：Rust 内核（`crates/`）通过 UniFFI 给原生界面（`apple/`，以后 `android/`）提供全部业务逻辑；`eddy-server` 是可选的同步中枢。架构决定和理由在 `docs/adr/`，改动边界、数据模型或同步协议前先读对应的 ADR。

## 构建与验证

- Rust：`cargo test --workspace`、`cargo clippy --workspace --all-targets`。首次构建前先跑 `scripts/fetch-pdfium.sh`：Apple 平台上 `eddy-pdf` 静态链接 pdfium，库文件下到 `build/pdfium/`，`.cargo/config.toml` 里的 `PDFIUM_STATIC_LIB_PATH_*` 指向它。
- Windows 上只有内核和 `eddy-cli`，pdfium 用 `scripts/fetch-pdfium.sh windows` 下载。那里 pdfium 是运行时加载的 `pdfium.dll`：`eddy-pdf` 按完整路径加载 exe 旁边的那份，调试构建和测试找不到时再用 `build/pdfium/windows/` 里解出来的。以后别的非 Apple 平台也这样按完整路径加载，绕开系统的库搜索顺序（它包含当前目录）。本机验证不了 Windows：只在 Windows 上编译的代码推上去后才由 CI 的 Windows job 编译和 lint，它用 Git Bash 跑脚本，那里没有 `shasum` 这类 macOS 自带的工具。
- Mac 应用：`scripts/build-mac-app.sh`。它先跑 `scripts/build-apple.sh` 生成 XCFramework 和 Swift 绑定，再用 XcodeGen 生成工程并构建。
- 生成物不进仓库：`apple/Eddy.xcodeproj`、`apple/EddyKit/EddyFFI.xcframework`、`apple/EddyKit/Sources/EddyKit/Generated/`。改了 `eddy-ffi` 的导出接口就重新跑构建脚本。
- 本机的 `xcode-select` 可能指向命令行工具；`scripts/env.sh` 会自动设置 `DEVELOPER_DIR`，手动调用 `xcodebuild` 时也先 source 它。
- 服务端：`deploy/compose.yaml`，端口只映射到 `127.0.0.1`；部署步骤见 `deploy/README.md`。
- 服务端改动除了 `cargo test -p eddy-server`，再跑 `scripts/test-server-smoke.sh`：它在 OrbStack/Docker 里起一个临时服务端，用两个 `eddy-cli` 数据目录走一遍配对、停机、离线改动、恢复、收敛、登出。
- 推到 GitHub 的每个提交都会跑 `.github/workflows/ci.yml`：`cargo fmt --all --check`、clippy 和测试（macOS 上整个 workspace，警告即失败）、Linux 上的服务端测试和镜像构建、Windows 上的内核测试和 `eddy-cli` 构建、Mac 应用构建。每次运行会把顺带编译出来的服务端二进制（linux/amd64）和 Mac 应用（Release 配置，未公证）作为 artifact 保留 14 天。`dev` 和 `main` 的服务端镜像（linux/amd64）还会推到 `ghcr.io/wheheohu/eddy/eddy-server`，标签是分支名和 `sha-<提交>`；`:latest` 和版本号标签只由发布流程打。提交前本地先跑 `cargo fmt --all` 和 clippy，别让 CI 替你发现。
- 不开界面验证真实网络行为（保存网页、连服务端、搜索）用 `tools/eddy-cli`，它的数据和密钥都在 `build/` 下自己的目录里，和应用数据隔离。
- 调试 Mac 应用时不要往用户的真实数据目录里写测试内容：用 `open --env EDDY_DATA_DIR=$PWD/build/app-test apple/DerivedData/Build/Products/Debug/Eddy.app` 指向 `build/` 下的测试目录。设了 `EDDY_DATA_DIR` 的应用用自己的钥匙串条目（`EddyPaths.keychainService`），不会读写真实安装的密钥。
- 阅读视图用 TextKit 2（`NSTextView`），不要改回 SwiftUI 的 `LazyVStack` 拼文章：UX demo 阶段两次卡死都是它在滚动时的布局反馈循环。
- 界面只有一套设计语言，定义在 `apple/Eddy/Shared/Design.swift`（`Theme` 的颜色、圆角和扁平控件：`FlatButtonStyle`、`KeyButtonStyle`、`SwitchBar`、`FlatToggleStyle` 等）：平面、两级色调、方形圆角，不用渐变、内阴影和系统的材质背景。新界面用这些，不直接用系统的 `.secondary`、`.separator`、分段选择器和分组表单。
- 窗口没有系统标题栏和工具栏，不要再给任何视图加 `.toolbar`：每一栏顶部有自己的按钮条（`ColumnStrip`），红绿灯由 `WindowChrome` 挪进领头那一栏的按钮条里（那一栏传 `leads: true` 留出位置）。Library 和 Gallery 的栏用 `LeadingColumn` 加 `SidebarColumn` 排。设置是主窗口里的面板（`SettingsPanel`），页面仍写成 `Form`，由 `PanelFormStyle` 排版。
- 三栏模式不用 `NavigationSplitView`：三栏都由 `PaneContainer` 按窗口宽度的档位（`PaneTier`：三栏、列表加文章、仅文章、一次一栏）排，收起的栏移出屏幕而不卸载，阅读位置和列表滚动位置跨档保留。`ThreePaneView` 只存档位不存宽度（它的 body 会查列表，不能随窗口拖动逐帧重算）。左边的栏盖在右边的栏上，被盖住的阅读区下沉（缩小、圆角、变暗）。侧边栏在每一档都是 240 宽；最窄档它进来时把文章列表向右推 240 并让列表下沉，而不是盖在列表上：两个列表上下叠放时指针悬停会让 AppKit 的约束更新进入死循环而崩溃。会随栏移动的背景用 `Color`，不用 `.background(.background)` 这类背景样式（它不跟着位移）。
- Library 和 Gallery 的侧边栏（以及读书时的目录栏）在 940 以下没有位置放在内容旁边：`ColumnDock` 量窗口宽度，`LeadingColumn` 把栏改成盖在内容上、平时收在左边缘外，按钮或右滑呼出，内容下沉。档位变化用动画表示栏去了哪里。三栏侧边栏的选中项存在 `AppModel.threePaneSelection`，切换模式时保留；三栏和 Library 的侧边栏互相跟随，用户在一边选了什么就通过 `AppModel.chose` 设好另一边（`List` 出现时也会回调一次选中项，那不算用户的选择）。在侧边栏里选一项不收起侧边栏；“侧边栏已呼出”存在 `AppModel.sidebarPulled`，切换模式时保留。
- 侧边栏和各种列表仍是 SwiftUI 的 `List`，用 `.flatList()` 和每行的 `.flatRow(chosen:)` 画成扁平样式（选中底色自己画，系统的高亮由 `TableTuner` 关掉）。分组标题用 `SideHeading` 当普通行，不用 `Section` 的 header（表格会给它画自己的底色）。同一个 `List` 里行的样式随状态变化时只变数值、不变视图类型，否则 SwiftUI 复用单元格时会崩。`List` 增删行本身没有动画：三栏侧边栏的文件夹不用 `DisclosureGroup`，箭头（`FoldArrow`）画在行里，子行是普通的行，展开收起的滑动由 `TableHandle.fold` 直接加在表格的行上（改 `ThreePaneSidebar.rows` 的行结构时同步改 `row(ofFolder:)`）。键盘焦点所在栏的选中行用 `flatRow(chosen:active:)` 在行首画强调色竖条；三栏里左右键在栏之间进出（等同 Esc/Return），空格折叠文件夹。
- 栏跟手滑动时每一帧都会重新布局，滑动结束时所在界面的 body 会连跑几次，所以这条路径上有三条规矩：会随栏移动的视图里不用 `ViewThatFits`（它每次布局都把候选视图连同里面的弹出按钮重建一遍），按宽度自己判断；三栏的侧边栏和文章列表是独立的 `Equatable` 视图（`ThreePaneSidebar`、`ThreePaneArticles`，用 `.equatable()`），内容没变就不更新行；它们自己的 `@AppStorage` 变了并不会让它们重画，决定行怎么画的状态（比如文件夹的折叠）由外面传进来并算进 `==`；`AppModel.items` 的结果缓存到下一次 `reload`/`reloadSoon`，改了数据的地方必须调其中之一。“栏是否显示”由 `LeadingColumn`、`Sliding`、`Sinking` 自己读，不要在界面的 body 里读 `PaneDrag` 或 `sidebarPulled`。
- 界面文字先只做英文（SwiftUI 的字符串字面量就是本地化 key），中文等以后加 `.xcstrings`。

## 版本

- 内核和客户端各有版本号（[ADR 0009](docs/adr/0009-versions-and-compatibility.md)）：内核是根 `Cargo.toml` 的 workspace 版本（`eddy-server`、`eddy-cli` 跟随），Mac 应用是 `apple/project.yml` 的 `MARKETING_VERSION`。只改界面就只升客户端。
- 两份 changelog（根目录 `CHANGELOG.md` 记内核，`apple/CHANGELOG.md` 记 Mac 应用）按 [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/) 写：每个对用户可见的改动，在同一个提交里记到 `## [Unreleased]` 下对应的小节（Added / Changed / Deprecated / Removed / Fixed / Security），写用户看得到的变化，不写实现过程。
- 发布由用户决定时机，agent 不自己发：`scripts/bump-version.sh core|mac X.Y.Z` 设版本号并把 `Unreleased` 变成带日期的版本小节，合并到 `main` 后在那个提交上打标签 `core-vX.Y.Z` 或 `mac-vX.Y.Z`。推送标签会触发 `.github/workflows/release.yml`（内核：发布服务端镜像到 ghcr.io 并建 GitHub Release；Mac：构建应用并附到 Release），标签和仓库里的版本号不一致或 changelog 没有对应小节时不发布。
- 两端能否同步只看 `eddy_sync::PROTOCOL_VERSION`：新增实体、字段，或字段含义变了就加一，并在内核 changelog 里写明；`MIN_PROTOCOL` 只在放弃旧版本时提高。
- 改数据库结构只在 `eddy-store/src/schema.rs` 末尾追加迁移，已有的迁移不改。

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
- 服务端随时可能不在线（[ADR 0010](docs/adr/0010-offline-refresh-from-freshrss.md)）：新功能默认只依赖本地数据；确实要问服务端的，连不上时要有本地结果或明确的"离线"状态（`EngineError::Offline`），不能卡住或当成普通错误弹出来。只有服务端向 FreshRSS 写状态，服务端模式下的客户端只读。
- 登出并清除数据（`sign_out_and_erase`）不在运行中的内核下面删文件：它留一个标记，下次 `Engine::open` 在打开任何东西之前清除。清除的是 `eddy-core/src/erase.rs` 里 `OWNED` 列出的文件和目录，内核在数据目录里新增东西时要加进去；密钥由 `Platform::secret_clear` 一次清空。
- 条目 ID 确定性生成：订阅文章由来源账号和上游 ID 推算 UUIDv5，保存的网页和系列用 UUIDv7。
- 文本偏移一律用 UTF-16 码元，和 Swift/Kotlin 字符串对齐。
- 文档结构升级时提升 `eddy_doc::SCHEMA_VERSION`，旧内容从保存的原始内容重新生成。
