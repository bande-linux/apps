# Media and DRM

## Recommendation: our own media stack, hardware first

| Part | Choice | Why |
|---|---|---|
| Demuxers: MP4 and fragmented MP4 (with CENC), WebM/Matroska, Ogg, WAV, MP3 frames | **our own**, 10–15k | bounded specs; MSE needs byte-stream parsing and buffered-range control that a library hides; EME needs CENC boxes (`pssh`, `tenc`, `senc`, `saiz`/`saio`, `sbgp`/`sgpd`); memory-safe parsing of the most hostile input |
| H.264, HEVC, VP9 video | **our own bitstream parsers driving the M2's Apple Video Decoder** through V4L2 stateless decoding, 10–15k | the kernel repo's held AVD driver (`patches/7.3/held/avd`) decodes H.264, HEVC and VP9 bit-exactly through `V4L2_CID_STATELESS_*` controls, so the parsing (SPS, PPS, slice and frame headers) is ours in Rust and the hardware does the pixel work. That is the best performance per watt, and it puts the codec patent licence with Apple's hardware |
| AV1 video (the M2 has no AV1 hardware) | **rav1d**, a dependency | the Rust port of dav1d; 1080p in software costs about two cores. MediaCapabilities reports AV1 as not power-efficient, so YouTube serves VP9, which the AVD decodes |
| H.264, HEVC, VP9 without the AVD (the dev VM, other machines) | **Debian's libavcodec**, loaded at run time in the sandboxed media process | writing software H.264 or VP9 decoders at competitive speed is years of SIMD work, and it carries patent exposure that Debian already takes on |
| Opus | **our own decoder**, 6–9 eng-mo | the web's main audio codec (YouTube, WebRTC); RFC 6716 and RFC 8251 define the bitstream exactly and ship test vectors; there's no mature Rust decoder |
| AAC-LC, MP3, Vorbis, FLAC | **symphonia**, a dependency | pure Rust and fuzzed; AAC-LC's patents have largely expired; HE-AAC is added to it upstream if sites need it |
| Audio output | PipeWire through the desktop's `bande-audio` | one audio path for the desktop |
| Video output | decoded frames as NV12 dmabufs, shown on a video subsurface, scanned out directly when possible | zero copies from the decoder to the screen. **Desktop D5:** multi-planar NV12 dmabufs with modifiers in the compositor (today it takes single-plane ARGB/XRGB LINEAR only), YCbCr sampling, and a video subsurface in the toolkit |

**Security.**
- Every demuxer and parser runs in the sandboxed media process. It has no network and no files,
  and it can reach only the V4L2 device and its own memfds.
- Our parsers validate every header before it reaches the kernel driver, because malformed slice
  parameters reach the kernel, not just userspace. The AVD driver gets fuzzed through the V4L2
  path before it ships in the build.

**MSE.** `MediaSource`, `SourceBuffer` (append, remove, buffered ranges, timestamp offset,
coded-frame eviction and splicing), `ManagedMediaSource`, and fMP4 and WebM byte streams. This
is what YouTube, Netflix and most video sites use.

**Size.** About 2–3 engineer-years for the media element, MSE, demuxers, the AVD path, Opus, A/V
sync and the D5 desktop work, spread over phase 4.

**Kernel prerequisite (K1).** Move `held/avd` into the build, package `avd-fw`, and fuzz the
driver.

## DRM: EME with the Widevine CDM

**What's needed.**
- Netflix, Disney+, Prime Video and Spotify need **Widevine**. No other DRM is offered to
  non-Apple browsers on Linux.
- On desktop Linux, Widevine runs as **L3** (software only, no hardware-protected path) and
  without VMP (Verified Media Path, which Google enforces on Windows and macOS only). Netflix
  therefore caps Linux browsers at about **720p**; other services cap L3 similarly.
- Services also check the User-Agent, so the compatibility token matters.

**How other browsers get it.**
- **Firefox** downloads Google's CDM as a GMP plugin and runs it in a sandboxed plugin process.
- **Chromium** runs it in a sandboxed utility process.
- Both hold Widevine agreements with Google, and so do Brave, Vivaldi, Opera and Edge.
- Google ships no aarch64 Linux CDM. On Asahi, `widevine-installer` downloads a ChromeOS
  recovery image from Google, extracts the aarch64 `libwidevinecdm.so`, and patches its ELF
  alignment so it loads on 16K-page kernels (ours is 16K). Firefox and Chromium are then pointed
  at it. It's a user-run download, not a redistribution.

**Licence.**
- Integrating the CDM into a browser needs an agreement with Google: Widevine's CDM licence,
  plus Google signing or allow-listing the host where VMP applies.
- Google has been slow or unwilling with small and independent browsers, so approval is
  uncertain.
- **Technically**, an unlicensed browser could load a user-installed CDM on Linux: there's no VMP
  check there, and license servers see the CDM's own client identity.
- **Legally**, that uses Google's binary outside the terms it was distributed under, and Google
  can revoke CDM versions. Bande must not ship, download or advertise that path without a
  licence.

**Design** (crate `web-cdm`, process `bande-browser-cdm`).
- **The process:** one sandboxed process per site, like Firefox's GMP process. It has only the
  CDM library, a per-origin storage folder through the host API, no network, and seccomp tuned to
  the CDM.
- **The host side:** we implement Chromium's CDM interface (`InitializeCdmModule_4`,
  `CreateCdmInstance`, `ContentDecryptionModule_10/11` and `Host_10/11`) in Rust. The C++ vtables
  are matched as `#[repr(C)]` function-pointer tables in one audited `unsafe` module, with no C++
  compiler.
- **Decode path:**
  - Audio, and video where the licence allows, is decrypt-only: the CDM returns clear samples
    to our decoders, so the AVD keeps working.
  - Otherwise the CDM decrypts and decodes in software and returns YUV frames, which are copied
    into a dmabuf for the video subsurface.
  - We report no output protection (HDCP), which also caps resolution.
- **EME API:**
  - `navigator.requestMediaKeySystemAccess`;
  - `MediaKeySystemAccess`, `MediaKeys` (`createSession`, `setServerCertificate`);
  - `MediaKeySession` (`generateRequest`, `update`, `load`, `close`, `remove`, `keyStatuses`,
    the `message` event, expiration);
  - `HTMLMediaElement.setMediaKeys` and the `encrypted` event;
  - init data `cenc`, `webm` and `keyids`; encryption schemes `cenc` and `cbcs`.
  - **Clear Key** is ours, built in. It's required by the spec, and it lets WPT
    `encrypted-media` run without Widevine.

**Phase.** Late phase 4, after MSE: **W-EME** (Clear Key and the CDM host, 6–9 eng-mo). The
Widevine host is turned on only once a licence exists.

**Fallback without a licence.** Run Asahi's `widevine-installer` with Debian's chromium or
firefox-esr, which are licensed browsers. Browser offers "Open in System Browser" automatically
when a page asks for `com.widevine.alpha`.
