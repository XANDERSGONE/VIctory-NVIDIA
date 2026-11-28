# Combined test build, round 1

One build with every change we want tested, each behind its own switch.

**Downloads**
- Combined build: https://github.com/XANDERSGONE/VIctory-NVIDIA/releases/tag/test-build-20261128-ca271a6
- Compare against the latest official release: https://github.com/trustno1-coder/VIctory-NVIDIA/releases/latest

Put each `victory.exe` in its own folder. A new build restarts itself once on its first start (about 20 s); start timing after that. The first minute or two can sit near 14 fps while shaders warm up.

**What's in it:** lower RAM use on PCs under 24 GB, VRAM trims for the texture pool and target reserve, fewer wave64 barriers (helps when the GPU is the limit), the Windows BAR cap under 24 GB RAM, and no frame gen memory hold-back where frame gen can't run.

**The test (10+ minutes per build)**
1. Same save, same spot, same F1 settings for both builds. If you normally play with DLSS or frame gen on, keep them on.
2. Play or stand for 10+ minutes on the official release, then the same on the combined build.
3. At about 1, 5 and 10 minutes, note:
   - F3: fps, frame time, VRAM in use
   - Task Manager: victory.exe memory, and Performance > GPU dedicated and shared GPU memory
4. Note anything wrong: crash or 0x8007000E (device removed), flicker, dark or black tiles, loading stutter.

**Report:** GPU and VRAM, RAM, Resizable BAR on or off, settings, the numbers above for both builds, and anything from step 4. Attach the newest `victory-nvidia-play-...log` from `%LOCALAPPDATA%\victory\logs\` (and `crashes.log` if it crashed).

**If something breaks, find which change did it.** Run the combined build from a Command Prompt in its folder with one switch at a time, e.g. `victory.exe play --set VICTORY_BARRIER_TRIM=0`. If the problem goes away, that's the change.

| Switch | Turns off |
|---|---|
| `VICTORY_BARRIER_TRIM=0` | wave64 barrier trim (try first for flicker, dark tiles, broken shadows) |
| `VICTORY_VRAM_TRIM=0` | all VRAM trims (try first for blurry or missing textures, VRAM crashes) |
| `VICTORY_LOW_RAM=0` | the low-RAM defaults (allocator trim, fewer staging blocks) |
| `VICTORY_WINDOWS_BAR_MB=0` | the BAR cap on PCs under 24 GB RAM |
| `VICTORY_TEXTURE_ROOM_PERCENT=80` | gives back the old frame gen hold-back |

Finer VRAM switches, if `VICTORY_VRAM_TRIM=0` fixed it: `VICTORY_POOL_PACK`, `VICTORY_POOL_DRAIN`, `VICTORY_POOL_SPARE_SCALE`, `VICTORY_TARGETS_ROOM_SCALE`, `VICTORY_SMALL_CARD_LINE`, `VICTORY_TEXTURE_FLOOR_CHECK` (each `=0`). Finer RAM switches: `VICTORY_ALLOCATOR_TRIM=0`, `VICTORY_STAGING_KEEP=8`.
