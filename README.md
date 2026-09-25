# SiloObjectAudio

A C-callable decoder for lossless object-based audio on Apple platforms: Dolby TrueHD streams,
including the Dolby Atmos object presentation: the bed channels, up to 15 dynamic objects, and the object audio metadata (OAMD)
that positions them, with sample-accurate update timing. It ships as `SiloObjectAudio.xcframework`
with a Swift package, for iOS, tvOS, macOS and visionOS.

[AetherEngine](https://github.com/Silo-Server/AetherEngine) uses it to keep TrueHD Atmos heights
on Apple TV and iPhone: Apple devices cannot bitstream TrueHD, so AetherEngine renders the decoded
objects into a speaker bed and delivers it as Apple Positional Audio, which tvOS sends to an Atmos
receiver as Dolby MAT and iOS renders as Spatial Audio.

The decoding is the [`truehd`](https://github.com/truehdd/truehdd) crate (Apache-2.0), pinned to
an exact commit. This repository adds the C API, an access-unit framer that reports where each
decoded block sits in the input (needed to place audio on a timeline after a seek), OAMD update
timing per ETSI TS 103 420 clause 5.3, and the Apple packaging.

## Use

```swift
.package(url: "https://github.com/Silo-Server/SiloObjectAudio", .upToNextMinor(from: "1.0.0")),
// target dependency:
.product(name: "SiloObjectAudio", package: "SiloObjectAudio"),
```

```swift
import SiloObjectAudio

let decoder = truehd_atmos_decoder_create(TRUEHD_ATMOS_PRESENTATION_HIGHEST)!
defer { truehd_atmos_decoder_destroy(decoder) }

// For every demuxed TrueHD packet:
truehd_atmos_decoder_push(decoder, bytes, count, Int64.min)
var block = TrueHDAtmosBlock()
while truehd_atmos_decoder_pull(decoder, &block) == TRUEHD_ATMOS_OK {
    // block.channels: planar float32, one plane per element (bed channel or object)
    // block.speakers: each element's speaker code, TRUEHD_ATMOS_SPEAKER_OBJECT for objects
    // block.metadata: OAMD updates starting in this block (positions, gain, size, zones, ...)
    // block.input_frame_offset: input frames before this block since the last reset
}

// On seek:
truehd_atmos_decoder_reset(decoder)
```

[`include/truehd_atmos.h`](include/truehd_atmos.h) is the reference: ownership, threading,
coordinates (OAMD room coordinates, x left→right, y front→back, z floor −1 / ear level 0 /
ceiling +1), the input timeline, presentation selection and every metadata field are documented
there.

## Build

```sh
./build.sh
```

Requires Xcode and rustup. The toolchain and targets are pinned in `rust-toolchain.toml` (stable
Rust; every Apple target used here ships a prebuilt std). The script builds each slice with fat
LTO, prelinks it so only the `truehd_atmos_*` functions are global (the C API keeps its `truehd_atmos_` prefix; so it links next to other
Rust static libraries such as LibDovi), and assembles the xcframework. Source paths are remapped
so the binaries carry no build-machine paths. Minimum OS: iOS 18, tvOS 18, macOS 15, visionOS 1.

## Verify

```sh
harness/verify.sh
```

Needs Homebrew `ffmpeg`. It downloads Dolby's "Unfold" demo and 7.1.4 Atmos test file from
archive.org, clones `truehdd` at the pinned commit and builds its CLI for reference decodes, then
checks the macOS slice: PCM bit-exact against `truehdd`, object positions against its
`.atmos.metadata`, reset and seek with an exact input timeline, and arbitrary input chunking. It
also links every other slice and imports the module from Swift. Working files go to `.work/`.

`truehdd`'s metadata export omits the `32 × block_offset_factor` term TS 103 420 5.3 specifies,
so some of its update times are 32 frames earlier than this library's; the verifier accounts for
that difference.

Measured on an M4 Max, one thread, CPU time: 28× realtime for a 16-element Atmos presentation,
46–51× for 7.1.

## Limitations

- Intermediate spatial format (ISF) object positions are not decoded; no content with them was found.
- Screen-anchored positions are reported raw with their screen and depth factors.
- A 96 kHz Atmos stream would decode at 96 kHz; MLP has no reduced-rate path.

## License

Apache License 2.0, see [LICENSE](LICENSE) and [NOTICE](NOTICE) for the included `truehd` and
`oamd` crates and the Rust dependencies compiled into the library.

Dolby, Dolby Atmos and Dolby TrueHD are trademarks of Dolby Laboratories. This project is not
affiliated with or endorsed by Dolby Laboratories.
