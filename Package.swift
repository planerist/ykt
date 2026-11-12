// swift-tools-version:6.0
// The release's manifest, written by swift_release_manifest.sh (see .github/workflows/main.yml):
// YktFFI is this release's xcframework. For local development use yrs_swift/Package.swift.

import PackageDescription

let package = Package(
    name: "Ykt",
    platforms: [.macOS(.v12), .iOS(.v15)],
    products: [
        .library(name: "Ykt", targets: ["Ykt"]),
    ],
    targets: [
        .binaryTarget(
            name: "YktFFI",
            url: "https://github.com/planerist/ykt/releases/download/v0.5.0/YktFFI.xcframework.zip",
            checksum: "31a431085e3210ddf17677ea018c65b597416a80780680ffa9b35bce35de43db"
        ),
        .target(name: "Ykt", dependencies: ["YktFFI"], path: "yrs_swift/Sources/Ykt"),
    ]
)
