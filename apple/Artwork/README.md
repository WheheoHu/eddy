# Eddy macOS 图标

图形是隐约呈现小写 e 的漩涡，采用靛蓝、米白和浅蓝色块，借用浮世绘木版画的配色与纸感。主体保持抽象，不包含海浪、浪尖或文字。

- `AppIcon.png`：经用户确认的设计去除展示背景后的 1254 × 1254 RGBA 母版；圆角底座外透明，留白已包含在画布中。
- `../Eddy/Assets.xcassets/AppIcon.appiconset/`：16、32、128、256、512 pt 的 1× / 2× PNG，共十个资源槽位。
- `../project.yml`：通过 `ASSETCATALOG_COMPILER_APPICON_NAME: AppIcon` 绑定应用图标。

资源格式参考 [Apple App Icon Type](https://developer.apple.com/library/archive/documentation/Xcode/Reference/xcode_ref-Asset_Catalog_Format/AppIconType.html)。

在仓库根目录运行：

```sh
scripts/generate-app-icon.sh
scripts/build-mac-app.sh
```

生成脚本使用 macOS 自带的 `sips` 和 `iconutil`，直接从母版导出各尺寸，并额外生成 `build/icons/Eddy.icns` 和 `build/icons/Eddy.iconset/`。母版与资源目录入库；`build/` 下的导出文件不入库。日常构建直接使用已提交的资源，无需重新生成图片。

母版通过内置 imagegen 工具从选定概念稿移除外围展示背景得到。后续导出使用本地母版，不依赖对话或生成服务。最终背景提取提示词如下：

```text
Use case: background-extraction.
Input image: the single supplied PNG is the EDIT TARGET, the user's approved final Eddy macOS icon design.
Make a production master by removing ONLY the light gray outer presentation background and any outer cast shadow. Those pixels outside the navy rounded square must become genuinely transparent alpha.
Keep the complete navy rounded-square tile and EVERY part of the ivory, medium-blue and pale-blue abstract whirlpool / lowercase e emblem unchanged. Preserve the exact silhouette, proportions, placement, colors, paper-like texture and interior pixels. Do not redraw or reinterpret the artwork. Do not change the e, the curve, the linework, or the roundness of the tile corners.
Keep a square canvas and the existing transparent padding around the tile, with the tile occupying approximately 82% of the canvas width and centered as it is in the source. All four corners of the full canvas and all space outside the rounded tile must be transparent. The navy tile itself, including its internal negative space, remains fully opaque.
Crisp clean antialiased tile edges with no gray or white fringe. No new shadows or gradients. No typography, labels, watermark or extra objects. Output one high-resolution RGBA PNG. This is a cutout of the approved art, not a new design.
```
