# Blockers

What the emulator is still missing, per title, and who is on it. Updated every time a run finds a gap. Game dumps are complete and untouched; every entry here is an emulator gap.

Last update: 2026-11-21 (run 33: backdrop diag, 45 FPS lobby).

---

## Online policy (user decision, 2026-11-21): offline, private sessions only
- The user's words: "keep it offline, like its just going to be private lobbys and private games".
- No external traffic, ever, for every title. Content that exists only online (VOD, CDN art) gets placeholders served in-process.
- Matchmaking and ready-up land in private lobbies and private or custom matches, hosted by the in-process backend and the Party relay. Public queues are never used.
- Party, Meet and session work targets private sessions between local or LAN clients.
- Applied so far: a PlayFab matchmaking ticket resolves at once into a private match with the local tickets in its queue, and does not wait for strangers.
- **Scope change (user, 2026-11-21): single player only for now.** Multiplayer work is stopped:
  - The shared LAN backend (`online/lan.rs`) is built and tested, including a two-process loopback test. It is off by default (`lan=off`) and not being extended.
  - The PlayFab Party relay and PubSub are not emulated and are not being worked on.
  - Nadeo Meet (Trackmania) is not implemented and is not being worked on. Meet calls are logged as unhandled and get a Nadeo 404.
- Single-player online answers, all in-process: Xbox Live achievements, peoplehub and userpresence (`online/xbl_title.rs`), RTA, PlayFab login/QoS/profile/events, Ubi sessions, Nadeo Core and Nadeo Live services. Every request that is not answered is logged as `[online] unhandled [<title>] ...` in that title's `tombox_compat.log`.

## Fortnite (Xbox Series X|S)

### 🟨 Lobby 3D backdrop: now drawn (run 35), detail missing
- **Seen:** run 22-27. The scenery around the character never reaches the GPU. The character, floor and stage discs draw.
- **Cause found:** Fortnite culls far objects with GPU occlusion queries. It nests a dummy outer query around the per-object queries. Xbox allows that; PC D3D12 allows one active occlusion query per command list.
- **Done:** `query_x.rs` un-nests them. Run 27 confirmed the nesting: 245 un-nested queries.
- **Still missing:** every sampled query result is still 0, so everything still reads as occluded.
- **Run 33 diag:** the occlusion boxes (VS `f89dddcf70078d07`) reach clipping (CPrimitives 10-12 of 12), but PSInvocations is 0 and no samples pass. The viewport and scissor are full screen. Suspects: depth-bounds or depth/stencil state in the PSO translation, or sample mask 0. The box PSO is not registered in the diag, so its creation path is not hooked.

### 🟨 Lobby hologram character / slow login
- **Seen:** the character shows as a cyan hologram, which means default profile/loadout data. Login→lobby takes about 29 s after the splash screen (QueryProfile 8.6 s).
- **Missing:** a full athena profile loadout in the in-process services, and connection reuse.

### 🟨 Performance
- **Run 33 lobby:** 45.2 FPS average (the console runs it at 60), with 1-2.5 s hitches. Frontend LoadMap takes 24.5 s.
- **Missing:** console frame pacing (SetFrameIntervalX / ScheduleFrameEventX), title core affinity, and hitch sources.
- **Translated shader cost (offline, 336 run35 shaders):**
  - **Cause:** every lane-masked VALU write became `if (exec_lo & 1u) v = ...;`. That produced 47,923 branches and 40,658 phi nodes in DXIL. The worst passes were the deferred-lighting PS `33a16d2f`, `a06ce28f` and `f3bc163c`, and the CS `4e9db7ab` and `eb81f3b1`, at 6-7x the ISA instruction count.
  - **Fix:** pure ALU writes now use `select()`. This gives the same result for active and inactive lanes. Wave, quad, derivative, sample and memory ops keep the branch.
  - **Result:** branches 47,923 to 6,668, phi 40,658 to 20,302, DXIL 460,604 to 400,850 instructions, no new compile failures. The top pixel shader went from 7.0x to 5.0x the ISA, with 3,908 branches down to 224.
  - **Still to do:** GPU ms per pass in a live run (`TOMBOX_GPUPROF=1`, A/B with `TOMBOX_XLAT_GUARD_BRANCH=1`).
  - **Not used:** no 16-bit packed math (`v_pk_*`) appears in these shaders, so there is no 16-bit gap to recover. SampleGrad appears in only 1 shader (in-loop implicit samples).
- **GPU hitches are waits, not shader cost:**
  - **Run 40:** 145 HITCHHUNTER GPU hitches, 70-1,250 ms each, about 2 per second through the lobby. Many are on frames of only 30-50 draws.
  - **Run 35 was worse:** up to 10.5 s on frames of 39-52 draws.
  - **Likely cause:** that much GPU time on so few draws means the timed interval includes idle or wait time (cross-queue fence waits, late submits) or a timestamp problem.
  - **Run 42 (gpuprof live) confirms waits:**
    - **Normal windows:** summed GPU 6.2-8.9 ms/frame against a wall span 0.7-1.0 ms/frame larger.
    - **Window 2:** the only one that overlaps hitches (f744-f784). It lasted 1540 ms for 16 frames, but GPU timestamps cover only 374 ms of that, and only 129 ms of it is GPU work. No single in-list interval was over 100 ms. So the GPU sat idle with nothing submitted.
    - **47 "Hitch in GPU", 20.6 s total.** 12 of them (6.7 s) line up with shader-translation waits ("caller thread blocked N ms") plus driver CreateGraphicsPipelineState time in the same frame. UE agrees: STAT_PSOGraphicsFindOrCreateTime appears 14 times (7.9 s).
    - **The rest have no emulator log line.** The worst are f689 (2.86 s) and f692 (2.61 s): IO and prewarm are idle and d3d12_xs logs only perf. The UE scopes there are TaskGraphThreadNP STAT_ParallelChainTranslate (2.79 s, the RHI recording into our command-list wrappers), PoolThread MipCalc (2.2 s) and GameThread FAudioCommandFence_Wait (2.37 s).
    - **Open:** a CPU-side stall inside a wrapped call or a process-wide stall. Candidates are a synchronous GPU wait while holding a lock (vmem_x D3D12ZeroMemoryX waits on its fence with `GPU` held) or the upload pump (max 60 ms per present). It needs per-call slow-call timing on the command-list and queue wrappers.
  - **Fixed:** gpuprof labelled every row 0. `shader_xlat::ORIGIN` is only filled when rtcap or hot_reload is on. There is now a hash-only map that is always filled (`note_origin_hash` / `origin_hash`), and gpuprof uses it.
- **Anaconda (XSX_Anaconda profile) black scene, offline checks:**
  - 62 shaders appear only in Anaconda runs (37-39). All translate and compile.
  - No PSO failures (pso_fail=0, untranslatable=0).
  - None of the new compute shaders uses LDS or 16-bit packed math.
  - So it is not a translation failure. It needs an RT capture.
  - Occlusion results: about 97% are zero in runs 39 and 40, against 17% in run 35. The early proxy-box diagnostics are identical, so this may reflect the scene rather than a query bug.
- **After that:** the older build drew the backdrop black (PS `31dd3b33133e9ca3`). Its material constant buffers (cb1/cb2) are bound as root CBVs, and the trace does not capture those yet.

### 🟨 Flashing rectangle (bloom inf) - FIXED
- **Seen:** a top-left cream block, about 46% wide and 31% tall.
- **Cause:** infinite values in the R11G11B10F bloom chain.
- **Done:** run 27 clamps overflowing UAV stores toward zero, as AMD hardware does. Very bright stage-light texels no longer reach large inf counts.
- **Root cause (fixed in target-shade6, verified in run 35):**
  - The TAA-like compute shaders 1e8bbd9b and 251ed6ba share LDS with no `s_barrier`, which is only safe under wave64 lockstep. Per-lane HLSL raced, so the history exploded (up to 57120), went inf, and the bloom spread it into a block.
  - The translator now inserts `GroupMemoryBarrierWithGroupSync` at LDS write→read and read→write hazards.
  - Run 35 captures 10/11/12: inf=0 in every target, the 3200x1800 history peaks at 0.43, and the 1600x900 first stage peaks at 488-536 (was 1000+).

### 🟨 Audio
- **Done:**
  - The XMA2 decoder (crate `xma`, ported from FFmpeg wmaprodec) is within 1 LSB of ffmpeg. Hardware mode keeps frame 0, so output is 512 × frames.
  - The Opus decoder (crate `opusdec`, ported from libopus) is within 1 LSB in all modes.
  - AcpHal, SHAPE, xapu and chad run on the CPU.
  - xapu is asynchronous, like the console. The console's `XApuEnqueueCommand` only copies the command into a shared submit ring (xapu.dll sub_37b0). `XApuGetResultCount` reads a completion ring the device fills. Our version now queues each command to a host worker pool (cores minus one, at most 8). Per-stream order is kept, results commit in submission order, and the result event fires on commit. `TOMBOX_XAPU_SYNC=1` decodes on the calling thread instead. Async and sync produce identical results and PCM (test `async_results_commit_in_submission_order`).
  - Fortnite "enables with input but no output" are resolved. With the recorder race fixed, a fresh harness recording shows 0 of 60104 enables (it was 109, then 362). All of them were recorder artifacts, not a real end of stream and not a SHAPE bug.
  - Record/replay: `TOMBOX_AUDIO_RECORD=1` writes `%LOCALAPPDATA%\TomBox\audio-rec\*.tbrec`, or the folder in `TOMBOX_AUDIO_RECORD_DIR`. `target-audio\release\audio-replay.exe <file>` writes WAVs plus a stall report.
- **Decoder cost (release build, one stream):**
  - XMA2: 8-15 us per 512-sample frame, 945x real time. It was 299x before this round of optimisation.
  - Opus: 6-39 us per packet for CELT, SILK and hybrid; 143 us for 60 ms SILK stereo. That is 420x to 1780x real time.
  - xapu data command: 41 us mean for hybrid stereo, of which 39 us is the decode.
  - At console maximum concurrency the load nears one core: 512 stereo XMA contexts take about 70%; 500 Opus hybrid streams take about 97%.
- **Missing:**
  - SHAPE EQ, FILTVOL and PCM contexts: layouts unknown, and no title on disk drives them.
  - SRC paired with PCM contexts.
  - XMA loops are packet-granular, not sample-granular.
  - xapu codecs 1/3 and chad kinds 2/3 return E_NOTIMPL, because no title issues them.
  - SHAPE command types outside the known set return E_NOTIMPL.
  - XMA output ring fills to c-1 blocks and never clears d1 bit31. On the console it fills all c blocks and clears OutputBufferValid when full: Fortnite 0x1406f2fce takes the cold path at 0x14262da2a, reading r==w with bit31 clear as a full ring. It is left as is because how the hardware sets bit31 again after a drain is unknown. No title on disk fills the ring (Fortnite asks for at most 8 of 31 blocks and drains every pass; Wwise pulls one block at a time). XmaState::service and pull_frame should change together once a title or a console trace shows the re-validate rule.
  - Some xapu result fields are guessed: +0x1C, +0x1E, +0x22.
- **Recording caveats:**
  - EQ/FILTVOL arrays are not captured.
  - Fixed: a title write that landed while a SHAPE op was running could be absorbed into the shadow and missed. The engine's `wr`/`set_bits` now log what they write during recording, and the shadow takes only those bits.
  - Recording costs about 8 us per op. The Fortnite harness takes 1.5-2.1 s with recording, against 0.6 s without.
  - Fixed: the XDefiant replay mismatch. The packet cache was not cleared when APU memory was freed. Same-build record and replay now match for all three titles.

### 🟨 System functions: 33 of 671 unfinished
- **xmem:** XMemTransferMemory, XMemFreePhysicalPages.
- **d3d12_xs:** D3D12XboxSetProcessDebugFlags.
- **XAudio2_9.dll:** ordinals #1, #2, #4, #5, #6.
- **MFPlat.DLL:** 16 functions. These now forward through the compat mfplat (233 forwarders, decoders bind).
  - Run 35 evidence (`media.json`, from `runs
un35	ombox_compat.log`): all 16 import slots plus the 2 MFReadWrite slots are bound in-game to the PC modules. Nothing failed.
  - **Missing:** in-game calls. Run 35 called only MFStartup (x3), MFCreateDXGIDeviceManager, MFResetDXGIDeviceManagerX and MFCreateAttributes. No source reader was opened, so 14 functions have no in-game call yet:
    - The `Movies/Bifrost/Comp 1-6.mp4` files are missing from the dump ("Failed to validate media source").
    - `ShouldShowVideo` returned false for SeasonTrailer.
- **MFReadWrite.dll:** 2 source-reader functions.
- **AcpHal:** 6 functions.
- **GameChat2.dll:** Fortnite's own DLL runs on PC; chatprobe passes all 14 imports in two peers, plus the opus/SILK codec tone round trip.
  - **Missing:** the emulated `XSystemGetAnalyticsInfo` family is `Xbox`. A real console returns `Windows.Xbox`, which puts GameChat2 in console mode (mode 2). With the family set to `Xbox`, GameChat2 never loads AudioHALWrapper and makes no Xbox Live calls. The GDK agent owns the switch in `gdk/system.rs` and the `gdk_runtime` test.
  - **Done (console path, proven with a temporary flip):**
    - `chatprobe --console` passes: AudioHALWrapper loads in both peers, and the RTA websocket connects. The mutelist and neverlist subscriptions are acked and later unsubscribed. 5 of 5 Xbox Live requests get JSON back, with nothing unhandled and no crash.
    - On the console path GameChat2 also loads the system `Windows.Media.MediaControl.dll`. It calls `CreateRenderAudioStateMonitorForCategory(8 = GameChat)`, then `IAudioStateMonitor::RegisterCallback` and `GetSoundLevel`.
      - `gdk/mediacontrol.rs` routes all 8 `Create*AudioStateMonitor*` exports through GetProcAddress and logs the category, apartment and sound level.
      - It holds the process MTA only when the caller has no COM apartment. The PC API's RegisterCallback fails with CO_E_NOTINITIALIZED there, but works on the console.
      - In the console run, the caller was already MTA (a GameChat2 thread) and the sound level was Full, as for a focused console title.
  - **Fixed:** the console-path crash at `GameChat2+0x52153`. The cause was an empty `XUserGetTokenAndSignature` signature: its UTF-8 to UTF-16 helper reads `[s + len - 1]`, which is out of bounds at length 0. Tokens now carry a real XSTS v1 signature: version, FILETIME and an ECDSA P-256 signature over method, path, token and body, from `gdk/xsign.rs`.

### 🟨 Online
- **Missing:** XMPP needs wss (the game refuses ws://).
- **Done:** an RTA V2 websocket (`online/rta.rs`, subprotocol `rta.xboxlive.com.V2`). It supports:
  - subscribe and unsubscribe acks;
  - `ConnectionId` data for sessiondirectory connections;
  - mutelist and neverlist data, plus `Added`/`Removed` events when privacy mute/avoid changes.
- **Done:** GameChat2's console requests in `online/xboxlive.rs`:
  - social `users/me/people` with paging;
  - privacy `permission/validate`, which now accepts capitalized keys;
  - `mutelist` and `neverlist`;
  - the profile `SpeechAccessibility` setting;
  - the userstats reputation stats.
- **Done:** an Xbox Live route that answers 200 with an empty body is now turned into a logged JSON 500.
- **Fixed:** net_guard dropped the websocket handle's HANDLE_CREATED callback, which WinHTTP sends synchronously inside `WebSocketCompleteUpgrade`, before the handle's parent is recorded. Handles created during the upgrade are now adopted on that thread; test: `tests/net_ws_async_rta.rs`. The console chatprobe run has no drops.
- **Done:** the cutscene video stream is emulated in-process.

### ✅ Resolved this session
- Cutscene blocks the lobby: fixed in run 26. The video plays and ends; mfplat now forwards every PC export, so the H.264/AAC decoders bind.
- GDK layer crash in EOS: fixed. Wrong argument order in the XNetworking slot.
- Frame-0 device removed (runs 24-25): fixed. Textures had been given a nonzero `GetGPUVirtualAddress`; it is now 0 as on PC.

### 🟥 Freezes when navigating menus
- **Seen:** run 35. Opening the item shop and other screens freezes the game. The log shows GPU hitches up to 1060 ms at about 329 draws, and game-thread hitches of about 305 ms. The lobby averaged 15.4 FPS while the user navigated and two 12-frame captures ran.
- **Suspects:** shader translation, compile and PSO creation run synchronously on first use (the console uses precompiled PSOs), waits on GPU sync points, and file IO for streaming.

### 🟨 3D scenes low detail
- **Seen:** run 35. Lobby structure surfaces are flat and untextured, the big screens are blank, and the challenge holotable map is a smeared blob.
- **Cause narrowed (run 35 capture 10):** texture residency, not shader translation.
  - GBufferC (base colour) and GBufferA (normals) show full detail on characters, but one flat value per environment object.
  - Material PS sampling checks out against the ISA (SampleBias operands correct). 257 PSOs translated, 0 failures.
  - The log has 400+ "upload unreadable placed texture" lines (the log cap) covering 179+ BC1/BC3/BC5 textures. Their mips 0-3 sit in MEM_RESERVE pages, and only the first 64 KB (low mips and mip tail) is committed. The title never commits or loads the high mips.
- **Found and fixed (target-shade7 xmem.dll): wrong memory stats.**
  - The game's FPlatformMemory reads XMemGetWorkingSetStatistics (flags 0x10000001) as +0 = title limit and +8 = title USED bytes. This is confirmed at five call sites, e.g. GetStats at 0x140964b14.
  - xmem returned PC total RAM at +0 and PC FREE RAM at +8. Run 36 logged "AppInit PhysicalAllocated 12459MB, PhysicalFree 20121MB": 12.4 GB "used" at boot, against a 13.5 GiB console budget, and falling as the game allocated.
  - It now reports a 13.5 GiB limit, with used = process private bytes + live physical pages.
  - To verify on the next run: the AppInit line shows small PhysicalAllocated, and the partly-unmapped texture count drops.
- **Missing:** find why texture streaming does not stream high mips in. Candidates are IoDispatcher bulk reads, the xmem commit/map of the reserved texture VA, and the streamer's budget/visibility inputs. The backdrop agent is on it.
- **Armed for next run:** `%TEMP%\tombox_trace_shaders.txt` lists base-pass PS hashes, so the next rtcap run dumps their SRV MostDetailedMip/MinLODClamp and placed mip counts.
- **Fixed alongside (target-shade7):** implicit-derivative samples inside loops were forced to LOD 0. They now use per-quad gradients (QuadReadLaneAt + SampleGrad, bias applied as a 2^bias scale), which matches the console.

### 🟨 Login with emulated XAsync: fixed on disk, confirming in run 37
- **Seen:** run 34. Login aborts at ConsolePrivileges when the emulated XAsync is the default. Run 35 (real XAsync) passes, with QueryProfile at 0.98 s (was 8.6 s).
- **Cause:** closing or terminating one composite queue cancelled the ports it shares with other composites, so later async work on them ended with E_ABORT (`XStoreQueryGameAndDlcPackageUpdatesResultCount failed with 0x80004004`). Fixed in `gdk/rt.rs`: each queue has its own terminate state, and ports count their users. `tests/gdk_multi_dll.rs` now runs the privilege + XStore update sequence on composite queues.
- **Also fixed:** one signed-in user got two different XUser handles across DLLs. The GDK owner now publishes one handle process-wide (`TOMBOX_GDK_USER_HANDLE`), and XUserCompare compares user identity. `xhost game_load_order` passes.

### ✅ Fortnite language paks 2/5/7/8/9 not mounting is correct console behaviour
- Layout: pakchunk2/5/7/8/9 are chunks 4/7/9/10/11 = de-DE, fr-FR, pl-PL, ru-RU, zh-CN. Fortnite skips them itself ("Locale not current", the en-US locale) before asking XPackage. No XPackage answer is involved.
- XPackage now reports real installed sizes from the layout and resolves language/tag/chunk/feature selectors. Selectors that match no chunk report Unavailable, and unknown package ids return E_NOT_FOUND. Test: `tests/gdk_package.rs`.

### 🟨 XGameSaveFiles: no title uses it
- None of Fortnite, XDefiant, Terraria or Trackmania calls XGameSaveFiles; their call sites only cover XGameSave. Its interface GUIDs are not in any dump, so it is not implemented. Add it when a title needs it.

### 🟨 Stale DLLs install a second, older GDK runtime
- **Seen:** 11-21 Terraria and Trackmania runs with `d3d12_x`/`xg_x`/`dxcompiler_x`/`xfrontpaneldisplay` from `target-xone
elease` (05:03-05:26). Their compat-common predates the single-owner claim, so they install a second runtime ("26 interfaces") after the owner. In Terraria that copy answered QueryApiImpl: the real XAsync and a placeholder XUser token.
- **Fix:** build every staged DLL from one tree and target. With all DLLs from `target-gdk
elease`, the log shows one install and every interface is emulated.

### 🟨 Match hosting (Workstream B): hooks on disk, no run yet
- **Built (11-21):** `os_host/fortnite_match.rs`. A 7-byte StartGameInstance patch makes `-REPLAY=` and a leading map URL on the command line work. In Play mode, a `UEngine::Browse` vtable hook (3 engine vtables) turns travel to the sentinel game server into a local `Apollo_Terrain?game=Athena_GameMode_C` load. All RVAs are checked against the dump by the `dump_constants_match` unit test. Launch with `run_fortnite.ps1 -Match Replay|Map|Play`; see `emulator\docs\MATCH_TEST.md`.
- **Unproven:** whether the patched StartGameInstance path plays BR_FortBench (replay version check); whether the client really reaches `Browse` with the sentinel URL, or stops at a reservation beacon (BEACONPORT_i 15009) first.
- **Expected next gates once the map loads (each needs its own hook file and run evidence):**
  - ReadyToStartMatch waits on a playlist (CurrentPlaylistInfo / FortPlaylistManager in a standalone game). The `Playlist=` URL option may not be read by this build.
  - MCP athena profile loading on the Map path, which skips the frontend login. The Play path keeps the login.
  - Pawn spawn ("AFortPlayerController::OnReadyToStartMatch: Marking player ... ready to start").
- **Outside B:** Apollo_Terrain is multi-GB. With xcrdapi on one worker the loading screen may sit for minutes. That is throughput, not a hook failure.

---

## Terraria (Xbox One, `d3d12_x`)

### 🟨 Runs 60 s without crashing, no frame presented yet
- **Fixed:**
  - `IDXGIDevice1` on the D3D12 device (QI, GetParent → the host adapter, private data, GPU thread priority, frame latency).
  - Xbox One queue descs (type 0x10 DMA → COPY, priority 1 → HIGH).
  - Descriptor heap type 4 → CBV_SRV_UAV. This fixed the AV at `EXE+0x7cbd32`.
  - DMA command allocators and lists (type 16).
  - `xg_x` is now the real GCN XG model. Tile modes, depth/stencil validation and texture layouts are byte-identical to the console `xg_x.dll` on about 1700 random cases. Before this, texture sizes came out as 0.
- **Missing:**
  - After `XGCreateTexture2DComputer` (2048x2048 fmt 65 mode 13), the log has no `PresentX`.
  - Found 11-21: the cause is texture GPU VA = 0 (see below). Frame events are not it: `SetFrameIntervalX` / `ScheduleFrameEventX` (types 0 and 2) arrive and the frame clock runs. It may be a fence wait (DMA/copy queue), or the upload path for Xbox One placed textures (`upload.rs` uses `xg_xs`, which Terraria does not load).
  - One run in two crashed after 6 s: an AV in ntdll on a networking/PlayFab Party thread, after a C++ exception.
  - 11-21 GDK sweep (40 s, one DLL set from target-gdk, d3d12_xs from target-xone): no E_NOTIMPL or unhandled GDK calls. Title view: Series X (device type 7), 7 cores, mask 0x7f. Still no `PresentX` in 40 s.
  - 11-21 thread dump at 30 s (`MDBG_HANGDUMP=30`): no title thread is blocked in a GDK call. The only GDK frame is Terraria's XTaskQueue dispatcher thread idling in `XTaskQueueDispatch` (manual queue, timed wait), which is normal. No XGameUi calls. The last GDK call is at about +17 s, the gamepad is never polled, and XGameSave is not initialized yet.
  - The IL2CPP game-logic thread waits in the kernel, called from `GameAssembly.dll +0x17f8ce`. Callers: +0x155b9a, +0x1b3dfc6, +0x1c70da, +0x1b3d409, +0x1361954, +0x135e60b, +0x1beaf5e, +0x1923c6, +0x168691. Unity threads wait at `Terraria.exe +0x55e2cf / +0x522eb5`. (That thread turned out to be the `CacheFileManager::FileUpload` worker, not the main thread.)
  - **Cause of the first-frame hang (found 11-21, the fix belongs to the Xbox One D3D layer, `d3d12_xs/src/gpu_va.rs`):**
    - A proper unwind of the main thread shows the C# path `XNAUnityRunner::Start` → `Main::Initialize` → `ClientInitialize` → `LoadSettings` → `GUIMap::set_ActiveFrameStyle` → `GUIMap::LoadContent` → `ContentManager::LoadTexture2D` → `Texture2D::get_UnityTexture` → `TextureAtlasDB::AltlasTexture`. Unity then waits forever in `Terraria.exe +0x4873c0` (the sync wait for an async texture upload): `WaitOnAddress` loop until `texture+0xe9` is set.
    - `+0x442a60` starts that upload. It asks the texture-memory object (`[+0x1892a88]`, vtable slot 0x598 = `+0x7f6b30`) for a CPU pointer to the texture's memory. That calls `ID3D12Resource::GetGPUVirtualAddress` (vtable slot 11) and uses the result as a CPU address, because on the console the GPU VA and the CPU VA are the same unified memory.
    - Our `get_gpu_va_x` returns 0 for committed textures unless `TOMBOX_TEXTURE_VA=1`. With 0, Unity takes the error branch at `+0x442bbe`, the "async texture load: ... (unable to get texture memory)" path. That message never appears in our logs. Unity drops the request there and never sets the completion flag. Live memory at 30 s matches: texture `e8=1 e9=0`, AsyncUploadManager idle (32 free slots, nothing pending), render thread and AsyncRead thread idle.
    - Check: the same run with `TOMBOX_TEXTURE_VA=1` gets past the hang (CopyTextureRegionX, PSO creation, shader_xlat). It then dies at 8.5 s. Two upload-heap buffers fail `CreateCommittedResource` (E_INVALIDARG; heap type 2, state 0x8c3, flags 0x8 deny-SRV, 256x1 and 52x1), and two VS shaders fail with "inflated N bytes but magic=[1,0,0,0]". Unity then logs "D3D12: Removing Device" and an AV follows at `EXE+0x7e7048`.
    - Needed: texture VA on by default, with CPU-visible memory per texture that is synced into the real texture before the GPU uses it.
      - Unity writes the texels there in the tiled layout that `xg_x` reported for the texture (mode 13), so the sync must detile with that layout. It cannot copy the bytes as linear.
      - Today `back_committed_texture` memory never reaches the real texture.
    - Needed: translate upload-heap creation in `CreateCommittedResource`. On PC an upload heap (type 2) must start in `GENERIC_READ` (0xAC3); Terraria passes 0x8c3, which has no `COPY_SOURCE`. `DENY_SHADER_RESOURCE` (0x8) is only valid together with `ALLOW_DEPTH_STENCIL`.
    - Needed: handle that shader container form.
    - No GDK call is involved.
    - 11-21 fixes on disk (d3d12_xs, target-xone, not yet run):
      - Texture VA is on by default for Xbox One titles (when `d3d12_x.dll` is loaded; `TOMBOX_TEXTURE_VA=0` turns it off). The backing is sized to at least the `xg_x` layout size.
      - `upload.rs` builds layouts from `xg_x.dll` when `xg_xs.dll` is absent, with the GCN layout's bytes-per-element at 0x38. So committed backings registered in `placed` are detiled with the mode-13/14 element offsets of `xg_x`'s computer (slot 6).
      - `CreateCommittedResource`: an upload heap starts in GENERIC_READ (0xAC3), a readback heap in COPY_DEST, and DENY_SHADER_RESOURCE without ALLOW_DEPTH_STENCIL is dropped.
      - VS containers: the `magic=[1,0,0,0]` blob is a 144/184-byte variant table. The DXIL part holds three Durango variants (header word 0x20 = 4/2/0). The extractor now takes the variant that exports a position.
      - Those two VS still fail: every variant starts with `s_swappc_b64 s[0:1], s[0:1]`, a call to a fetch shader that the console runtime builds from the input layout. Needed: synthesize the attribute fetch (VGPR assignment from the Durango input metadata).
  - Online: `[online] unhandled POST /MultiplayerServer/ListPartyQosServers` (PlayFab Party QoS, `40467.playfabapi.com`). The run also fell back to the Fortnite service module, because Terraria had no title module.
    - Now on disk (`online/playfab`, not yet tried in a run): a Terraria/PlayFab module picked by `Terraria.exe`, and `*.playfabapi.com` is routed for every title.
    - It covers every PlayFab route found in `PartyWin.dll`, `PartyXboxLive.dll` and `PlayFabMultiplayerGDK.dll`: LoginWithXbox, ListPartyQosServers, GetCognitiveServicesToken, Party/RequestParty, Event/WriteEvents and WriteTelemetryEvents, Profile/GetTitlePlayersFromXboxLiveIDs, and all 20 Lobby/Match routes. It also covers the common Client/Authentication/Profile calls.
    - A UDP QoS responder on 127.0.0.1:3075 answers the 0xFFFF probe. The 7:14 run logged bind error 10048 on 3075, most likely because a unit test held the port at the same time. Unit tests now use port 0 (`TOMBOX_PLAYFAB_QOS_PORT`, cfg `playfab_qos_port`), and the responder retries the bind on the next QoS call.
    - Single-player world load: the 7:14 run made only the ListPartyQosServers call and then crashed in rendering (AV at `Terraria.exe+0x7e7048`). That crash is not an online gap.
    - Paused (multiplayer, out of scope): the Party relay. RequestParty gives out 127.0.0.1:30000, but the relay's DTLS protocol is not emulated, so CreateNewNetwork/ConnectToNetwork will fail after the REST calls.
    - Missing: PlayFab PubSub (SignalR websocket). Lobby and matchmaking changes are not pushed, and a title that waits for a push will stall.
    - Unknown: which of the ~295 Unity-SDK PlayFab calls in `global-metadata.dat` Terraria actually makes. Anything not implemented is logged as `[playfab]` unhandled.
- **Dump:** complete. All 19 layout file groups are present.

---

## Trackmania (Xbox One, `d3d12_x`)

### 🟨 Presents frames (60 fps at start, 34 fps average over 40 s); shaders not translated yet
- **11-21 GDK sweep (40 s, run_xone):** reaches the main menu at 55-60 fps (p99 17-50 ms). No E_NOTIMPL or unhandled GDK calls. Title view: Series X (device type 7), 7 cores, mask 0x7f.
- **Fixed:**
  - Packs on `G:\` and the Series device type (GDK layer).
  - `Close 0x80070057` is gone. The cause was a ResourceBarrier `RENDER_TARGET -> 0x4000c0`.
    - On Xbox the extra bit 0x400000 is combined with the shader-resource states. On PC that bit is `RAYTRACING_ACCELERATION_STRUCTURE`, which is valid only on its own.
    - `xone_barrier.rs` now strips the bit when it is combined with other states.
    - The device is no longer removed, and PresentX shows frames in the title window.
  - The bad command was found with `TOMBOX_CMDLIST_SPY=1` (`cmd_spy.rs`). It logs the last commands of a list, decodes their barriers, and prints them when Close fails.
- **Missing:**
  - 42 Xbox One (GCN/Durango) shaders.
    - `tombox-xshader/src/gcn.rs` parses the Durango blob and re-encodes gfx7 ISA as RDNA2 words.
      - 122/123 dumped shaders lower with no unknown opcodes.
      - The remaining one uses `s_getpc` (PC-relative code), which is not relocated yet.
    - HLSL translation: 0/123. The Durango user-SGPR binding model is not decoded yet.
      - Example: the CB pointer is in `s[3:4]` and a 32-bit table pointer in `s5`, while the RDNA2 path expects root tables at s8/s4.
      - The mapping is in the blob header and the tail after `code_end`.
    - Program range solved: the program starts at `code_start + 0x28` (after a 10-dword prefix) and its byte length is the u32 at header `0x1dc`. Data follows it. All 123 blobs decode exactly to a final `s_endpgm`.
    - MIMG `dim` is taken from the DA bit (2D or 2D array) until it can come from the resource info.
  - D3D12 debug layer: `d3d12SDKLayers.dll` (Agility 1.620.2) cannot be enabled.
    - `SetSDKVersion` needs Developer Mode.
    - `CreateDeviceFactory(620)` works, and the device is created through it, but the factory reports the SDK layers as not installed.
- **11-21 GDK run (run_xone, overlay `run-xone-gdk`, all DLLs from `target-gdk\release`):** 115-129 fps (the d3d12_xs frame clock currently follows the host's 144 Hz; `TOMBOX_FRAME_HZ=console` restores the console interval). It crashed at 42.7 s with an AV write at 0x8 in `GbxGame+0x153017d`. The call came from libHttpClient's HCCleanupAsync termination callback `+0x1542270`, running on a thread spawned by our `XTaskQueueTerminate`.
  - Cause (GDK layer, `gdk/rt.rs`), two bugs:
    - Our XAsync passed the provider a stack `XAsyncProviderData`. libHttpClient keeps that pointer and reads `[data+0x18]` (the context) later.
    - `XTaskQueueTerminate(wait=false)` fired the callback at once on a new thread. It should run after the pending items, through the work port and then the completion port; on manual queues it should run during dispatch.
  - Fix on disk: a persistent ProviderData in each async state, and the termination is delivered through `submit_marker` on the work and completion ports. Test: `tests/gdk_rt_terminate.rs`. Not re-run in game: in-game QA is paused until the progress map reaches 100%.
- **Dump:** complete. All 53 layout file groups are present, including `Packs\*.pak`.
- **Online (in-process, `online/trackmania`), not yet tried in a run:**
  - Done: Ubi session through the XDefiant module; Nadeo token exchange, refresh and nadeoservices; `/api/routes`; all 115 Core routes, stateful.
  - Done (`online/trackmania/live.rs`): Live services (`live-services.trackmania.nadeo.live`), with the NadeoLiveServices token check. Covered:
    - official campaigns (v1 and v2), Track of the Day months, cup of the week, weekly grands and shorts, map campaign info, and maniapubs;
    - map info, get-multiple, uploaded maps, and favourites (add, remove, list);
    - leaderboards: player records, position for a score, top, surround, medals, campaign ranking and top, and trophies. These are computed from the Core map records, so a solo personal best shows up at once;
    - clubs, stored locally: create, edit, delete, mine, search, members, join, pin, tag, activities, news, folders, and club campaigns.
  - Not handled, logged as unhandled: club rooms, competitions, map review, player-server accounts and club leaderboards. These are multiplayer features.
  - Meet (`meet.trackmania.nadeo.club`): not implemented. Multiplayer is out of scope.
  - Missing: real official campaign, Track of the Day and weekly data. Lists come from cfg `trackmania_live_data` or `<data_root>\trackmania\live.json`; without that file they are empty, and the log says so.
    - Campaign maps also need Core map entries. A `"maps"` array in the same file (`{mapUid, name, authorScore, goldScore, silverScore, bronzeScore, file}`, with `file` relative to the JSON) registers each map in Core at startup, along with its file under `storageObjects`. Then `/maps/?mapUidList=`, the downloads and the Live map info resolve.
    - Where the official map files are has not been found yet. A plain-text search of `Trackmania.Title.Pack.Gbx`, `Titles.pak` and `Stadium.pak` turned up nothing, but those packs are compressed or encrypted.
  - Meet's main-menu polls (for example the current cup of the day) get a logged Nadeo 404 where the real service may answer 204. The first Trackmania run that reaches online will show whether that matters.
  - The 6:27 run reached no online calls; it stopped before the online stage.

## XDefiant rendering and startup (parked 2026-11-21)
- **Current state:** runs about 140 s at 60 fps. All graphics PSOs build, and command lists close cleanly after the aliasing-barrier fixes (`xone_barrier.rs`, `placed::is_committed_placement`). The presented frame is still all black. Captures at frames 1800 and 6900 show only compute dispatches and one ExecuteIndirect draw into a 960x540 target, and every target reads zero.
- **Aliased-VA render targets:**
  - The title creates pairs of `CreatePlacedResourceX` targets at the same Xbox VA (e.g. 0x3e2400000, 0x3e13f0000).
  - On console they share memory. Here each one is a separate committed resource, so a write through one alias never appears in the other.
  - Needs a real shared backing for same-VA placements: placed resources in one heap per VA range, or aliasing through a reserved resource.
- **`rta.xboxlive.com` websocket:**
  - The title retries `wss://rta.xboxlive.com/connect` every 10-25 s and never gets past startup.
  - RTA (real-time activity) must be emulated in-process: a websocket upgrade plus the RTA subscribe protocol.
  - Never make a real connection.
- **Texture-pool crash at `Rogue_scarlett_rtx.exe+0x67a8f9`:**
  - A NULL write about 140 s in (around frame 8400).
  - `rbx=0x80070057` and a nearby string `pool/rgba_float16 120x68x64/268` suggest a resource or view creation for a 120x68x64 R16G16B16A16_FLOAT 3D texture returned E_INVALIDARG. The title does not handle the failure.
  - `d3d12_xs` logged no failing call, so the failing method is an unlogged forwarder.
  - Run log: the ISA agent's scratchpad `xdrun19`.

- **First blocking wait:** not found yet. The 11-21 GDK pass did not run XDefiant, because in-game QA is paused until the progress map reaches 100%.

## XDefiant online gaps
- The 6:58 run used stale DLLs. It logged `xdefiant-stub ... (generic empty answer)`, a string that no longer exists in the source, and those DLLs had no RTA emulation. The current build (`D:\TomBox\target-xcurl\release`) has:
  - in-process RTA (`online/rta.rs`; Fortnite's 7:47 run shows `[online] websocket [rta] /connect opened` and subscriptions answered);
  - achievements, peoplehub and userpresence (`online/xbl_title.rs`), the three calls the run made before RTA.
- Transport: `Rogue_scarlett_rtx.exe` uses libHttpClient, with HTTP over XCurl (`XCurl.dll`, `XCurlMulti`) and websockets over WinHTTP (`WINHTTP.dll`: `WinHttpWebSocketCompleteUpgrade`, `Send`, `Receive`, `Close`). There is no `curl_ws_*`. That is the same path net_guard and `rta.rs` already serve for Fortnite. In the stale run, no WinHTTP traffic followed the RTA security query, which fits DLLs built before the WinHTTP websocket fix.
- Missing: catalog data (store items, seasons, battle pass). Nothing is loaded unless `xdefiant_catalog` or `<data_root>\xdefiant\catalog.json` exists; it must be built from the game's own files.
- Missing: Vivox voice transport (SIP/RTP). Tokens and config are issued locally, but there is no Vivox server emulation, so voice will not connect.

---

## Not started
- Keyboard input. The controller works through the virtual pad (`tools\dev\pad.ps1`).

## Fortnite solo party and single-player path (2026-11-21, party agent)

- Party state is real now (online/fortnite/party.rs): create, GET party, GET user current, party PATCH and member-meta PATCH with revision and a 409 stale_revision check, plus PARTY_UPDATED, MEMBER_STATE_UPDATED and MEMBER_JOINED pushed from xmpp-admin. The run36 lobby sequence is replayed by the test run36_lobby_party_replay, and the notifications arrive in well under 2 s. Not yet seen in game: every run so far (35 to 41) predates this code. Next run must show no "TIMING OUT TASK [UpdateParty]" and must show "[party] PATCH ... rev N->N+1 published 1".
- The real PATCH body keys are not captured yet. Bodies are now logged as "[party] PATCH <path> body=..." in tombox_compat.log. If the client sends a revision the server does not expect, it gets 409, so check for "stale rev sent" lines.
- Single player: every playlist, including Playlist_Tutorial_1, BattleLab and Playground, is logged as "(Server Only)". The XSX client has no offline or local-match path in its strings. The only way in with no server is the client as its own authority (NM_Standalone): the StartGameInstance command-line patch at RVA 0xB46800 plus a leading map URL (run_fortnite.ps1 -Match Map, os_host/fortnite_match.rs). Not yet run in game. Open: the game-mode class path, ReadyToStartMatch without a playlist, MCP and inventory loading when the frontend is skipped.
- On user request, multiplayer matchmaking, MPSD, QoS and the LAN party work are paused. That code stays on disk, tested, but is not being extended.
- Run 42 update (party agent): the solo party fix is confirmed in game (0 UpdateParty timeouts). Play failed with invalid_bucket_id because QoS left the region at NONE (bucket `14457890:0:NONE:playlist_defaultsquad`). The ticket now treats NONE as the local region (`TOMBOX_FN_REGION`, default NAE). It still rejects the pre-init bucket `0:0:NONE:0`.
- Likely QoS cause: two datacenters (VA and OH) pointed at the same echo port, so both replies were credited to VA and OH stayed at 9999 ms. NAE now has one datacenter only. Unconfirmed whether the region is then selected.
- The session now writes the ticket's playlist into TOMBOX_FN_PLAYLIST, so the Play-mode Browse rewrite loads Apollo_Terrain with `?Playlist=<selected>`. Every Battle Royale playlist uses Apollo_Terrain. Creative and Playground playlists would need their own map plus game mode; not done.
- The lobby → Play → local match flow only works when the game is launched with `run_fortnite.ps1 -Match Play`. Run 42 was launched in Off mode: its log has no fn-match lines.
- Not verified, and the next risk after Play: the content beacon on BEACONPORT 15009 (failure should not cancel, because ContentBeaconFailureCancelsMatchmaking=0) and any reservation beacon. Watch for Beacon lines.

## Fortnite movies and loading screens (run41, 2026-11-21)
- Bifrost `Movies/Bifrost/Comp 1-6.mp4`: not in the dump (checked the extracted tree, EmbeddedXvd and the pak indexes). The six "Failed to validate media source" errors at 12:56:07 are expected. The game continues cleanly: the frontend loads, the second loading screen hides at 12:56:09.9, and the lobby clip opens, plays and ends at 12:56:59–57:00.
- No startup movies: DefaultGame.ini, XSXGame.ini and BaseXSXGame.ini (read with uex) have no MoviePlayerSettings/StartupMovies. The engine loading screen is the Slate FortniteEngineLoadingScreen, and the in-game loading screens use pak textures (Content/2dAssets/LoadingScreens/*). So no local mp4 is opened on the normal boot → frontend path; run41 has no source reader, which is correct. Onboarding, Wargames and HomebaseReturn movies play only in their own flows.
- Lobby clip path, verified in game (run41): Electra → H.264 MFT → D3D12 NV12 → `video.copy` into an R8 texture. The readback gave Y avg 15 / max 16, i.e. video black, which is what the old placeholder contained. The placeholder is now a 640x360 blue-gradient H.264 Main@3.0 clip (backend_data/fortnite/video/init.mp4 + seg0.m4s), so the next capture can show whether it reaches UMG. Test: mfplat proxy_dll `lobby_video_placeholder_decodes_to_nonblack_frames_on_every_path`.
- Offline MF harness (mfplat tests/proxy_dll.rs, per-mode tests): SatelliteConnection, Onboarding_Appended_Intro, WargamesVictory and HombaseReturn_OutpostIntro decode to non-black 1920x1080 frames in software and as D3D12 NV12 on the console device (MFResetDXGIDeviceManagerX).
- OPEN (host MF/driver, not on the Fortnite path): with a D3D12 manager, a source reader asked for RGB32 output gives all-zero frames for 1920x1080. 640x360 works. The game uses NV12.
- OPEN (host MF/driver, intermittent): a second D3D12 source reader on the same device sometimes fails its first ReadSample with DXGI_ERROR_DEVICE_HUNG (GetDeviceRemovedReason 0x887a0006). It reproduces with the plain IMFDXGIDeviceManager::ResetDevice as well, so MFResetDXGIDeviceManagerX is not the cause. Flushing the reader and shutting down the source don't help. Risk: a second in-game video on the same device. Not seen in a game yet.
- OPEN (most likely the user's "loading screen doesn't show"): from 12:55:29.68 (PresentX frame 3) to the splash at about 12:55:52, the game thread sits in engine init/LoadMap at frame [5] while the EngineLoadingScreen preload screen is active ("HasActivePreLoadScreenType(EngineLoadingScreen) Is True"). Yet frames 3→300 span 29.68→60.54 s, and the splash alone accounts for about 300 frames, so almost nothing is presented for about 22 s. Also logged: "LogMoviePlayer: PassLoadingScreenWindowBackToGame failed. No Window". Next step: run once with TOMBOX_RTCAP=1 and emulator/tools/dev/loading_capture.ps1. A "still-pending" mark means no present happened in that window. Then check the SlateLoadingThread / render-thread present path in d3d12_xs (owner: the d3d12/present agent).
- Measurement for the startup present stall (run41, from the d3d12_xs upload pump, which runs once per PresentX): presents 1–31 fall in 12:55:29.67–31.02, then nothing until present 32 at 12:55:37.95, a gap of 6.93 s. Between 37.95 and present 300 at 12:56:00.54 there are only 268 presents, against ~79/s in the lobby (presents 300→600 took 3.8 s), so this whole init/preload window presents rarely. Watcher for the capture run: emulator/tools/dev/loading_capture.ps1 -LiveLog <live ODS log> (frames=1, marks in UTC at %TEMP%\tombox_capture_marks.txt).
- mfplat proxy_dll: `fortnite_movies_decode_to_nonblack_nv12_on_console_d3d12_manager` and `..._rgb32_...` are #[ignore] with the two host-MF reasons above. The software test and `lobby_video_placeholder_decodes_to_nonblack_frames_on_every_path` stay active. d3d12_device() now panics if device creation fails after a device has existed, so a lost device can no longer pass as a skip.
- Run 43 crash (party agent, 07:53 build fixes the likely cause): an access violation in ntdll inside a winhttp websocket callback. The path was net_guard `cb_wrap` (xmem +0xd5b60/+0xd5efa) → the game's callback (EXE +0xbd93ec) → Websocket.dll. The faulting object held the RTA websocket handle 0x2910724f0 and looked freed and reused (its critical-section fields were garbage).
- Root cause found in net_guard: the game-callback table was keyed by raw handle value, was never cleaned when a handle closed, and child handles found their owner by walking the parent chain at callback time. WinHTTP reuses handle values. So when the upgrade request closed and a new request took the same value with its own callback, the websocket's events went to the wrong callback and context.
- Fix: a child now takes a copy of its parent's callback when it is created (as WinHTTP does). A creation purges any stale entry for that value, and HANDLE_CLOSING removes the closed handle's entry.
- Unit tests `net_guard::callback_owner_tests` cover it, and net_ws_async_rta and net_ws_winhttp still pass. Whether this was the run 43 crash is not proven: it was a single, timing-dependent crash. Run 42 used the same code paths and did not crash.

## xmem (emulator, all titles) - 2026-11-21, imports agent
- OPEN: the console XMemAllocatePhysicalPages (xmem.dll 0x1a40 -> 0x3704) refuses allocations with ERROR_NOT_ENOUGH_MEMORY once paged+nonpaged pool bytes (SystemPerformanceInformation +0x70/+0x74) exceed the hard limit, and warns once past the soft limit. The emulator keeps both limits (DllMain base + 32/30 MB, raised by XMemTransferMemory directions 0/1, reported by XMemGetWorkingSetStatistics internal flag 8) but does not apply the gate: on PC the kernel pool is shared with every other process, so a 32 MB headroom would fail title allocations at random. Needs a title-only pool measure before the gate can be wired.
- OPEN: the XMemPreallocateFreeSpace / hysteresis granule cache is used by XMemAlloc only for allocations that are exactly one granule (512 KB, or 2 MB with option 6 = 0; 4 MB on the 19041 console build that Fortnite ships, selected through the gameinput_os_build setting, which defaults to 22000 and is not set for Fortnite yet); the console carves every page-pool allocation out of granules. Alt pools (index type | 0x10) keep their own cache and statistics but no emulator allocation path draws from them, because which attributes select the alt pool is not decoded yet.

## D3D12.x device slots (emulator, all titles) - 2026-11-21, dev-b
- OPEN, Device slot 68 (console/GPU info, 16 bytes out): out = {u64 KMD reply[0..8], u32 console type, u32 reply[12..16] on Xbox One}. The emulator emulates the IOCTL 0x7900c1fc reply per device type. reply[8..12] is the hardware mode: Scorpio flag on Xbox One (xgs12 0x49a96 -> XGSetHardwareVersion(2)); 1 = Lockhart / 2 = Anaconda on Scarlett (xgs12_10 0x50390, 0xa30c). reply[12..16] is the active CU count: 12/12/40/44 for One/One S/One X/One X devkit (44 from pixrt 0xf1b8), 20/52 for Series S/X (public specs). Still missing: reply[0..8]. No console binary reads it (xgs12, xgs12_10, pixrt, d3d12_x and d3d12_xs discard it or pass it through), so its meaning is unknown and it is returned as 0. Code: crates/compat/d3d12_xs/src/console_slots_dev2.rs (kmd_info).
- OPEN, Device slot 82 (not implemented): creates a GPU hardware performance-counter set, a device child of 0xaa70 bytes on Xbox One (vtable xgs12 0x93b20) and 0xb9f0 on Scarlett. Construction (0x572ac) turns per-block counter selections into PERFCOUNTER select/register entries from the tables at 0xb9740 (Durango) / 0xbaaf0 (Scorpio). Method 8 returns the sample size; method 9 resolves samples from a buffer resource into a {0x1ef8, 2} result block. PC D3D12 cannot program hardware counters, so no sample data can exist. Same situation as CommandList 100-102.
- OPEN, CommandList slot 81 DrawIndexedX 5th argument (more evidence for the cl-a entry): release xgs12 0x512c0 writes record 0x4004cd00 {u64 index VA @+4, format @+0xc, count @+0x10, arg5 @+0x14}. Debug xgs12_d 0x820e0 checks PSO, format, count and VA alignment, but copies arg5 unchecked. Plain DrawIndexedInstanced (0x50c80) writes a different record, 0x4004c200 {InstanceCount, StartIndex, IndexCount, BaseVertex, StartInstance}, so the field layouts don't line up. No code in xgs12 parses opcode 0xcd (the record is only written at 0x512fb), so the CP microcode consumes it. Title call sites can't be told apart from other interfaces' slot 81.
- OPEN, Device slot 44: two narrow cases remain where PC D3D12 can't express the console placement (placed_offset.rs, msaa_place.rs). Everything else listed earlier is fixed: GetDesc width, CopyTextureRegionX offset, CopyResourceX write tracking, offset cleanup on a failed watch attach. DiscardResource on a buffer does no GPU work, matching console xgs12 0x38020, which only discards resource kinds 3/4 (colour and depth targets).
  - Typed or structured view whose element size doesn't divide its byte position in the page: the view is built on a helper placed buffer that aliases the same heap 64 KB steps lower, so its first element is exact (stride 12 at +0x2000 uses one step). PC placed buffers must start on 64 KB steps, so this has no solution when the buffer is in the heap's first page or the position has an odd power-of-two factor smaller than the stride's. Then the view is logged and starts early.
  - MSAA textures: alias heaps of 4 MB or more are now created 4 MB-aligned, so MSAA placements share the heap when their offset from the span start is a 4 MB multiple. A 4 MB-aligned MSAA placement at a VA whose heap offset isn't a 4 MB multiple still gets a separate committed resource. PC requires that alignment, and the heap layout follows the title's VA reservation.

## Console ntdll / KernelBase / SHCore exports (emulator, all titles) - 2026-11-21, imports agent
- OPEN, ntdll RtlpExecuteUmsThread, RtlpUmsExecuteYieldThreadEnd, RtlpUmsThreadYield (console 19041 only): assembly labels inside user-mode scheduling. They need kernel UMS, which Windows 11 removed; console 22000+ drops these labels and its UMS exports return STATUS_NOT_SUPPORTED. Not exported by crates/compat/ntdll. No title imports them.
- OPEN, KernelBase AddPackageToFamilyXref / RemovePackageFromFamilyXref: deployment internals that create/delete ACL-protected HKEY_USERS\<sid>\...\AppModel\Repository\Families\<family>\<fullname> keys (InstallTime qword plus a dword, built security descriptor). Doing it for real writes restricted keys into the user's real hive, and the emulator has no package repository behind it. Not exported by crates/compat/kernelbase. No title imports them.
- DONE, SHCore #227 (console 19041 0x5a3e0): decoded as the COM twin of RegisterScaleChangeNotifications, (kind, IUnknown *handler, DWORD *cookie). Handlers get slot 3 Invoke(scale) on a scale change; non-agile handlers go through RoGetAgileReference(abadfe7f). crates/compat/shcore/src/scale.rs backs each handler with a PC RegisterScaleChangeNotifications window registration, so cookies share the PC counter, and exports RevokeScaleChangeNotifications to release it. Known differences: kind > 2 and NULL handler/cookie return E_INVALIDARG (the console indexes and dereferences them unchecked).
## Mesh / amplification shader stages (emulator, all titles) - 2026-11-21, stages agent
- DONE: Mesh shaders translate from RDNA2 ISA to HLSL ms_6_5 (crates/tombox-xshader/src/hlsl/mesh.rs). The console MS ABI is taken from 8765 GTA VI mesh programs: v5 = workgroup vertex id (gid.x * threads + thread index), s3[27:24] = wave index, s_sendmsg 9 (GS_ALLOC_REQ) with m0 = prims << 12 | verts, exp pos 12 / clip-cull 13-15, exp prim 20 (indices at bits 0/10/20, bit 31 = culled). It renders correctly on WARP (tests/mesh.rs).
- OPEN, real mesh programs: none of the four titles has a mesh or amplification shader in its uncompressed data or captured shader caches (Fortnite pakchunk0: 105,506 unique shaders, 0 MS/AS). All 8765 GTA VI mesh shaders have no RTS0 chunk, so their user-SGPR resources (s16 and up) cannot be bound without the title's root signature. Without an external root layout, translation stops at the first resource load (8749 of them) or at an unsupported wave-reduction prologue (16 of them). The mesh pipeline now supplies the original RTS0 layout from a root signature created by the console compatibility DLL. Offline analysis can supply the layout with `xshader --root-signature`. A synthetic stripped mesh shader translated and compiled with that layout. The examined GTA VI sample has no executable to provide its pipeline root signatures or validate the resulting bindings.
- OPEN, amplification shaders: not translated. The ABI was derived from the xgs12_10d driver side (0xde6f0, 0xde8a0, 0xde570):
  - The AS runs as a compute dispatch on the ACE queue.
  - The user data is written by SET_SH_REG to COMPUTE_USER_DATA_0..3:
    - s[0:1] = the user-data block (memory-based root ABI);
    - s2 = mesh-args ring >> 8;
    - s3 = payload ring >> 8.
  - COMPUTE_START_X carries the first group, and the dispatch is DISPATCH_DIRECT start+count, 1, 1. Wave32 is selected from PSO bit 0x2000000. PSO bit 0x4000000 marks an AS PSO.
  - For List6::DispatchMesh, the device rings are used: args ring at [dev+0xc440] (size [dev+0xc448], 16-byte entries); payload ring at [dev+0xc460]+0x100 (size [dev+0xc468]-0x100, entry = (PSO+0x554 >> 8) & 0xfff0). They are dispatched in chunks of at most min(ring entries).
  - The AS leaves only through global memory stores into those rings, and the translator has no global-store path.
  - Still unknown: per-entry ring addressing and wrap inside the shader, and the register the mesh shader reads its payload from. The translator refuses Stage::Amplification with this text.
- DONE, console entry points: crates/compat/d3d12_xs/src/console_slots_mesh.rs wires the Scarlett family only. xgs12/xgs12_d (Xbox One) have no mesh slots, and their slot 98 is PrefetchMemoryX. The slots come from the xgs12_10d tables built by static initializers:
  - device 0x1cc7c0 (86 slots), followed by the .data chain 0x335780 -> 0x334890 -> 0x3379f0 -> 0x335dd0 -> 0x336640 -> 0x336990 -> 0x336120 (the object vtable, stored at 0x7ca2c);
  - graphics list 0x1ccf10 (111 slots), followed by 0x334510 -> 0x335a40 -> 0x3353a0 -> 0x336cf0 -> 0x334bd0 (stored at 0x4fb1e).
- The wired slots:
  - Device 86 = Device2::CreatePipelineState (0x7c1f0). Null this, desc or ppv returns E_INVALIDARG. The stream is translated and forwarded to PC Device2 slot 47.
  - List 98 = DispatchMeshX(count, u64 flags, indirectArgsPointer, payloadPointer) (0xdec00). It runs every debug check (flags 0/1, 256B/16B alignment, 40-bit limit, args 0 without AS, no AS in bundles); on any failure nothing is recorded. flags 0 with payload 0 is forwarded to DispatchMesh(count,1,1).
  - List 127 = List6::DispatchMesh(x,y,z) (0xdf210), with Y=Z=1, forwarded to PC List6 slot 79.
- OPEN, DispatchMeshX not executed in two cases (logged):
  - flags 0 with a payload pointer: the MS reads its payload from memory through an undecoded register.
  - flags 1: the console AS fills the app's rings without launching mesh groups, and PC D3D12 has no such dispatch.
- CHECK (console_slots_dev2 owner): for Scarlett, dev-b's plan installs gated_uav_x at device 86 and multi_fence_x at 88. In xgs12_10d, 86 is Device2::CreatePipelineState and 87..89 are Device3 E_NOTIMPL (0x28a0). device_wrap v[87] = device87_release_x has the same mismatch. console_slots_mesh installs after them and overrides 86 for Scarlett; 87/88 are left to their owners.
- DONE, FORCE_MEMORY_BASED_ABI: this is RTS0 flags bit 0x01000000. xgs12_10d 0xbdc04 stores RTS0+0x14 with bit 31 cleared into [rs+0x538]. The 0x6dc70 check runs when AS and MS are set.
  - mesh_pso::as_root_rule returns the console text "When an amplification shader is used, the root signature must use D3D12XBOX_ROOT_SIGNATURE_FLAG_FORCE_MEMORY_BASED_ABI flag" -> E_INVALIDARG.
  - The flags come from the RS subobject, or from the AS's embedded RTS0.
  - WARP rejects bit 24 (CreateRootSignature E_FAIL, tests/mesh.rs). console_slots_mesh therefore hooks device slot 16 for both families: it clears the bit for PC and records the console flags per root signature.
- OPEN: the shader translator does not model the memory-based root ABI that FORCE_MEMORY_BASED_ABI selects (root arguments read through s[0:1] instead of user SGPRs). Shaders used with such root signatures still go through the register-based mapping.

## D3D12.x CommandList slots (emulator, all titles) - 2026-11-21, cl-a
- OPEN, slot 64 SetDBCountControlX(const UINT* pControl): writes DB_COUNT_CONTROL (null = 0x11000100). Z-pass counting matches PC occlusion queries. Z-fail, stencil-fail, disabled counting (bit 0) and fixed-sample-rate modes have no PC query type, so those controls are logged and PC keeps counting Z-pass samples. Code: crates/compat/d3d12_xs/src/cl_ext.rs.
- OPEN, slot 80 RSSetMSAAParametersX centroid priority (u8[16]): PA_SC_CENTROID_PRIORITY has no PC control; the PC rasterizer picks centroid samples itself. Sample positions are applied through SetSamplePositions.
- OPEN, slot 81 DrawIndexedX 5th argument: the UMD copies it into firmware record 0x4004cd00 without validation; its meaning is in the CP firmware, not in xgs12/xgs12_d. Draw is forwarded to IASetIndexBuffer + DrawIndexedInstanced.
- OPEN, slot 86 (no arguments): the UMD only appends firmware record 0x4000d100. Meaning not in the UMD.
- OPEN, slot 74 ExecuteIndirectBundleX raw data: buffers holding a CloseBundleX record (console_slots_cl2) execute through PC ExecuteBundle. A size read from pSizeBuffer by the CP at execution time, and raw console bundle packets written by the title, have no PC equivalent (PC bundles are CPU-recorded).
- NOTE, slots 75-77 GDS: the D3D12 side is done (64 KiB device UAV buffer, WriteGDSX/ReadGDSX copies, root UAV u0 space4000002 appended to every root signature with room, binding contract in crates/tombox-xshader/src/gds_abi.rs). Shaders see GDS once the mem agent's tombox-xshader lowering of GDS ds_* / ds_ordered_count / ds_gws uses that binding. ds_ordered_count becomes atomics: unique but not in wave-launch order. SetupOrderedAppendCounterX config for counters 0-7 is written to the table at GDS dword 0x3f00 (TB_GDS_OA_* in gds_abi.rs). The GDS buffer (like the wait-probe readback buffer, the gpu_wait device and the SYSTEM_TIMESTAMP pipeline) is one per process, made on the first device seen; whether the console partitions GDS per queue is unverified. Bundles cannot use WriteGDSX/ReadGDSX/SetupOrderedAppendCounterX on PC (no barriers or copies in bundles), logged.
- OPEN, slot 70 Write64BitValueBottomOfPipeX SYSTEM_TIMESTAMP frequency: the UMD emits EVENT_WRITE_EOP data_sel 4 (system counter); its frequency is read by CP firmware and is not in xgs12/xgs12_d. The emulator converts the GPU timestamp to the QueryPerformanceCounter time base via GetClockCalibration on the GPU (crates/compat/d3d12_xs/src/sys_ts.rs). Needs the console system-counter frequency to confirm. 2026-11-21 d3d-clgpu: every other flag/flush combination now follows the release UMD (bop64.rs), and the GDK headers/docs for D3D12XBOX_WRITE_VALUE_BOP_FLAG_SYSTEM_TIMESTAMP are not public. A further PC limit: a 64-bit value is written through WriteBufferImmediate as two 32-bit writes, so a concurrent CPU or other-queue reader can see a torn value; the console's EOP write is a single 64-bit write.
- CLOSED 2026-11-21, slots 71/72 waits on shader-written default-heap memory: a wait on a tracked GPU buffer with no satisfying tracked write becomes a condition fence released by a sampling probe (crates/compat/d3d12_xs/src/wait_probe.rs). After every ExecuteCommandLists while a wait is pending, and on every known queue when one is armed, a small list copies the watched dword/qword into a readback slot and signals its own fence; a watcher thread compares the sampled value and signals the condition from the CPU. Real-device tests (hw_tests.rs, hardware and WARP) cover a compute shader writing the value on another queue after the waiter was submitted. Remaining: the value is only sampled at submission boundaries (the waiter resumes after the producing batch ends, never earlier than on console), a buffer the title left in UNORDERED_ACCESS cannot be sampled on a COPY queue (sampled on the other queues), and waits on texture memory (no PC byte address) still run without the wait.

## Console OS service / proxy DLLs (console-libraries map) - 2026-11-21, dev-a
- OPEN, gamecoreclient.dll DllGetClassObject (re-checked 2026-11-21 dev-a): the WRL module (vtable 0xb068) has two COM entries, {F5715449-10F6-4225-A7C6-D584BD3C7F5B} (entry 0xb2a8, factory creator 0x5840, factory vtable 0xb1e0) and {7FA7A8FD-A259-4A79-BB7E-094C18CD0723} (entry 0xb460, creator 0x6f90, vtable 0xb348). The objects are the GameCore VM proxy-app side: the message-queue client opens \Device\xvcmgmt (NtOpenFile 0x7f1e) and sends IOCTLs 0x150098 and 0x1500a0 (0x8296, 0x8317) plus dynamic codes (0x8715, 0x8dfc). It also uses WNF (subscribe at 0x74d6 and 0x81aa, query at 0x7461 and 0x8152), Local\SM0:%d:%d:%hs mutex and semaphores, PostMessageA window forwarding (0x6444..0x66d7), CoCreateInstanceEx (0x7d74), and waits on the RpcSs service through the real SCM (OpenSCManagerA, OpenServiceA, NotifyServiceStatusChangeA at 0x79c4). The peer of the message queue is xvcmgmt.sys plus the host OS, and the host OS is not in any dump. Import map: 29 of 131 imports are covered by console-sandbox, 74 are pure in-process, and 49 need new emulation (xvcmgmt message queue, WNF, ntuser messaging, SCM notify). It stays missing, because a factory whose objects cannot reach a peer would be fake success.
- DONE 2026-11-21 dev-a, TimeBrokerServer.dll: the crates/compat/timebrokerserver crate forwards ServiceMain and SvchostPushServiceGlobalsEx to timebrokerserver_sys.dll (a staged System32 copy). The legacy SvchostPushServiceGlobals builds the PC Ex block the way PC svchost 0x3680 does: version 1, legacy block at +8, Ex+0xd0 calls legacy +0xc0 with the same six arguments, and Ex+0xd8 does a non-blocking unregister or returns 0x57 for NULL. It then calls the PC SvchostPushServiceGlobalsEx. If the PC DLL is missing, the error is recorded in last_push_error. OPEN: the forwarded ServiceMain is never called by the tests. It runs the PC time broker, which binds the real PC RPC endpoint and SCM, so it needs the in-process service host (console-sandbox) before it can run in the emulator. NOTE staging: stage System32\TimeBrokerServer.dll as timebrokerserver_sys.dll beside the compat DLL.
- OPEN, EventAggregation.dll BriSignalBrokeredEvent (re-checked 2026-11-21 dev-a): an in-process broker cannot be added while the other exports are forwarded. Console BriCreateBrokeredEvent(Ex) and BriDeleteBrokeredEvent register and delete events with the broker over RPC, and the Ea* aggregation layer calls them internally: 0x6a19 and 0x6d8a call BriCreateBrokeredEventEx 0x7300, and 0x66df and 0x67c6 call BriDeleteBrokeredEvent 0x7c00. If those 32 exports are forwarded to the PC DLL, the events live in the real PC broker, and an in-process Signal could not see them (fake). The Ea* layer also writes its store through ZwCreateKey and ZwSetValueKey into the real HKLM. The consistent design: run the console EventAggregation.dll (all 33 exports, console-image) with its registry routed to console-sandbox, and RPC (RpcBindingCreateW, RpcBindingBind, NdrClientCall3 on stub descriptors 0xe650 and 0xead0, procs 4 and others) routed to an in-process broker emulation. The emulation still needs: the opnum and argument map for each broker proc, decoded from the MIDL proc format strings, and the broker semantics for delivering a signal to registered events. Console consumers are services.exe and TimeBrokerClient.dll (Create and Delete only); nothing in the 19041 image imports BriSignalBrokeredEvent. Not done.
- DONE 2026-11-21 dev-a, bootshsvc.dll ServiceMain: runs the console's own bootshsvc.dll (console-image) with every import routed to crates/compat/console-sandbox. The sandbox provides: an emulated registry persisted at <data dir>\console\registry.txt; an emulated hostname (ComputerName, ActiveComputerName, Tcpip Hostname and NV Hostname); a console C: drive under <data dir>\console\fs; and an in-process service host (RegisterServiceCtrlHandlerExW, SetServiceStatus, the legacy RegisterStopCallback ported from svchost 0x3c50/0x2b20, and the SCM open/query/close used by the 22000 and 22621 builds). It also emulates the service-process environment and priority class. Tests run the 19041, 22000 and 22621 builds: startup.bsc sethostname and copy work, status goes 2/4/3/1, and the PC registry, hostname and process environment are checked unchanged. OPEN: the script start command (CreateProcessW) cannot host a second console process in-process. It returns ERROR_NOT_SUPPORTED, which is recorded, when the image is in the sandbox, and ERROR_FILE_NOT_FOUND otherwise. OPEN: the sandbox registry starts empty; no console SYSTEM hive is in the dumps. NtCreateKey and RegCreateKeyExW create missing ancestors.
- OPEN, netprofmsvc_c.dll ServiceMain and DllMain (mapped 2026-11-21 dev-a): 224 imports. 33 are covered by console-sandbox and about 110 are pure in-process; the rest need new emulation before the console code can run. That covers: an RPC server inside the in-process host (25 RPCRT4 imports, RpcServerUseProtseqW and RpcServerRegisterIf3); WNF publish and subscribe (RtlPublishWnfStateData would write real PC WNF state); offline connectivity probes (12 WINHTTP imports, which must go through net_guard and never reach the network); an emulated network view (8 IPHLPAPI, 6 WS2_32 and NSI imports); COM and WinRT class registration (11 + 2 + 4 combase); and NetGetJoinInformation. Only SvchostPushServiceGlobalsEx is implemented.
- OPEN, ConIoSrv.dll ServerDllInitialization (mapped 2026-11-21 dev-a): 74 imports. 26 are covered by console-sandbox, 8 are in-process, and 46 need emulation: 33 ntdll calls (the ALPC server NtAlpcCreatePort, NtAlpcSendWaitReceivePort and NtAlpcCreatePortSection, plus RtlCreateUserThread); 5 CM_Register_Notification and device-interface list calls (the keyboard and display arrival the emulator would have to synthesize); DeviceIoControl to the console device; CreatePipe; and LoadLibraryExW of kbdus.dll. There is no CSR host or ConDrv emulation in the emulator. Not ported.
- NOTE 2026-11-21 dev-a, hostname scope: console-sandbox's emulated hostname covers only console code routed through the sandbox (bootshsvc today). A title calling GetComputerNameW still goes through kernel32legacy to the PC function and gets the PC name. Routing those calls to console_sandbox::system::hostname is open.
- NOTE, staging: crates connectedstorageservice_proxystub (stage as ConnectedStorageService.ProxyStub.dll) and dab (needs System32 dab.dll staged beside it as dab_sys.dll, as mfplat_sys) are not yet in run_xone.ps1.
- FIXED 2026-11-21 stages, hlsl.rs forward branches: a second branch to an already-open target used to only set `jT = true`, and the open `if (!(jT))` guard kept running the instructions up to T. emit_body now closes the guard after any forward branch whose target is in the open set, so the next instruction re-tests it. Exec test: tests/exec.rs second_forward_branch_into_open_guard_skips (taken, not taken, and first branch taken).

## Console-DLL import routing (emulator, all titles) - 2026-11-21, imports agent
- Shared mechanism: crates/compat/compat-common/src/os_host/routes.rs. compat-common's build.rs builds the route table from every cdylib compat crate's exports.txt (exports_<build>.txt selects the <lib>_<build> crate for older console builds). os_host's sweep and late-load watch patch title IATs (direct and delay imports, by name and by console ordinal). Only implemented, forwarded and hooked rows are routed; everything else keeps the PC binding. API-set imports go to the console host taken from crates/compat/compat-common/data/console_apiset_hosts.txt (tools/dev/gen_apiset_hosts.py, the gen.py host rule, per console OS build).
- DONE: run-time lookups are routed (src/os_host/routes_rt.rs). The title's IAT slots for GetProcAddress, LoadLibraryA/W/ExA/ExW, LoadPackagedLibrary and GetModuleHandleA/W are hooked from kernel32, kernelbase, api-ms-win-core-libraryloader-* and api-ms-win-core-kernel32-legacy-*. A slot is hooked only while it still holds the PC binding, so existing hooks are kept. The gdk GetProcAddress and game-drive LoadLibrary hooks call the same routing. GetProcAddress by name or ordinal returns the routed compat export and the PC result otherwise. A console-only DLL name (not an API set and not in System32) loads the compat DLL. Datafile and image-resource loads and explicit existing paths pass through unchanged.
- DONE: the variant rule matches k32legacy: kernel32legacy_19041 serves builds below 22000, and the default copy serves 22000 and later. TOMBOX_KERNEL32LEGACY overrides both copies (TOMBOX_<LIB> is checked first, then TOMBOX_<console stem>; "0" disables).
- DONE: GetModuleHandleExA/W, GetProcAddressForCaller, and direct ntdll imports of LdrGetProcedureAddress, LdrGetProcedureAddressEx, LdrLoadDll and LdrGetDllHandle use the same routing. Ldr lookups by name or ordinal return STATUS_SUCCESS with the compat export. LdrLoadDll of a console-only name loads the compat DLL. Misses keep the PC status, for example 0xC0000135 or 0xC0000139. GetModuleHandleEx keeps FROM_ADDRESS and refcount flags by re-asking the PC with the compat module's path.
- DONE: a slot already patched by a foreign (non-gdk) hook is chained through a stub generated on demand. Any number of hooks is supported (tested with 24 real hooks and 5000 stubs). Each foreign hook gets a 32-byte stub in a private RX page (written via RWX, then back to RX, then FlushInstructionCache): `mov r10, ctx; mov r11, shim; jmp r11`. The shim, tombox_route_shim, is a global_asm function with SEH unwind data. It passes the context as an extra first argument to the Rust dispatcher, which routes first and then calls that foreign hook instead of the PC. A thread-local re-entry guard keyed per stub sends a foreign hook that calls back into its own stub to the PC. A foreign hook that overwrote our hook later (and so already calls it) is left in place. Those per-slot records are cleared for a module's range on its load notification, so a DLL loaded at a reused base is patched again. In-crate hooks can call routes_rt::resolve_proc, resolve_load and resolve_handle directly.
- DONE: the routing sweep is event-driven. It runs at install, once more after LdrRegisterDllNotification is registered, and on every load notification. The os_host polling thread now sweeps routes only when the load watch is unavailable. Its own hooks keep polling as before. TOMBOX_COMPAT_ROUTES=0 now also skips the sweep entirely.
- Measured, net_async_bulk warm-fetch timing failure, over full `cargo test -p compat-common` runs. Before the change, with polling: routing on failed 2/2 runs that ran the test, routing off failed 0/2. After the change, event-driven: routing on failed 0/5, routing off failed 1/5 (headers 60.5 ms against the 60 ms limit). Warm athena body times are similar on and off (on 43-67 ms, off 40-66 ms). Routing no longer contributes.
- DONE (imports agent; no net agent active, net files untouched for 2+ hours): net_async_bulk now takes the best of 5 warm athena fetches against the original 60 ms header and 100 ms body bounds. Every sample must also finish within 1 s and stay under 2 MB, so a real regression (every warm fetch slow) still fails, but a single load spike does not. Two full-suite runs afterwards: best warm fetch 33-37 ms headers and 37-44 ms body, all samples 33-46 ms.
- DONE: every in-tree hook that re-patches a loader slot resolves through routing. gdk GetProcAddress (gdk/hook.rs) calls routes_rt::resolve_proc. gdk game-drive LoadLibrary* (gdk/fs.rs) calls routes_rt::fs_load_override before its own installed check. A game-drive path that maps to a real file is never overridden. k32legacy redirects no longer take the loader functions. Their kernel32legacy exports are forwards to KernelBase, so routing owns those slots with identical PC behaviour. Direct kernel32legacy.dll loader imports are now routed too. routes::patch_image_for treats a slot whose routing hook already leads to the route target as done, so the import router and the loader hooks do not fight. Tested per family: gdk_get_proc_address_hook_resolves_through_routing, gdk_game_drive_loadlibrary_hooks_resolve_through_routing, kernel32legacy_redirects_leave_loader_slots_to_routing_without_a_patch_fight. The other in-tree hooks (identity, registry, fs file APIs, net_guard GetAddrInfo, console-sandbox, hix_xs) are not loader lookups.
- OPEN: none for routing within the tree. An out-of-tree hook that keeps re-patching its slot over ours and never calls routing would still bypass routing for that slot. Out-of-tree hooks that do not re-patch are chained.
- OPEN: the console apisetschema is not in any dump, so API-set hosts come only from API sets that a title imports and from console export tables. Rerun gen_apiset_hosts.py after adding a title.
- OPEN: a brand-new compat crate is picked up the next time compat-common's build script runs, for example when any watched Cargo.toml, exports.txt or the hosts file changes. Cargo has no directory watch that does not also rebuild on every source edit.

## Xbox Tools Framework xtf* DLLs (console-libraries map) - 2026-11-21, xtf
- DONE: xtfbaseservices.dll 60/60 (crates/compat/xtfbaseservices, shared pieces in crates/compat/xtfcommon). xtfeventserver.dll GetEventSourceMap (crates/compat/xtfeventserver). xtfpix row 7 belongs to the debug/PIX agent; rows 1-6, 8 and 9 are mine.
- OPEN, needs XbtpOpenSession2 (still missing in crates/compat/xbtp): xtfpix XtfCreatePixClient, XtfCreatePixClientQuiet, XtfPixClearRunningProcessList, XtfPixEnableGameDvr, XtfPixPCLogTelemetryEvent and XtfPixRequestVideoClip; xtfremoterun XtfCreateRemoteRunClient; xtffileio XtfCreateFileIOClient; xtfdebugmonitor XtfCreateDebugMonitorClient. Each opens an XBTP session to the console address first.
- OPEN, needs XbtpOpenPipeSession (still missing in crates/compat/xbtp): xtfconsolecontrol XtfCreateConsoleControlClient (Fortnite build only), plus whatever diaginfo and consolecontrol handlers use it.
- OPEN, XtfExtensionEntryPoint (#4) in all seven DLLs: each checks its extension GUID (E_UNEXPECTED otherwise), calls XbtpCreateExtensionMetadata (now implemented) and registers request handlers. The handlers are not ported yet: fileio 22, diaginfo 14, consolecontrol 14, debugmonitor 4, eventserver 2 (real-time ETW consumer: StartTraceW, EnableTraceEx2, OpenTraceW, ProcessTrace), remoterun 1 (RunHandler), and pix via XtfCreatePixServer (#1, a 0x788-byte PixServer COM object). A #4 is only done when all of its handlers are real.
- OPEN, xtfdiaginfo #10 XtfMonitorOperationRequest: it creates an OperationRequestMonitor thread that serves the console-only pipe `\\.\SEVPIPE\XTF_CONSOLE_EXTENSION_API_SYSTEM_TO_TITLE_OPERATION_REQUEST` (CreateNamedPipeW, ReadFile loop, request dispatch). SEVPIPE is a console namespace with no PC equivalent. Not ported.

## xcrdapi.dll XvdXfer host side (console-libraries map) - 2026-11-21, xcrd2
- DONE: XCrdXvdXferInitialize/Start/Read/GetResult/Stop/Uninitialize now run end to end in-process. The DLL side creates the emulator-private pipe `\\.\pipe\TomBox.HSVPipe.XvdXferPipe.<pid>` instead of the console's `\\.\HSVPipe\XvdXferPipe`. The host side (crates/compat/xcrdapi/src/xcrd2_hsv.rs) opens a session on XVD message 0x780000c0, connects to that pipe on its own thread, and serves the console protocol: Start 1/2, Read 3/4 (ack plus data), Stop 5/6. Data comes from an `XferStore`.
- OPEN, store contents: with no store registered, `TOMBOX_XVD_STORE=<dir>` serves `[TAG:]\path` as the raw bytes of `<dir>\TAG\path`. With neither, Start returns STATUS_DEVICE_NOT_READY. Which bytes the console host returns (the raw XVD file, or a decrypted or user-data view) is not in xcrdapi.dll, and the HSV host service is not in any dump.
- OPEN, host status codes: the DLL only forwards the host's NTSTATUS. For a missing file, missing directory, read at or past EOF, and wrong transfer id, the in-process host returns STATUS_OBJECT_NAME_NOT_FOUND, STATUS_OBJECT_PATH_NOT_FOUND, STATUS_END_OF_FILE and STATUS_INVALID_PARAMETER. The console's exact codes are unconfirmed.
- NOTE: XCrdXvdXferStart with a path of 0x104 characters or more returns 0x8007007a. On the console, wcscpy_s would call the CRT invalid-parameter handler.

## pixrt.dll / pixrt10.dll PIX GPU capture runtime (console-libraries map) - 2026-11-21, k32l
- DONE: pixrt.dll 3/3 and pixrt10.dll 1/3 (crates/compat/pixrt, crates/compat/pixrt10, one source). The console's lazy tooling-memory setup is ported for both and for each console build: the `\.\xvcmgmt` overflow IOCTL 0x150068, `XMemCreateWin32Heap(0x8001)`, the "PIXTOOL" heap name on 22000+, `XMemGetWorkingSetStatistics` (0x20008002/0x78 on 22000+, 0x10008002/0x50 on 19041), the pixrt10 hypervisor CPUID 0x4000000D check, `d:\dbgbreakonnotoolingmemory`, and every PIX OutputDebugString line. Also the tooling-memory allocator table (tag 1 heap, 2 XMemAlloc 0x8e0000, 3 CRT malloc).
- pixrt.dll guards all three exports on that state. Under the emulator's xmem there is no tools heap and no xvcmgmt device, so all three return E_OUTOFMEMORY, as on a console without tooling memory. pixevt therefore attaches no runtime and keeps its no-runtime defaults.
- OPEN, the capture runtime behind a passing guard (pixrt PixDllInitialize and PixDllInitializeFromTitle return E_NOTIMPL there; the emulator never reaches it today), and pixrt10 PixDllInitialize and PixDllInitializeFromTitle, which build it with no guard: the 0x120960/0x95ba8-byte capture objects, the capture-control and end-capture threads, ETW sessions, the Xbtp link to the PIX tool, and the 15-slot title table plus the D3D12-side table. pixevt only picks pixrt10 when `\.\XgsKmd10` exists, which it never does on a PC.
- Staging: stage_compat.ps1 already stages pixrt.dll and pixrt10.dll. Every cdylib with live exports goes into the title binary directory, and mdbg starts the title with that directory as its current directory. So pixevt's `.\` entry finds them, and `g:\` does too for XDefiant and xone, whose binary directory is the G: root.
- Retail path under the emulator, checked against the XDefiant/Terraria 22000 binaries: a retail title never loads pixrt10.dll. Both console loaders open `\\.\XgsKmd10` and load pixrt10 only when that succeeds:
  - pixevt's DLL init: PixDllInitializeFromTitle(magic, version, flags 0, 0x78-byte table), then ToolMemory.
  - The console xgs12_10 driver: UMDCreateDevice12 0x58150 -> 0x57cd0 -> 0x57e2c -> 0x59680. It calls PixDllInitialize(0x55f0114d, 0x35dbc, 0, 2, 0, 0x20004, driver table, 0xf8, out table, 0x150) at 0x5b8f6, then FromTitle with 0x78 and ToolMemory with 0x10.
  - Without XgsKmd10 both load pixrt.dll, which stops at the tooling-memory guard (E_OUTOFMEMORY, ported).
  - The emulator's xgs12_10 crate has no PIX attach at all. That is a gap for the xgs12_10 owner, not for pixrt. The D3D12 capture hooks are the 0xf8 driver table and the 0x150 table exchanged there, not d3d12_xs.
- OPEN, pixrt10 PixDllInitializeFromTitle (row 2). It is not ported because of its size. FromTitle at 0x234a0 does the following:
  - No magic/version/size check.
  - Builds the 0x95b30-byte capture singleton (vtable 0x4e2c8).
  - Builds the 0x1098-byte PIX::InteractiveRuntime at 0x18234: events, critical sections, CreateDirectoryW("D:\PIX"), and three named threads ("device engine request" 0x18ac0, "capture engine request" 0x18af0, "notification" 0x18b20).
  - Memcpys its 15-slot table with the caller's byte count, unclamped, and returns S_OK.
  - Reaches 329 functions, about 0x19b1a bytes of code, excluding vtable targets. The 15 slots alone are 63 functions, about 0x249f bytes, including the 0x8fff2090/0x8fff2098/0x8fff209c driver IOCTLs and tooling-memory TLS words.
  - There are three distinct pixrt10 builds: XDefiant = Terraria 22000, Trackmania 22621, and Fortnite 19041.
- Per-build size of the FromTitle call tree (vtable targets excluded):
  - 22000: 329 functions, 0x19b1a bytes, capture object 0x95b30.
  - 22621: 341 functions, 0x1e50e bytes, capture object 0x95ba8.
  - 19041: 212 functions, 0xebeb bytes.
  - Slot 0x22d50 (begin capture) alone jumps to 0x3256c, which is 0x872 bytes. It drives the D3D12 capture interface at 0x5f1d0 and the runtime's capture session at +0x1030.
  - 2026-11-21 k32l status: the xbtp prerequisite is done (link_dev). The runtime itself is still unported because of its size, and a partial port cannot flip a row without fake slots.
  - The xgs12_10 PIX attach (0x59680, 0x911 bytes) builds the 0xf8-byte driver callback table that PixDllInitialize consumes. It is not added either: without a ported pixrt10 it would only reach classic pixrt's E_OUTOFMEMORY guard, and the table would point at driver callbacks that are not ported.
- OPEN, pixrt10 PixDllInitialize (row 1). Same runtime. Flags bit 0 is gated by 0x227c0 (E_NOTIMPL when it fails), it keeps a refcount, and it hands out a 0x150-byte table whose slots switch on the capture object's +0x95b28 byte.
- DONE (k32l, 2026-11-21): the in-process `\??\xbtplink` device is in crates/compat/xbtp/src/link_dev.rs and is the xbtp default DeviceOps. A real device handle is used if one opens; otherwise the endpoint is in-process, with nothing on TCP or the network. XbtpLinkListen now parks its accept thread on a pending device receive until a peer connects or the handle shuts down, so the spin described below no longer happens (tests: crates/compat/xbtp/tests/link_dev.rs). Router codes with no console reference: CONNECT with no listener is refused at once with 0xfd000102; IOCTL_RESERVE returns ERROR_INVALID_FUNCTION; XbtpLinkSetCompletionPort fails on in-process handles because they are not kernel file objects. History, the device-engine spin before link_dev:
  - The thread (0x1b3a0) retries 0x3a4e0 in a loop. The only check between attempts is a zero-timeout `WaitForSingleObject` on the shutdown event.
  - The transport config at 0x51c88 is type 3 (Xbtp) on service 0x1080 with an infinite accept wait. 0x3a4e0 calls the Xbtp transport vtable 0x4e5d0: slot 1 is XbtpLinkInitializeSystem, slot 2 is 0x39790, which does CreateHandle, `XbtpLinkListen(h, 1, 0x1080, &ready)` and WaitForMultipleObjects.
  - On a console `\??\xbtplink` exists, so Listen blocks until the PIX tool connects. The emulator's xbtp `bind` always opens that device, so it returns LINK_E_NO_DEVICE immediately.
  - Neither cleanup path sleeps: 0x3b350 only waits to join worker threads, and 0x39b28 only closes handles and resets events. So a faithful port would peg a core, cycling CreateHandle, Listen and CloseHandle.
  - It needs an in-process `\??\xbtplink` (device-mode bind/listen) before it can be ported faithfully. Once that exists, a port would bind xbtp service 0x1080, which must stay private and in-process (no TCP fallback) under the offline rule.

## xcrdapi.dll emulated XCRD storage driver (src/xcrd_driver.rs) - 2026-11-21, xcrd
- DONE: `\\.\Scsi0:` is an in-process emulated XVD miniport. All xcrdapi OS calls go through EmuHost. It also emulates `\\.\PhysicalDrive{64+lun}`, `\\?\GLOBALROOT\Device\Harddisk{64+lun}\Partition1`, `\\?\Volume{5843524b-...}` and `\\.\MountPointManager`; anything else passes through to Win32. Read-only content comes from TOMBOX_XCRD_ROOT\Mount (default D:\TomBox\run\Mount). Writable XVDs, blobs and feature bits live in TOMBOX_XCRD_ROOT\XcrdStore\<TAG>\. Implemented messages: probe, mount (content type, corrupt header → WNF corruption event), address-for-path, remove LUN, create, delete, user-data/blob read and write, XVD info codes 0/5/0x17, set class 0xa, blob and storage-device enumeration, resize (plus grow-partition, extend-volume and fs-size), 0x780000c0 XvdXfer sessions (via xcrd2_hsv with DriverStore), and the DirectStorage control page IOCTL 0x7800013c.
- DONE, store XVDs are real files: a 0x3000 head, user data, then a u32 block allocation table (dynamic type 1, 0xFFFFFFFF = unallocated, DynamicHeaderLength at hdr+0x94). XCrdReadBatInfoXVD returns that table (src/xcrd_xvdfile.rs). Volume data lives in the `.content` directory, so blocks are allocated sequentially to cover its byte size when the BAT is queried. There is no per-block write tracking.
- DONE, XVD info codes 0/1/3/5/6/7/9/0xa-0xf/0x12-0x17, CRD info codes 2/4/0x10, and set classes 0-0xa/0x11 are implemented (src/xcrd_driver/info.rs). The reply layouts come from xcrdutil.exe's printers.
- OPEN, inferred values with no driver binary (the console XVD driver is not in any dump):
  - BAT reply fields other than those the library copies.
  - Counters (code 0xe) are kept per session only.
  - The XAS/XVCS/update/histogram codes reply all-zero, because no streaming or XVCS infrastructure exists.
  - CRDs 0/1/2/3/5 count as present; the others return STATUS_NO_SUCH_DEVICE.
  - BusType 0, Format Internal, device name "TomBox emulated internal storage".
- OPEN, STATUS_NOT_SUPPORTED (0xC00000BB) remains in four places:
  - XVD info codes 0x1a and 0x1b (classes 19 and 20): xcrdutil never prints them, so their meaning is unknown.
  - CRD log info (code 8) and vendor info (code 0x11): xcrdutil's help marks them "NVMe only", and the emulated device is not NVMe.
  - Package-backed (header-less) XVD info and BAT queries.
- Every other message code returns STATUS_INVALID_DEVICE_REQUEST.
- OPEN, inferred values: the device number 1 for the internal storage device, the mapping of the blob-enumeration device number to the PATH_PREFIXES index, the SCSI bus 7 / target 0x58 / disk 64+lun numbering, and the NTSTATUS codes for collision, sharing, access-denied, mismatch and corrupt are emulator choices, not confirmed console values.
- DONE, title file I/O: compat-common `gdk::fs::translate` sends paths through `gdk::xcrd_paths::resolve` → `TomBoxXcrdResolvePath`. The paths covered are `\\?\` / `\\.\` / `\??\` GLOBALROOT\Device\Harddisk{N}\Partition1 and Volume{5843524b-…NN}.
  - The resolver is found through a registered pointer or GetProcAddress on xcrdapi.dll.
  - The hooks are now installed unless setting `xcrd_paths=off`.
  - CreateFile2 now has a row in the fs hook table.
  - OPEN: compat-common has no NtCreateFile/NtOpenFile hook, so titles that call ntdll directly are not mapped.
- OPEN, DirectStorage control page: the page is real and mapped, and SetDebugFlags writes its shared flags dword. The statistics counters at page+0x200+i*0x80 and the globals at 0x606..0x63c stay zero, because the CPU backend does not update them and their meanings are unknown.
- OPEN, xg_x hardware-version layouts (found 11-21 by a32l): the console `xg_x.dll` texture layout depends on `XGSetHardwareVersion`. With version >= 2 (Scorpio / One X), a 32x1920 R32G32B32A32 2D-thin texture changes: size 0x500000 -> 0xa00000, alignment 0x8000 -> 0x20000, pitch doubles, and the tile words at mip+0x48/0x50 change from 0x1c00/0xc00 to 0x7800/0x3800. Our `crates/compat/xg_x/src/layout.rs` ignores the hardware version, so layouts are only byte-identical for versions 0 and 1. The emulator's Scorpio mode calls XGSetHardwareVersion(2), so One X layouts are wrong until the version-2 GCN macro-tile parameters are ported.
- OPEN, xg_x DCC: XGDecodeDCC and XGEncodeDCC still need the console's GCN software DCC codec ported (xg_x RVA 0x168c0-0x1dd10). It does not match the xg_xs RDNA2 codec, so the xg_xs port cannot be reused.

## RDNA2 s_sendmsghalt / s_trap (tombox-xshader, all titles) - 2026-11-21, scalar
- DONE, s_trap translates as a no-op, because titles run with no trap handler and the hardware does not take the trap:
  - COMPUTE_PGM_RSRC2 ships inside the shader binary. It is FSSM dword 0x130, which decodes as USER_SGPR=8, TGID_*_EN, TIDIG_COMP_CNT and LDS_SIZE.
  - TRAP_PRESENT (bit 6, mask 0x40, gc_10_3_0_sh_mask.h) is 0 in every compute fixture in analysis/shaders.
  - The release UMD copies RSRC1/RSRC2 from shader data into SET_SH_REG 0x212 unchanged (xgs12_10.dll RVA 0x71b0) and never ORs the bit in.
  - No UMD writes SQ_SHADER_TBA/TMA. A scan of xgs12_10.dll found no immediate for mmSQ_SHADER_TBA_LO 0x2312 or its byte offset 0x8C48, and those registers are privileged in any case.
  - Mesa radv 8e97a61: "When TRAP_PRESENT is not enabled, all traps and exceptions are ignored."
  - LLVM non-HSA lowering (AMDGPUUsage, lowerTrapEndpgm) uses s_endpgm instead of s_trap when no trap handler is installed.
  - Test: tests/exec.rs scalar_s_trap, with trap ids 0/2/3/0xff. VALU and SALU writes after s_trap land, state is preserved, and the exec mask is honoured.
  - Not checked: graphics-stage RSRC2 locations in FSSM. They were not decoded.
- OPEN, s_sendmsghalt (SOPP 0x11): stays a translator refusal. There is no faithful PC equivalent.
  - The RDNA2 machine-readable ISA (AMD_GPU_MR_ISA_XML-2024_08_22, amdgpu_isa_rdna2.xml) says: "Send a message to upstream control hardware and then HALT the wavefront". It flags the instruction IsProgramTerminator=FALSE.
  - A halted wave stays resident and does not retire until a debugger, SQ_CMD or the trap handler resumes or kills it. Without one, its draw or dispatch never completes, so on a console it is a GPU hang.
  - Translating it as "message, then end wave" would let the dispatch complete where hardware does not. That is fake success, so it is not done.
  - Corpus check: Fortnite pakchunk0 xscan translates 105506/105506 shaders while this opcode is an Err. No title shader uses it.
  - Unresolved lead: xgs12_10.dll RVA 0x1001d4 logs "Hang detected, trying to exit the waves..." after a fence-wait timeout, then calls vtable +0x88 with 0x10f. That path is gated by a global byte flag that is not shown to be set in retail, so it is not evidence of hang recovery for titles.

## GameHostService host side (crates/compat/gamehost) - 2026-11-21, svc
- DONE: in-process host for WerXboxGc (0xc8 crash-dump info reply 0x218, 0xca progress, disconnect = dump completed), 0xc9 controller notify, 0x12c package mount and 0x12d unmount, decoded from gamehostservice.exe 22000/22621; werxboxgc hooks call it.
- OPEN, 0x67 LaunchNewGame / 0x68 LaunchRestartOnCrash: the wire format is decoded and tested, but the host side (GameHostService::LaunchGameProcess at 22621 RVA 0xa548, the 0xadb0/0xaeb0 handlers) is not ported. dispatch returns Fault::NotImplemented. No emulator path loads the console xgameruntime.dll that sends these; compat-common gdk launch_new_game spawns natively.
- OPEN, launch crash-dump config: the 16 bytes the reply copies (launch data +0xf8, service +0x128) come from the OS-side GameController launch message, which is not in any dump. GameHost::set_launch takes them; nothing in the emulator calls it yet, so replies carry zero flags (heap dump and hang file off) until a launcher supplies them.
- OPEN, quiesce/hang path: the host writes the Quiesce-<exe>-<time> name (0x95cc) when its suspend worker reports a hang (0xfddc). GameHost::record_quiesce exists; the emulator has no suspend-timeout detector that calls it.
- OPEN, \Device\xvcmgmt MessageQueue transport (IOCTLs 0x1500a8/ac/b0/f4): not emulated. Only the Rust werxboxgc crate reaches the host; a console WerXboxGc.dll or xgameruntime.dll loaded by forwarding would still get no connection.
- OPEN, 19041 gamehostservice: its WerXboxGc has no host protocol (no gamehost strings), so nothing is sent there; its dispatcher was not decoded.

## xhit_s GPU hang decoder (crates/compat/xhit_s) - 2026-11-21, xhitdec
- DONE (Fortnite 19041 xhit_s.dll): KMD mailbox protocol (rva 2728, 12230, 122d0, 12320, 3d8c) as `xhit_s::kmd`, with an in-process `kmd::install(data, mailbox, doorbell)` hook in place of `\.\XgsKmd`. Also done: register-snapshot decoder rva 3ea0 + 3e20 (`regs`), SQ capture rva 13dd8 + SQD2 chunk writer 25f0 (`sq`, `chunk`), high-level summary rva 4b34 + 4a50 (`report`), and the bucket-name table rva 9c5c with its 11 helpers (`bucket`). XhitPerformHangSummary now does the console mailbox round trip and returns 0. These offsets apply only to the 19041 build; the XDefiant 22000 build has different RVAs.
- OPEN, bucket classifier rva 13cf8 (83cc, 112f0, 14700 plus helpers, about 13 KB): this is what writes report +0x320 (hang bucket) and bit 18. It is not ported. collect_sq returns the SQ capture, and the classifier stage is not run, so +0x320 keeps the caller's value.
- OPEN, detailed block dump rva 6bec (closure about 42 KB, including the 22 KB PM4/IB walker b5e0): it sets report bits 15/16/17 and +8 and fills the CP wait records. It is not ported, so decode_registers stops after its own block list.
- OPEN, XhitPerformHangAnalysis detailed writer rva 13f98 (closure about 20 KB): not ported. Analysis still writes only the Xhit header and returns E_FAIL, so high_level_summary (4b34) is not reached from the export yet.
- OPEN, emulated Xbox One KMD side: nothing in the emulator calls xhit_s::kmd::install yet. hix_xs's kmd.rs (xbsc) models the Scarlett cmds only. D3D12 has no GRBM/SRBM equivalent, so an honest responder returns a zero register dump (decodes as idle). DRED page-fault and breadcrumb data belongs in the caller-filled report (+0x10/+0x18 wait records, +0x258 objects, +0x318 fence table).
- NOTE, console printf %p in the 0x214b0/0x222b0/0x223b0 messages is rendered as MSVC's 16 upper-case hex digits. Cpc_BadPacket location indices 6/7 read past the console's stack table (undefined on console) and render as "".


## msave64xgs.dll AVE encode client (crates/compat/msave64xgs) - 2026-11-21, xcrd2
- DONE: all 24 ordinal exports (#6934-#6958, no #6951) follow the 22621 console DLL (Trackmania dump). The 19041 error codes (Fortnite) are used when compat_common::title_os::title_os_build() < 22000. 22000 and 22621 behave the same.
- OPEN, the device side of the AVE extension command. msave is only a D3D12 client. Every package (cmds 1-0x23) goes to the device it is handed, through:
  - ID3D12VideoDevice2 slot 12 CreateVideoExtensionCommand, CommandId a218961c-091f-494e-9188-65e2187e69c0, 4-byte creation params;
  - encode list slot 25 InitializeExtensionCommand (16 zero bytes);
  - slot 26 ExecuteExtensionCommand, with params {u32 0x64, package*, 5 data ptrs, 5 u32 sizes} (0x50 bytes).

  d3d12_xs/xgs12 must implement that command on the console device. That means AVE/VCE H.264/HEVC encode via D3D12 video encode, or MF where the PC lacks it. They must also provide console device slot 73 (CreateCommandList with params {type 6, 0, 0, 0}), VIDEO_ENCODE queues, allocators and fences.
- OPEN, the handler must fill replies at record time, inside ExecuteExtensionCommand. The caller reads the results straight after Close and Submit, without waiting on the fence (only packages 6 and 0x19 wait):
  - package +0x10 status;
  - cmd 1 reply +8 = 0x30017 (interface version);
  - cmd 5 handle at package +0x20.

  After packages 6 and 0x19, msave records a package 0x23 {+0x18 = original +0x18} and closes the list, but never submits it. This matches the console. The handler must treat the record itself as the work, or 0x23 never runs.
- OPEN, package layouts for cmds 2-0x22 (the meaning of each a/b struct) are not mapped. msave passes them through untouched, so only the AVE handler needs them. They have to come from the xgs12 videoencoderextension code.
- NOTE, the console passes 4 uninitialised stack bytes as the extension-command creation params. The emulator passes 0.
- NOTE, the 19041 build does not check the resource count: more than 5 type-2/5 inputs overruns the console's own stack block (undefined behaviour). The emulator returns E_INVALIDARG there, as 22000+ does.

## sc_dll_xs.dll (Scarlett legacy SC back end behind xbsc_xs) - 2026-11-21, scdll
- CLOSED, not reached on retail: no emulator crate is built for it. Static evidence (Fortnite 19041 dump; the XDefiant/Terraria/Trackmania copies match):
  - xbsc_xs.dll binds 71 sc_dll_xs functions lazily (name table rva 389e30, slots 389bf8, resolver ba790). If the DLL is missing, the first call raises 0xC026FF00. The 9 SC* exports (SCCompileMultiShadersXbox, SCDump*, SC_ConvertRootSignatureToSCInputs, SC_OutputHandle_GetShaderObject, SC_SCDX12TableDesc_Destroy) are only jmp [slot] thunks.
  - Internally only XBSC_PipelineCompile (9f64, via aab8 and 1c1b8) reaches sc_dll_xs (SCCompileMultiShadersXbox at 1c87c), plus XBSC_ExternalDisassembler from the disassembly code. 1c1b8 picks the back end from the "newbe" chicken bit (c:\windows\system32 marker files: 1 = newbe, 2 = SC). With no marker it uses the PipelineCompileOptions compiler byte at input +0x61 (0 = "NB" newbe_xs, 1 = "SC" sc_dll_xs).
  - xgs12_10.dll (the UMD that d3d12_xs loads) calls XBSC_PipelineCompile from cb120. Both of its input builders set the compiler byte to NB: 72820 stores word 2 at +0x60 (asic 2, compiler 0), and 6cd00 stores word 0x100 and later 0x200 at +0x61 (compiler 0). No other xgs12_10 code writes that byte.
  - dxcompiler_xs.dll binds xbsc_xs through its own lazy table (names fcb1e0, slots fcb0a8). It never calls any of the 9 SC* names. Its XBSC_PipelineCompile input (275cec) is zeroed (0xc8 bytes), asic +0x60 = 2, and the compiler byte stays 0 (NB).
  - Title executables (FortniteClient-XSX-Shipping.exe, Rogue_scarlett_rtx.exe) reference neither xbsc_xs nor any SC* name. Fortnite only imports dxcompiler_xs DxcCreateInstance.
  - No console OS image ships sc_dll_xs.dll, so a retail title that reached an SC thunk would crash on hardware.
- NOTE: the console behaviour without sc_dll_xs (the 0xC026FF00 raise) is what the xbsc_xs forwarders already reproduce. Reopen if a title sets the newbe chicken bit to 2, or a caller passes compiler = SC (1). That would need an RDNA2 (gfx10.3) back end, for example dxil-spirv (MIT) plus Mesa spirv_to_nir/ACO (MIT).

## Audio console exports (aud2, 2026-11-21)
- GCXAudioExt.dll CreateXmaBufferEngine (ordinal 2) is not exported. Console 0x11280 -> 0x18c20 creates a 0x108-byte object (vtable 0x22010, 20 slots, about 13k disassembly lines) on top of an ACP state created with a NULL config. The slots are not ported yet, and a partial object would be fake success.
- XVMAudioService.dll DllGetClassObject (ordinal 3) is missing. It serves 4 WRL classes (148ec3bf-349c-4eac-b373-9fd57cb46d88, e782ced1-3b5d-43aa-a45d-82852b57d84c, 7f34f6c9-bed0-456d-9772-0d5b524a9d69, b988be16-d129-46fc-8c5a-c3ceff592137). Their objects use \.\VirtualAudioBus IOCTLs (the type-2 0x100000 region that acphal slot 10 also requests), the acpi_hal Logan device, \Device\xvcmgmt, WNF state and the XboxSystem job object. The region emulation needs those classes to be ported first.
- chad.dll XDspSubmitCommandWithEnvelope: the DSP microcode that consumes the envelope is not in any dump, so the per-partition gain is inferred from the GDK description. See the chad exports.txt note for the known differences from the console's 8-float snapshot.
- xmem/acphal test race: gcxaudioext tests crash with 0xc0000005 when the first ApuAlloc (xmem.dll + acphal.dll) runs at the same time as MMDevice COM enumeration on another thread. Loading the DLLs alone does not crash. The suspected cause is xmem IAT patching (iat.rs write_slot sets PAGE_READWRITE) racing live modules. The gcxaudioext tests are serialized to avoid it, and the root cause is not fixed.
