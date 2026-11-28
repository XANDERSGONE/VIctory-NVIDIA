# Public GDK and D3D12 docs mapped to layer gaps

Scope: only documentation Microsoft publishes openly (learn.microsoft.com,
the DirectX-Specs site, public GDK pages). Nothing here comes from the
Xbox-program (NDA) GDK or from leaked material, and none of it should be
added. Code references are to this repository at `9fce116`. Nothing in the
code was changed; each item is a pointer to read, not a verified bug.

## What is and is not public

- **Not public: XMem and D3D12.X.** The XMem reference page on learn.microsoft.com
  is gated: "Access to this topic requires membership in a non-disclosure
  agreement (NDA) Xbox developer program"
  ([XMem API contents](https://learn.microsoft.com/en-us/gaming/gdk/docs/reference/system/xmem/xmem_members)).
  So `victory-hle/src/modules/xmem.rs` (page flags `PAGES_2M`, `PAGES_64K`,
  address ranges) cannot be checked against public docs. Its behaviour stays
  reverse-engineered; the notes below cover only the host (PC) side.
- **Public:** the PC D3D12 and DXGI docs, the
  [DirectX-Specs](https://microsoft.github.io/DirectX-Specs/) site, GameInput
  pages in the public GDK
  ([overview](https://learn.microsoft.com/en-us/gaming/gdk/docs/features/common/input/overviews/input-overview),
  [FAQ](https://learn.microsoft.com/en-us/gaming/gdk/docs/features/common/input/overviews/input-faq),
  [XInput porting](https://learn.microsoft.com/en-us/gaming/gdk/docs/features/common/input/porting/input-porting-xinput)),
  and the [GDK home](https://learn.microsoft.com/en-us/gaming/gdk/) with
  [release notes](https://developer.microsoft.com/en-us/games/articles/2026/11/whats-new-in-the-october-2026-gdk/).
  The public GDK repo is [microsoft/GDK](https://github.com/microsoft/GDK).

## 1. Memory and heaps (RAM/VRAM blockers)

Docs: [D3D12_HEAP_TYPE](https://learn.microsoft.com/en-us/windows/win32/api/d3d12/ne-d3d12-d3d12_heap_type),
[UMA optimizations](https://learn.microsoft.com/en-us/windows/win32/direct3d12/default-texture-mapping),
[memory management](https://learn.microsoft.com/en-us/windows/win32/direct3d12/memory-management),
[uploading resources](https://learn.microsoft.com/en-us/windows/win32/direct3d12/uploading-resources).

- The layer already uses `D3D12_HEAP_TYPE_GPU_UPLOAD` (resizable BAR) in
  `crates/victory-gpu/src/host.rs` near line 10944, and `D3D12_HEAP_TYPE_CUSTOM`
  near 10857 and in `indirect.rs` near 733. The heap-type page is the
  reference for which CPU-page-property combinations are legal on a discrete
  card; check any new custom-heap use against it.
- `docs/findings.md` says the layer's central cost is that title and GPU do not
  share memory. The public docs describe UMA shortcuts only for UMA adapters,
  so they do not remove that cost on an RTX card; they are useful only for the
  integrated-GPU case.
- `memory.rs` and `shadow.rs` keep 64 KiB pages (`PIECE` is 4096 and the
  directory is per 64 KiB page, `memory.rs` header). The memory-management
  page covers residency and budget (`IDXGIAdapter3::QueryVideoMemoryInfo`,
  `MakeResident`/`Evict`); compare with `crates/victory-gpu/src/residency.rs`
  when working the 6/8 GB plan, since the budget it reports is what `windows-memory.md`
  quotes.
- Texture allocation uses `D3D12_HEAP_FLAG_CREATE_NOT_ZEROED` (`texture.rs`
  near 2656); the heap-type page covers the flag. No gap found.

## 2. Submission, fences and sync

Docs: [ID3D12Fence](https://learn.microsoft.com/en-us/windows/win32/api/d3d12/nn-d3d12-id3d12fence),
[fence-based resource management](https://learn.microsoft.com/en-us/windows/win32/direct3d12/fence-based-resource-management).

- `crates/victory-gpu/src/fence.rs` header says "Nothing runs on a GPU yet, so
  only signals move one": the title's fence is a software value with waiter
  events, not a host `ID3D12Fence`. The public fence page describes the
  contract the host fence gives (monotonic 64-bit value, `SetEventOnCompletion`,
  signal and wait on queues). Where the layer still settles by waiting on the
  card (`host.rs` `settle_now`, `settle_for_present`), the doc is the
  reference for replacing a CPU wait with a queue-side wait. This is the
  "two frames in flight" lever named at the end of the RTX 5070 Ti section of
  `docs/findings.md`.

## 3. Enhanced Barriers

Doc: [Enhanced Barriers spec](https://microsoft.github.io/DirectX-Specs/d3d/D3D12EnhancedBarriers.html).

- The code uses only legacy `ResourceBarrier` (about 20 call sites: `host.rs`
  1931-1941, 4875, 4998, 7134, 7459-7471, 7833-7863; `ambient.rs` 568;
  `capture.rs` 411-425; `compose.rs` 284-298). No `ID3D12GraphicsCommandList7`,
  no `Barrier(...)`, and no `EnhancedBarriersSupported` check were found.
- The spec lists what legacy barriers cost: excessive sync latency, excessive
  flushes, expensive aliasing barriers, no self-copy. `host.rs` 7459 issues
  `aliasing` and `to_copy` barrier batches, the pattern the spec calls
  expensive. Enhanced barriers also allow precise sync scopes, which may
  narrow the waits that `findings.md` blames on UAV declarations over a
  174 MB view. That is a hypothesis to measure, not a claim.
- Needs the Agility SDK and a driver reporting
  `D3D12_FEATURE_DATA_D3D12_OPTIONS12::EnhancedBarriersSupported`; keep the
  legacy path as fallback.

## 4. Wave intrinsics, Wave32 and Wave64

Docs: [SM 6.0 wave features](https://learn.microsoft.com/en-us/windows/win32/direct3dhlsl/hlsl-shader-model-6-0-features-for-direct3d-12),
[SM 6.6 WaveSize](https://microsoft.github.io/DirectX-Specs/d3d/HLSL_SM_6_6_WaveSize.html),
[capability querying](https://learn.microsoft.com/en-us/windows/win32/direct3d12/capability-querying).

- `host.rs` near 988 reads `WaveLaneCountMin/Max` from OPTIONS1 and passes the
  max to the translator; NVIDIA is 32/32, so wave64 console programs are
  lowered as two 32-lane halves. That matches how the public docs say wave
  size is exposed. `findings.md` records 22 wave64 programs with no wave32
  lowering.
- Gap to look at: the WaveSize spec lets a shader request a wave size
  (`[WaveSize]`), but NVIDIA reports a single size, so it cannot select
  wave64 there. Anything in `victory-shader/src/emit/wave.rs` that assumes
  wave64 must therefore be emulated, which the layer already does.
- The same doc's rule on helper lanes and quad operations is the one behind
  the "square holes in shadows" fault in `findings.md` (neighbour reads in
  pixel shaders); worth re-reading before touching `emit` pixel paths.

## 5. DXR and acceleration structures

Doc: [DirectX Raytracing spec](https://microsoft.github.io/DirectX-Specs/d3d/Raytracing.html).

- The layer builds structures with its own BVH builder
  (`VICTORY_BVH_BUILDER=1`, `pipeline/make.rs` 131, `upload.rs` 107) and
  leaves out roughly 1,000 resolve shaders without it. The spec defines the
  host-side flow: `BuildRaytracingAccelerationStructure`, the
  `COMPACTED_SIZE` post-build info, and acceleration structures living in
  buffers in the `RAYTRACING_ACCELERATION_STRUCTURE` state.
- `findings.md` (RTX 5080 run) notes a compacted size arriving one frame after
  it was asked. The spec's post-build-info path is the documented way to get
  that size without a stale value; compare with `fork-ray-tracing.md`.
- Gap: the console's structure format is read by shaders through raw
  addresses (`memory.rs` header), which the public spec does not describe; the
  host format is opaque by design. This part stays reverse-engineered.
- No `RaytracingTier` (OPTIONS5) query was found in `victory-gpu/src`; if the
  builder is meant for tier 1.0 cards only, say so in the code.

## 6. Pipeline caching

Doc: [ID3D12PipelineLibrary](https://learn.microsoft.com/en-us/windows/win32/api/d3d12/nn-d3d12-id3d12pipelinelibrary).

- `pipeline/library.rs` already uses a pipeline library, names entries by
  hash, maps the file, and cites the doc's thread-safety statement. The doc
  also says a library from another adapter or driver is rejected, which the
  header already relies on. No gap; this is the best-aligned area.

## 7. Frame pacing and presentation

Docs: [DXGI 1.3 improvements](https://learn.microsoft.com/en-us/windows/win32/direct3ddxgi/dxgi-1-3-improvements)
(waitable swap chains),
[GetFrameLatencyWaitableObject](https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_3/nf-dxgi1_3-idxgiswapchain2-getframelatencywaitableobject),
[DXGI present](https://learn.microsoft.com/en-us/windows/win32/direct3ddxgi/dxgi-present).

- The swap chain is `FLIP_DISCARD` with `BUFFERS = 2` (`host.rs` 9620,
  10574-10575) and optional tearing; `Present(0, ...)` is called at 10516.
  No `FRAME_LATENCY_WAITABLE_OBJECT` flag, `SetMaximumFrameLatency` or
  `GetFrameLatencyWaitableObject` appears anywhere in the code.
- `frame.rs` paces the title with its own clock and says "event offsets are
  ignored ... tie the clock to presentation if a title paces by them". A
  waitable swap chain gives the host-side signal that comment asks for, and
  it bounds queued presents without a CPU settle. Pair it with the late
  present work (`late_free_present`, `host.rs` 2871).

## 8. GameInput and XGameRuntime

Docs: GameInput [overview](https://learn.microsoft.com/en-us/gaming/gdk/docs/features/common/input/overviews/input-overview),
[FAQ](https://learn.microsoft.com/en-us/gaming/gdk/docs/features/common/input/overviews/input-faq),
[NuGet and WinGet](https://learn.microsoft.com/en-us/gaming/gdk/docs/features/common/input/overviews/input-nuget).

- The docs say the same GameInput API runs on PC and console, shipped on PC
  through the `Microsoft.GameInput` NuGet package or `winget install
  Microsoft.GameInput`, on Windows 19H1 or later.
- `victory-hle/src/modules/gameinput.rs` reimplements the API over host
  XInput plus HID, and notes "trigger motors are dropped (XInput has none)"
  (near line 1046). The FAQ says GameInput exposes the Xbox One impulse
  triggers, so forwarding to the host GameInput runtime could restore them
  and remove the separate HID path for some pads. It would also address the
  synthetic-pad XInput 1167 fault recorded in `findings.md`.
- XGameRuntime: `modules/xgameruntime.rs` serves `QueryApiImpl` by
  (provider, interface) GUID pairs. Public GDK pages cover the documented
  runtime API surface (XUser, XStore and so on) but not the GUID pairs, so
  the table of areas stays reverse-engineered; use the public per-API pages
  only to check return codes and async behaviour.

## Suggested order

1. Waitable swap chain (small, isolated, directly named in `frame.rs`).
2. Enhanced Barriers behind a capability check (larger; measure on RTX first).
3. Fence settling and post-build size handling, if the pipeline work above moves.
4. Host GameInput forwarding for trigger motors.
