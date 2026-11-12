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
            url: "https://github.com/planerist/ykt/releases/download/v0.5.1/YktFFI.xcframework.zip",
            checksum: "2f1bad2834ca2b2ee3e36dee04b886ca5563da803163913708a788c3aedb964f"
        ),
        .target(name: "Ykt", dependencies: ["YktFFI"], path: "yrs_swift/Sources/Ykt"),
    ]
)
