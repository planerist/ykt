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
            url: "https://github.com/planerist/ykt/releases/download/v0.5.2/YktFFI.xcframework.zip",
            checksum: "2af21201493533ac0a1c1986746c549e3e8805a44441c6f8f26db7c1d53df342"
        ),
        .target(name: "Ykt", dependencies: ["YktFFI"], path: "yrs_swift/Sources/Ykt"),
    ]
)
