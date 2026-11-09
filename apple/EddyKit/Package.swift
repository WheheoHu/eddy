// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "EddyKit",
    platforms: [.macOS(.v15), .iOS(.v18)],
    products: [
        .library(name: "EddyKit", targets: ["EddyKit"]),
    ],
    targets: [
        .binaryTarget(name: "EddyFFI", path: "EddyFFI.xcframework"),
        .target(
            name: "EddyKit",
            dependencies: ["EddyFFI"],
            path: "Sources/EddyKit",
            linkerSettings: [
                .linkedFramework("Security"),
                .linkedFramework("SystemConfiguration"),
                .linkedFramework("CoreFoundation"),
                // pdfium, statically linked into EddyFFI, is C++ and draws
                // with CoreGraphics.
                .linkedFramework("CoreGraphics"),
                .linkedLibrary("c++"),
                .linkedLibrary("resolv"),
            ]
        ),
    ],
    swiftLanguageModes: [.v5]
)
