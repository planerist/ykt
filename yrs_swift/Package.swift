// swift-tools-version:6.0

import PackageDescription

let package = Package(
    name: "Ykt",
    platforms: [.macOS(.v12), .iOS(.v15)],
    products: [
        .library(name: "Ykt", targets: ["Ykt"]),
    ],
    targets: [
        // produced by ../build_swift_bindings.sh
        .binaryTarget(name: "YktFFI", path: "YktFFI.xcframework"),
        .target(name: "Ykt", dependencies: ["YktFFI"]),
        .testTarget(name: "YktTests", dependencies: ["Ykt"]),
    ]
)
