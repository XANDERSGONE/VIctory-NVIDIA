# Combined test build, round 2

One build with round 1 plus the new changes. The new ones that change behavior are off unless you turn them on, so a plain start is round 1 plus a few always-on fixes.

**Downloads**
- Round 2 build: https://github.com/XANDERSGONE/VIctory-NVIDIA/releases/tag/test-build-20261128-cb246a9
- Round 1 build, to compare against: https://github.com/XANDERSGONE/VIctory-NVIDIA/releases/tag/test-build-20261128-ca271a6

Put each `victory.exe` in its own folder. A new build restarts itself once on its first start (about 20 to 30 s in game); start timing after that. The first minute or two can sit near 14 fps while shaders warm up.

**What's new since round 1**
- The Windows BAR cap now scales with the card: 1 GB on 8 GB cards and under, 3 GB on 12 GB and up, in between for 10 and 11 GB. It turns off with the other low-RAM changes (`VICTORY_LOW_RAM=0`).
- Always on: indirect draws are no longer cut off in normal play (should help flicker and missing water).
- Always on: the pipeline library saves less often during bursts of new shaders and while the PC is low on memory (fewer hitches entering new areas, mostly on 16 to 32 GB PCs).
- Always on, only on Series S without DLSS, frame gen or `VICTORY_FSR_PATH`: the game keeps its own upscaler instead of FSR 4 (about 28.5 to 30.9 fps measured upstream).
- Off until you turn them on: a separate thread for sending work to the card, one write-watch per memory reservation, a frame latency waitable swap chain (aimed at the 15 fps lock with frame gen off), and a RAM floor that stops Windows paging the game out.

**Test A: round 1 vs round 2, plain start (10+ minutes each)**
1. Same save, same spot, same F1 settings for both builds. If you normally play with DLSS or frame gen on, keep them on.
2. Play or stand for 10+ minutes on the round 1 build, then the same on round 2 with no switches.
3. At about 1, 5 and 10 minutes, note:
   - F3: fps, frame time, VRAM in use
   - Task Manager: victory.exe memory, and Performance > GPU dedicated and shared GPU memory
4. Note anything wrong: crash or 0x8007000E (device removed), flicker, dark or black tiles, missing water, loading stutter, hitches entering new areas.

**Test B: the new switches, one at a time, on the round 2 build (10+ minutes each)**

Run from a Command Prompt in the build's folder, one line per run, same save and spot as Test A:
1. `victory.exe play --set VICTORY_SEND_THREAD=1`
2. `victory.exe play --set VICTORY_SEND_THREAD=2`
3. `victory.exe play --set VICTORY_WATCH_RESERVATION=1`
4. `victory.exe play --set VICTORY_SEND_THREAD=1 --set VICTORY_WATCH_RESERVATION=1`
5. Frame gen OFF in F1: `victory.exe play --set VICTORY_FRAME_LATENCY=1`, then `=2`. Note if fps sits at exactly 15.0 (66.7 ms) and for how long, compared with the plain start.
6. Only if you have 32 GB or less and saw long slowdowns after alt-tabbing or with other programs open: `victory.exe play --set VICTORY_KEEP_RAM_GB=12` (16 GB PCs: try 8).

Note the same numbers as Test A for each run, plus any input lag that feels different.

**Report:** GPU and VRAM, RAM, Resizable BAR on or off, F1 settings, which runs you did and the numbers for each, and anything from Test A step 4. Attach the newest `victory-nvidia-play-...log` from `%LOCALAPPDATA%\victory\logs\` for each run (and `crashes.log` if it crashed). Useful log lines:
- `low-RAM defaults on: title memory windows ...` (the BAR cap size it chose, `mb=`)
- `preparing per frame, the submitting thread` and `the sender thread per frame` (Test B 1 to 4)
- `swap chain: at most N frames queued` and `frame latency wait (VICTORY_FRAME_LATENCY=N)` (Test B 5; `not used with a frame generation swap chain` means frame gen was on, redo with it off)
- `Memory: at least N GB of the game's memory is kept in RAM` or `Windows refused` (Test B 6)

**If something breaks on a plain start, find which change did it.** Run with one switch at a time, for example `victory.exe play --set VICTORY_BARRIER_TRIM=0`. If the problem goes away, that's the change.

| Switch | Default in this build | What it does |
|---|---|---|
| `VICTORY_BARRIER_TRIM` | on | wave64 barrier trim; `=0` turns it off (try first for flicker, dark tiles, broken shadows) |
| `VICTORY_VRAM_TRIM` | on | all VRAM trims; `=0` turns them off (try first for blurry or missing textures, VRAM crashes) |
| `VICTORY_POOL_PACK`, `VICTORY_POOL_DRAIN`, `VICTORY_POOL_SPARE_SCALE`, `VICTORY_TARGETS_ROOM_SCALE`, `VICTORY_SMALL_CARD_LINE`, `VICTORY_TEXTURE_FLOOR_CHECK` | on | the single VRAM trims; each `=0` turns one off, if `VICTORY_VRAM_TRIM=0` fixed it |
| `VICTORY_LOW_RAM` | on under 24 GB RAM | low-RAM defaults: allocator trim, fewer staging blocks, and the BAR cap; `=0` turns all off |
| `VICTORY_ALLOCATOR_TRIM` | on under 24 GB RAM | `=0` turns off only the allocator trim |
| `VICTORY_STAGING_KEEP` | 2 under 24 GB RAM, else 8 | empty staging blocks kept; `=8` gives back the old number |
| `VICTORY_WINDOWS_BAR_MB` | 1024 to 3072 by VRAM, under 24 GB RAM | BAR cap size; `=0` turns off only the cap |
| `VICTORY_TEXTURE_ROOM_PERCENT` | no frame gen hold-back where frame gen can't run | `=80` gives back the old hold-back |
| `VICTORY_SEND_THREAD` | 0 (off) | `=1` sender thread; `=2` also moves the heap wait to it |
| `VICTORY_WATCH_RESERVATION` | 0 (off) | `=1` one write-watch asking per reservation |
| `VICTORY_FRAME_LATENCY` | 0 (off) | `=1` to `16` frames queued; frame gen off only |
| `VICTORY_KEEP_RAM_GB` | unset (off) | keep at least this many GB of the game in RAM |
| `VICTORY_LIBRARY_SAVE_BUSY_S` | 180 | seconds between pipeline saves in a burst; `=20` gives back the old timing |
| `VICTORY_TITLE_PATCHES` | on (Series S, no DLSS, frame gen or FSR path) | `=0` lets the game use FSR 4 again on Series S |

Two always-on changes have no switch: the indirect list budget, and the 600 s wait between pipeline saves while the PC is low on memory. If a plain start breaks and no switch above fixes it, say so, because it's likely one of those two.
