#!/bin/bash
# Writes the root Package.swift of a release: the Swift package apps get with
#   .package(url: "https://github.com/<owner>/ykt", from: "<version>")
# Its binary target is the release's YktFFI.xcframework.zip (downloaded by SwiftPM, checked against
# the checksum); its Swift target is yrs_swift/Sources/Ykt, whose generated Ykt.swift the release
# commit carries. CI runs it (.github/workflows/main.yml, job "release"); local development keeps
# using yrs_swift/Package.swift.
#
#   ./swift_release_manifest.sh <zip url> <checksum> > Package.swift
#
# The checksum is the zip's SHA-256 (what `swift package compute-checksum` prints).
set -euo pipefail

if [ $# -ne 2 ]; then
    echo "usage: $0 <xcframework zip url> <sha256 checksum>" >&2
    exit 2
fi
url="$1"
checksum="$2"
if ! [[ "$checksum" =~ ^[0-9a-f]{64}$ ]]; then
    echo "not a SHA-256 checksum: $checksum" >&2
    exit 2
fi
if ! [[ "$url" =~ ^https:// ]]; then
    echo "SwiftPM downloads binary targets over https only: $url" >&2
    exit 2
fi

cat <<EOF
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
            url: "$url",
            checksum: "$checksum"
        ),
        .target(name: "Ykt", dependencies: ["YktFFI"], path: "yrs_swift/Sources/Ykt"),
    ]
)
EOF
