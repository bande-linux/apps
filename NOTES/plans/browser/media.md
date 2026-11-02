# Media and DRM

This file plans against the owner's binding requirements in the index ("DRM, Premium Streaming
and Video Playback"). It has three layers that know nothing of each other's internals:
- **DRM authorization** (`web-cdm`): key systems and CDMs.
- **Decode** (`web-media`): demuxers, bitstream parsers, decode backends.
- **Output** (the compositor): protected surfaces, HDCP.

The **capability detector** combines the three.

## Decode

**Demuxers, our own (10–15k):** MP4 and fragmented MP4 with CENC (`pssh`, `tenc`, `senc`,
`saiz`/`saio`, `sbgp`/`sgpd`), WebM/Matroska, Ogg, WAV, MP3 frames.
- MSE needs byte-stream control: append, remove, buffered ranges, eviction, splicing,
  `ManagedMediaSource`.
- It is also the most hostile input.

**Bitstream parsers, our own:** H.264, HEVC, VP9 and AV1 headers (SPS, PPS, slice, frame and
sequence headers), validated before anything reaches a driver. Both hardware APIs below take
parsed parameters, so one set of parsers serves both.

**Decode backends** sit behind one interface (`VideoDecoder`: probe, configure, decode to dmabuf,
flush). Every backend outputs NV12 dmabufs, or P010 for 10-bit and HDR.

| Backend | Where | Codecs |
|---|---|---|
| V4L2 stateless (request API) | the M2's Apple Video Decoder (held driver in the kernel repo, bit-exact); also Rockchip, MediaTek and Hantro boards | H.264, HEVC, VP9 (the M2 has no AV1 hardware) |
| VA-API (libva, loaded at run time) | Intel (iHD) and AMD (radeonsi) machines running Bande on amd64 | H.264, HEVC, VP9, AV1 where the GPU has them |
| Software | everywhere | AV1 through rav1d; H.264, HEVC and VP9 through Debian's libavcodec (open decision 2) |

At run time the browser probes each backend for its codecs, profiles, maximum size and frame
rate, and whether it is power-efficient. It picks the best backend per stream, and reports the
result through MediaCapabilities, so YouTube serves VP9 to the M2's hardware rather than AV1 in
software.

**Audio.** Our own Opus decoder (RFC 6716 and RFC 8251, with their test vectors, 6–9 eng-mo).
Symphonia for AAC-LC, MP3, Vorbis and FLAC. Output through PipeWire with the desktop's
`bande-audio`.

**Security.** Everything above runs in the sandboxed media process. It has no network and no
files, and it reaches only the V4L2 or DRM render node and its own memfds. The AVD driver is
fuzzed through V4L2 before it ships.

**Patents.** H.264 and HEVC decoding in hardware carries the hardware vendor's licence. Software
decoding of those codecs is left to Debian's libavcodec.

## DRM authorization

**Key systems.** A `KeySystem` interface covers capabilities, sessions, decrypt-only, and
decrypt-and-decode. Its implementations:
- **Clear Key**, our own: required by the spec, and it runs WPT `encrypted-media`.
- **Widevine** through a host for Chromium's CDM interface (`InitializeCdmModule_4`,
  `ContentDecryptionModule_10/11`, `Host_10/11`, in Rust with one audited `unsafe` vtable
  module).
- Later others, such as PlayReady, behind the same interface.

The engine sees only EME. A CDM runs in its own sandboxed process per site (`bande-browser-cdm`).
It has only its library, per-origin storage through the host API, no network, and its own
seccomp policy.

**Upgradable components.**
- CDMs are installed apart from the browser, in `/var/lib/bande-browser/cdm/<system>/<version>/`.
- They are updated by a component updater that checks the vendor's signature and version, from
  the source the licence names (Google's component update service, for Widevine).
- The host negotiates CDM interface versions 10 and 11, so a new CDM needs no browser release.
- A CDM is never fetched without a licence.

**EME API.**
- `requestMediaKeySystemAccess`, `MediaKeySystemAccess`, `MediaKeys`, `MediaKeySession`;
- `setMediaKeys` and the `encrypted` event;
- init data `cenc`, `webm` and `keyids`; encryption schemes `cenc` and `cbcs`.

**Honest robustness.** We answer `requestMediaKeySystemAccess` and MediaCapabilities only from
the detector. Widevine's levels are:
- `SW_SECURE_CRYPTO` and `SW_SECURE_DECODE`: L3.
- `HW_SECURE_CRYPTO`, `HW_SECURE_DECODE` and `HW_SECURE_ALL`: L1.

An `HW_*` level is claimed only when the CDM itself reports hardware security.
`QueryOutputProtectionStatus` reports exactly what the compositor says about the output.

## Capability detector and the fallback algorithm

The detector's inputs:
- **The CDM:** installed, licensed, version, security level.
- **Decode backends:** codec, profile, size, power-efficiency.
- **Output protection:** HDCP version and state of the window's output.
- **The display:** size, refresh, HDR.
- **Policy:** private window, site permission.

For each requested key system configuration, in the site's order, the detector picks the
highest tier that is true on every input:

1. L1 (`HW_SECURE_ALL`) with secure decode and 4K: a hardware TEE and secure buffers, HDCP 2.2
   or later on every output showing the video, a 4K display.
2. L1 at a lower resolution: as 1, but HDCP below 2.2, or a smaller display.
3. L3 with hardware decode: the CDM decrypt-only (where the licence permits it, as Chromium does
   on desktop Linux), then the V4L2 or VA-API backend.
4. L3 with software decode: the CDM decrypts and decodes, or our software backends decode.
5. Lower resolutions or other codecs: the site's own ladder, helped by honest MediaCapabilities
   answers.
6. Unprotected playback when no key system is required.

If no tier is true, the configuration is rejected rather than over-claimed. Playback moves down a
tier when the window moves to an output with weaker protection. Higher tiers come back when it
returns.

**Diagnostics.**
- `bande://media` lists each CDM, backend, output and tier, and for each unavailable tier the
  first condition that failed. For example:
  - "Widevine L1 not available: no hardware-backed TEE on this platform."
  - "HDCP 2.2 not reported by the display driver."
  - "AV1: no hardware decoder; using software."
  - "Widevine not installed: no licence."
- The page's info popover links to it as "Why not higher quality?".

## Protected path and HDCP

**What Linux offers.**
- KMS connectors expose **"Content Protection"** (Undesired / Desired / Enabled) and **"HDCP
  Content Type"** (Type 0 or Type 1 for HDCP 2.2+), which i915 and amdgpu implement.
- Secure decode buffers need a TEE-backed protected dma-buf heap. Our kernel has only the `system`
  and `cma` heaps. Protected-memory work upstream targets OP-TEE Arm boards.
- Wayland has Weston's `weston_content_protection` protocol: a surface asks for a protection
  type, and the compositor shows it only on outputs that meet it.

**On the M2 today.**
- Apple's DCP driver lists an `hdcp` firmware endpoint (`HDCP_ENDPOINT = 0x2b`) but implements
  nothing for it and attaches no content-protection property. HARDWARE-GAPS has no HDCP entry.
- So no HDCP is reported, and protected surfaces can't be enforced on the output.

**Milestones.**
- **K2 (kernel):** trace macOS's use of the DCP `hdcp` endpoint, then implement "Content
  Protection" and "HDCP Content Type" on the DCP connector if the firmware supports it.
- **D6 (desktop):** a content-protection protocol in Weston's shape:
  - the compositor reads the connector properties;
  - protected surfaces show only on outputs that meet their type;
  - protected surfaces are black in screenshots, screencasts and the screenshot and screencast
    portals, as on macOS.

  D6 is worth doing even without HDCP, so DRM video is never recorded against the provider's
  terms.
- **D5 (desktop):** NV12 and P010 multi-planar dmabufs with modifiers in the compositor (today it
  takes single-plane ARGB/XRGB LINEAR only), a video subsurface in the toolkit, and protected
  dmabufs passed through untouched where a platform has them. HDR output needs compositor colour
  management later.

## L1 honesty: what the M2 can reach

- **L1 (tiers 1–2) is not reachable on the M2 under Linux**, nor on any desktop Linux today.
  - Widevine L1 runs Google's OEMCrypto inside a hardware TEE that Google certifies and
    provisions per device family.
  - Apple Silicon has no TrustZone OS for Linux: there is no EL3, and the Secure Enclave runs only
    Apple's firmware.
  - Google ships no L1 for desktop Linux on any hardware.
- **The realistic top tier is 3**: Widevine L3, with the AVD decoding H.264, HEVC and VP9 where
  the licence allows decrypt-only. Otherwise it's tier 4.
- **Providers' L3 caps apply** (Netflix: about 720p), and only once Google licenses Browser.
- **Without a licence**, Browser reaches tier 6 itself, and DRM sites open in the system
  browser. That's Debian's chromium or firefox-esr with Asahi's `widevine-installer`: tier 4, at
  the same caps.
- **What would change this:**
  - Google certifying an L1 TEE for a Linux device family, which on Apple Silicon needs Apple;
  - or Bande running on hardware whose vendor ships a Widevine L1 OEMCrypto under Linux (some Arm
    TV and Chromebook SoCs with OP-TEE);
  - plus HDCP 2.2 on the output and Netflix authorizing Browser.

  **Netflix 4K on Linux stays the aspirational target.** The architecture above reaches it
  without redesign once those exist.

## Phase and size

Phase 4, after MSE:
- the decode backends (V4L2 first, then VA-API, then software);
- **W-EME** (Clear Key, the `KeySystem` interface, the detector and diagnostics);
- the Widevine host and the component updater, turned on once licensed.

About 3–4 engineer-years for media and DRM together, with D5, D6, K1 and K2.
