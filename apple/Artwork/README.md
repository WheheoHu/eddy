# Eddy macOS icon

The mark is a whirlpool that faintly reads as a lowercase e, in indigo, ivory and pale blue shapes that borrow the palette and paper feel of ukiyo-e woodblock prints. It stays abstract: no waves, wave crests or lettering.

- `AppIcon.png`: the 1254 × 1254 RGBA master, made from the approved design with its presentation background removed. Everything outside the rounded tile is transparent, and the padding is part of the canvas.
- `../Eddy/Assets.xcassets/AppIcon.appiconset/`: 1× and 2× PNGs at 16, 32, 128, 256 and 512 pt, ten asset slots in total.
- `../project.yml`: binds the app icon through `ASSETCATALOG_COMPILER_APPICON_NAME: AppIcon`.

The asset format follows [Apple's App Icon Type](https://developer.apple.com/library/archive/documentation/Xcode/Reference/xcode_ref-Asset_Catalog_Format/AppIconType.html).

From the repository root:

```sh
scripts/generate-app-icon.sh
scripts/build-mac-app.sh
```

The generator uses `sips` and `iconutil`, which ship with macOS, to export every size straight from the master. It also writes `build/icons/Eddy.icns` and `build/icons/Eddy.iconset/`. The master and the asset catalog are checked in; the exports under `build/` are not. Everyday builds use the committed assets, so the images do not need regenerating.

An incremental build may update files inside the `.app` without touching the bundle directory's modification time, which leaves Finder and the Dock showing the old icon. `build-mac-app.sh` touches the bundle directory after a successful build to trigger a refresh.

The master was produced with the built-in imagegen tool by removing the surrounding presentation background from the chosen concept. Later exports use the local master and do not depend on a conversation or a generation service. The final background-extraction prompt was:

```text
Use case: background-extraction.
Input image: the single supplied PNG is the EDIT TARGET, the user's approved final Eddy macOS icon design.
Make a production master by removing ONLY the light gray outer presentation background and any outer cast shadow. Those pixels outside the navy rounded square must become genuinely transparent alpha.
Keep the complete navy rounded-square tile and EVERY part of the ivory, medium-blue and pale-blue abstract whirlpool / lowercase e emblem unchanged. Preserve the exact silhouette, proportions, placement, colors, paper-like texture and interior pixels. Do not redraw or reinterpret the artwork. Do not change the e, the curve, the linework, or the roundness of the tile corners.
Keep a square canvas and the existing transparent padding around the tile, with the tile occupying approximately 82% of the canvas width and centered as it is in the source. All four corners of the full canvas and all space outside the rounded tile must be transparent. The navy tile itself, including its internal negative space, remains fully opaque.
Crisp clean antialiased tile edges with no gray or white fringe. No new shadows or gradients. No typography, labels, watermark or extra objects. Output one high-resolution RGBA PNG. This is a cutout of the approved art, not a new design.
```

## Browser marks

`../Eddy/Assets.xcassets/Browsers/` holds the line drawings shown on "Open in Browser" for the default browser: Chrome, Safari, Firefox, Edge, Arc, Opera and Vivaldi. `Shared/BrowserIcon.swift` picks one by the browser's bundle identifier and falls back to the `globe` symbol.

They are the outline `brand-*` icons of [Tabler Icons](https://github.com/tabler/tabler-icons) (MIT; the license ships in the app as `../Eddy/Licenses/TablerIcons.txt`), changed in three ways: the header comment is removed, `stroke="currentColor"` is `#000` (the image sets render as templates, so only the shape counts), and the stroke is 1.5 instead of 2 to sit beside SF Symbols at toolbar size.

To add a browser, add its SVG as an image set with the same `Contents.json` properties (vector preserved, template rendering) and a line for its bundle identifier in `DefaultBrowser.marks`.
