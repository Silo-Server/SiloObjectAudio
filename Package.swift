// swift-tools-version: 6.0

import PackageDescription

let package = Package(
    name: "SiloObjectAudio",
    platforms: [
        .iOS(.v18),
        .tvOS(.v18),
        .macOS(.v15),
        .visionOS(.v1),
    ],
    products: [
        .library(name: "SiloObjectAudio", targets: ["SiloObjectAudio"]),
    ],
    targets: [
        .binaryTarget(name: "SiloObjectAudio", path: "SiloObjectAudio.xcframework"),
    ]
)
