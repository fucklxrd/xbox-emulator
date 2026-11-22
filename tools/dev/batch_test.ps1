param(
  [switch]$DryRun,
  [switch]$SkipBuild,
  [switch]$NoD3d12Debug,
  [string[]]$Only = @(),
  [int]$FortniteLobbySeconds = 900,
  [int]$FortnitePlaySeconds = 1200,
  [int]$TrackmaniaSeconds = 600,
  [int]$XDefiantSeconds = 900,
  [int]$TerrariaSeconds = 600,
  [string]$Root = 'D:\TomBox',
  [string]$Report = 'D:\TomBox\tools\dev\batch_report.md',
  [string]$OutRoot = 'D:\TomBox\tools\dev\batch_runs',
  [string]$Agility = 'D:\xbox-pc-port\emulator\third_party\agility-sdk\build\native\bin\x64',
  [int]$AgilityVersion = 620,
  [int]$MaxEntriesPerCategory = 300
)

$ErrorActionPreference = 'Stop'
$Stamp = Get-Date -Format 'yyyyMMdd_HHmmss'
$RunRoot = Join-Path $OutRoot $Stamp
$BatchStart = Get-Date
$Mode = if ($DryRun) { 'DRY RUN (nothing launched)' } else { 'FULL PASS' }
$DbgLayer = -not $NoD3d12Debug

function Say([string]$m, [string]$c = 'Gray') { Write-Host ("[{0:HH:mm:ss}] {1}" -f (Get-Date), $m) -ForegroundColor $c }
function ConvertTo-MdText([string]$s) { if ($null -eq $s) { return '' }; return ($s -replace '\|', '\|' -replace '`', "'" -replace '[\x00-\x08\x0b\x0c\x0e-\x1f]', '') }

$Titles = @(
  [pscustomobject]@{ Slug = 'fortnite-lobby'; Name = 'Fortnite (lobby, default ready-up path armed)'; Script = 'run_fortnite.ps1'; Args = @{};
    LogDir = (Join-Path $env:TEMP 'tombox'); Proc = 'FortniteClient-XSX-Shipping'; Seconds = $FortniteLobbySeconds; Kind = 'fortnite'; Xone = $false;
    Exe = 'D:\xbox-pc-port\games\Fortnite\extracted\Mount\FortniteGame\Binaries\XSX\FortniteClient-XSX-Shipping.exe'
    Saved = @('D:\xbox-pc-port\emulator\run\Mount\FortniteGame\Saved') },
  [pscustomobject]@{ Slug = 'fortnite-play'; Name = 'Fortnite (ready up -> offline single player match)'; Script = 'run_fortnite.ps1'; Args = @{ GameServer = '127.0.0.1:7777' };
    LogDir = (Join-Path $env:TEMP 'tombox'); Proc = 'FortniteClient-XSX-Shipping'; Seconds = $FortnitePlaySeconds; Kind = 'fortnite'; Xone = $false;
    Exe = 'D:\xbox-pc-port\games\Fortnite\extracted\Mount\FortniteGame\Binaries\XSX\FortniteClient-XSX-Shipping.exe'
    Saved = @('D:\xbox-pc-port\emulator\run\Mount\FortniteGame\Saved') },
  [pscustomobject]@{ Slug = 'trackmania'; Name = 'Trackmania'; Script = 'run_xone.ps1'; Args = @{ Title = 'Trackmania'; Drive = 'T:' };
    LogDir = (Join-Path $env:TEMP 'tombox-trackmania'); Proc = 'GbxGame'; Seconds = $TrackmaniaSeconds; Kind = 'generic'; Xone = $true;
    Exe = ''; Saved = @('D:\xbox-pc-port\emulator\run-xone\Trackmania') },
  [pscustomobject]@{ Slug = 'xdefiant'; Name = 'XDefiant'; Script = 'run_xdefiant.ps1'; Args = @{ Drive = 'Z:' };
    LogDir = (Join-Path $env:TEMP 'tombox-xdefiant'); Proc = 'Rogue_scarlett_rtx'; Seconds = $XDefiantSeconds; Kind = 'generic'; Xone = $false;
    Exe = 'D:\xbox-pc-port\games\XDefiant\extracted\Mount\Rogue_scarlett_rtx.exe'; Saved = @('D:\xbox-pc-port\emulator\run-xdefiant') },
  [pscustomobject]@{ Slug = 'terraria'; Name = 'Terraria'; Script = 'run_xone.ps1'; Args = @{ Title = 'Terraria'; Drive = 'U:' };
    LogDir = (Join-Path $env:TEMP 'tombox-terraria'); Proc = 'Terraria'; Seconds = $TerrariaSeconds; Kind = 'generic'; Xone = $true;
    Exe = ''; Saved = @('D:\xbox-pc-port\emulator\run-xone\Terraria') }
)
if ($Only.Count) { $Titles = @($Titles | Where-Object { $Only -contains $_.Slug }) }
foreach ($t in $Titles) {
  if ($t.Xone -and -not $t.Exe) {
    $exeName = @{ Trackmania = 'GbxGame.exe'; Terraria = 'Terraria.exe' }[$t.Args.Title]
    $pkg = Get-ChildItem (Join-Path 'D:\xbox-pc-port\games' "$($t.Args.Title)\extracted") -Directory -ErrorAction SilentlyContinue |
           Where-Object { Test-Path -LiteralPath (Join-Path $_.FullName "Mount\$exeName") } | Select-Object -First 1
    if ($pkg) { $t.Exe = Join-Path $pkg.FullName "Mount\$exeName" }
  }
}

$MilestoneDefs = @{
  fortnite = @(
    @{ Name = 'boot'; Src = 'compat'; Re = '\[gdk\] install emulated GDK runtime' },
    @{ Name = 'first frame'; Src = 'compat'; Re = '\[d3d12_xs\] (PresentX presenting|\[perf\] fps=)' },
    @{ Name = 'login'; Src = 'ods'; Re = 'HandleLoginRequestComplete.*Result=2' },
    @{ Name = 'hotfix check'; Src = 'ods'; Re = 'CheckComplete UpdateSuccess' },
    @{ Name = 'game-mode select'; Src = 'ods'; Re = 'UI\.State\.Startup\.SubgameSelect\]' },
    @{ Name = 'lobby'; Src = 'ods'; Re = 'to \[UI\.State\.Athena\.Frontend' },
    @{ Name = 'ready-up ticket'; Src = 'compat'; Re = '\[mms\] websocket ticket \S+ accepted' },
    @{ Name = 'session assigned'; Src = 'ods'; Re = 'HandlePlayMessage - Received session assignment' },
    @{ Name = 'session joined'; Src = 'ods'; Re = 'OnJoinSessionComplete\] Result: Success' },
    @{ Name = 'match start'; Src = 'any'; Re = '\[fn-match\] browse rewrite|LogLoad: LoadMap: .*(Apollo|Athena_Terrain)|to \[UI\.State\.Athena\.(Loading|Playing|InGame|Spectating)' }
  )
  generic = @(
    @{ Name = 'boot'; Src = 'compat'; Re = '\[gdk\] install emulated GDK runtime' },
    @{ Name = 'first frame'; Src = 'compat'; Re = '\[d3d12_xs\] (PresentX presenting|\[perf\] fps=)' },
    @{ Name = 'login activity'; Src = 'any'; Re = '(?i)LoginWithXbox|XUserAdd\w*.*(S_OK|ok|signed)|signed in|logged in|login (ok|succe)' },
    @{ Name = 'menu / lobby'; Src = 'ods'; Re = '(?i)\b(main ?menu|title ?screen|frontend|lobby)\b' },
    @{ Name = 'match start'; Src = 'ods'; Re = '(?i)\b(load(ing)? (map|level|world)|match start|race start|world loaded|enter(ing)? (world|game))\b' }
  )
}

$DefectDefs = @(
  @{ Cat = 'Crashes and hangs'; Src = 'trace'; Re = 'firstChance=0|ACCESS_VIOLATION|STACK_OVERFLOW|ILLEGAL_INSTRUCTION|^\[HANG\]|too many exceptions|trace limit .* reached|EXIT_PROCESS code=(?!0 )' },
  @{ Cat = 'Missing exports / unimplemented calls'; Src = 'any'; Re = '(?i)no export|unimplemented|E_NOTIMPL|not implemented|not found in module|unresolved import|GetProcAddress\b.*(-> ?(0x0\b|NULL|null)|failed)|missing (export|function|entry)' },
  @{ Cat = 'Shader translation failures'; Src = 'any'; Re = 'shader_xlat FAIL|untranslatable|CreateGraphicsPSO_X counts: .*pso_fail=[1-9]|CreateComputePSO_X counts: .*fail=[1-9]' },
  @{ Cat = 'D3D12 debug layer / DRED / device removal'; Src = 'any'; Re = '(?i)D3D12 (ERROR|CORRUPTION|WARNING)|\bDRED\b|device ?removed|DEVICE_REMOVED|DEVICE_HUNG|DEVICE_RESET|DXGI_ERROR|GetDeviceRemovedReason' },
  @{ Cat = 'Network policy'; Src = 'compat'; Re = '\[net_guard\].*(refus|denied|deny|block|disabled|NOT restricted)|\[net_guard\] resolve ''(?!127\.0\.0\.1|localhost|::1)' },
  @{ Cat = 'First-chance exceptions'; Src = 'trace'; Re = '^\[EXC\] code=0x(?!406D1388|40010006|4001000A)' },
  @{ Cat = 'Compat errors'; Src = 'compat'; Re = '(?i)\bFAILED\b|\bfail(s|ed|ure)?\b|\berror\b|hr=0x8|\brejected\b|timed? ?out|\binvalid\b|\bpanic' },
  @{ Cat = 'Compat warnings'; Src = 'compat'; Re = '(?i)\bwarn(ing)?\b|unsupported|not supported|\bignored\b|fallback|\bdropped\b|\bskipp(ed|ing)\b' },
  @{ Cat = 'Title errors'; Src = 'ods'; Re = '(?i)\b(error|fatal|assert(ion)?( failed)?|exception|crash(ed)?)\b' },
  @{ Cat = 'Title warnings'; Src = 'ods'; Re = '(?i)\bwarn(ing)?\b' },
  @{ Cat = 'Harness errors'; Src = 'launcher'; Re = '(?i)exception|error|throw|cannot|missing|not found|failed' }
)
$BenignRe = [regex]::new('(?i)\b[\w.-]*(fail|error|err|reject|invalid|timeout|drop|skip)\w*[=:] ?0\b(?![.x\d])|\(0 (rejected|failed|errors?|dropped|skipped)\)|\(error [+-]?[\d.]+%\)|\btimeout=\d+(ms|s)?\b|\b0 (errors?|failures?|failed|rejected|dropped|skipped)\b', 'Compiled')
$IgnoreRe = [regex]'CreateGraphicsPSO_X counts: pso_ok=\d+ pso_fail=0 untranslatable=0|CreateComputePSO_X counts: .*fail=0\b|\bfail=0\b.*\bfail=0\b|err=0$'
foreach ($d in $DefectDefs) { $d.Rx = [regex]::new($d.Re, 'Compiled') }
foreach ($k in @($MilestoneDefs.Keys)) { foreach ($m in $MilestoneDefs[$k]) { $m.Rx = [regex]::new($m.Re, 'Compiled') } }
$UiRx = [regex]'UI State changing from \[([^\]]+)\] to \[([^\]]+)\]'
$FpsRx = [regex]'\[perf\] fps=([\d.]+)(?: ft_p50=([\d.]+)ms ft_p99=([\d.]+)ms)?'
$ExitRx = [regex]'\[trace\] EXIT_PROCESS code=(-?\d+) \((0x[0-9a-fA-F]+)\)'

function Get-Norm([string]$l) {
  $s = $l -replace '^\d{10,} ', ''
  $s = $s -replace '0x[0-9a-fA-F]+', '0x#' -replace '\b[0-9a-fA-F]{8,}\b', '#h' -replace '\d+', '#'
  if ($s.Length -gt 220) { $s = $s.Substring(0, 220) }
  return $s
}

function Get-SourceFiles([string]$Dir) {
  @(
    @{ Src = 'ods'; Path = (Join-Path $Dir 'run.txt') },
    @{ Src = 'trace'; Path = (Join-Path $Dir 'run.err.txt') },
    @{ Src = 'compat'; Path = (Join-Path $Dir 'tombox_compat.log') },
    @{ Src = 'launcher'; Path = (Join-Path $Dir 'launcher.txt') }
  )
}

function Invoke-Analyze([string]$Dir, [string]$Kind) {
  $defects = [ordered]@{}
  foreach ($d in $DefectDefs) { $defects[$d.Cat] = [ordered]@{} }
  $ms = [ordered]@{}
  $ui = New-Object System.Collections.Generic.List[string]
  $fps = New-Object System.Collections.Generic.List[double]
  $p99 = $null
  $exit = $null
  $lines = @{}
  foreach ($f in Get-SourceFiles $Dir) {
    if (-not (Test-Path -LiteralPath $f.Path)) { continue }
    $leaf = Split-Path $f.Path -Leaf
    $n = 0
    foreach ($l in [IO.File]::ReadLines($f.Path)) {
      $n++
      if (-not $l) { continue }
      foreach ($m in $MilestoneDefs[$Kind]) {
        if ($ms.Contains($m.Name)) { continue }
        if (($m.Src -eq 'any' -or $m.Src -eq $f.Src) -and $m.Rx.IsMatch($l)) { $ms[$m.Name] = "${leaf}:$n" }
      }
      if ($f.Src -eq 'ods') { $u = $UiRx.Match($l); if ($u.Success) { $ui.Add("$($u.Groups[1].Value) -> $($u.Groups[2].Value) (${leaf}:$n)") } }
      if ($f.Src -eq 'compat') { $fm = $FpsRx.Match($l); if ($fm.Success) { $fps.Add([double]$fm.Groups[1].Value); if ($fm.Groups[3].Success) { $p99 = $fm.Groups[3].Value } } }
      if ($f.Src -eq 'trace') { $em = $ExitRx.Match($l); if ($em.Success) { $exit = "$($em.Groups[1].Value) ($($em.Groups[2].Value))" } }
      if ($IgnoreRe.IsMatch($l)) { continue }
      $probe = $BenignRe.Replace($l, ' ')
      foreach ($d in $DefectDefs) {
        if ($d.Src -ne 'any' -and $d.Src -ne $f.Src) { continue }
        if (-not $d.Rx.IsMatch($probe)) { continue }
        $key = "$($f.Src)|" + (Get-Norm $l)
        $bucket = $defects[$d.Cat]
        if ($bucket.Contains($key)) { $bucket[$key].Count++ }
        else { $bucket[$key] = [pscustomobject]@{ Count = 1; Where = "${leaf}:$n"; Sample = ($l -replace '^\d{10,} ', '') } }
        break
      }
    }
    $lines[$leaf] = $n
  }
  [pscustomobject]@{ Defects = $defects; Milestones = $ms; Ui = $ui; Fps = $fps; P99 = $p99; Exit = $exit; Lines = $lines }
}

if (-not ('TbAudio' -as [type])) { Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public static class TbAudio {
  [ComImport, Guid("BCDE0395-E52F-467C-8E3D-C4579291692E")] class MMDeviceEnumerator {}
  [ComImport, Guid("A95664D2-9614-4F35-A746-DE8DB63617E6"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
  interface IMMDeviceEnumerator { int EnumAudioEndpoints(int flow, int mask, out IMMDeviceCollection c); }
  [ComImport, Guid("0BD7A1BE-7A1A-44DB-8397-CC5392387B5E"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
  interface IMMDeviceCollection { int GetCount(out uint n); int Item(uint i, out IMMDevice d); }
  [ComImport, Guid("D666063F-1587-4E43-81F1-B948E807363F"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
  interface IMMDevice { int Activate(ref Guid iid, int ctx, IntPtr p, [MarshalAs(UnmanagedType.IUnknown)] out object o); }
  [ComImport, Guid("77AA99A0-1BD6-484F-8BC7-2C654C9A9B6F"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
  interface IAudioSessionManager2 { int GetAudioSessionControl(IntPtr g, int f, out IntPtr c); int GetSimpleAudioVolume(IntPtr g, int f, out IntPtr v); int GetSessionEnumerator(out IAudioSessionEnumerator e); }
  [ComImport, Guid("E2F5BB11-0570-40CA-ACDD-3AA01277DEE8"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
  interface IAudioSessionEnumerator { int GetCount(out int n); int GetSession(int i, out IAudioSessionControl2 s); }
  [ComImport, Guid("bfb7ff88-7239-4fc9-8fa2-07c950be9c6d"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
  interface IAudioSessionControl2 {
    int GetState(out int s); int GetDisplayName(out IntPtr n); int SetDisplayName(string n, ref Guid g); int GetIconPath(out IntPtr p); int SetIconPath(string p, ref Guid g);
    int GetGroupingParam(out Guid g); int SetGroupingParam(ref Guid g, ref Guid c); int RegisterAudioSessionNotification(IntPtr n); int UnregisterAudioSessionNotification(IntPtr n);
    int GetSessionIdentifier(out IntPtr s); int GetSessionInstanceIdentifier(out IntPtr s); int GetProcessId(out uint pid);
  }
  [ComImport, Guid("87CE5498-68D6-44E5-9215-6DA47EF883D8"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
  interface ISimpleAudioVolume { int SetMasterVolume(float v, ref Guid g); int GetMasterVolume(out float v); int SetMute([MarshalAs(UnmanagedType.Bool)] bool m, ref Guid g); int GetMute([MarshalAs(UnmanagedType.Bool)] out bool m); }
  class Held { public uint Pid; public bool Orig; public ISimpleAudioVolume Vol; public string Id; }
  static List<Held> held = new List<Held>();
  static HashSet<string> heldIds = new HashSet<string>();
  public static int HeldCount { get { return held.Count; } }
  public static List<string> Restore() {
    var log = new List<string>();
    foreach (var h in held) {
      Guid g = Guid.Empty;
      try { int hr = h.Vol.SetMute(h.Orig, ref g); bool now; h.Vol.GetMute(out now); log.Add(string.Format("pid {0}: mute restored to {1} (hr=0x{2:x8}, now {3})", h.Pid, h.Orig, hr, now)); }
      catch (Exception e) { log.Add(string.Format("pid {0}: restore to {1} failed: {2}", h.Pid, h.Orig, e.Message)); }
      try { Marshal.ReleaseComObject(h.Vol); } catch {}
    }
    held.Clear(); heldIds.Clear();
    return log;
  }
  public static int Sessions(uint[] pids, bool mute, List<string> seen) {
    var want = new HashSet<uint>(pids);
    int hit = 0;
    if (mute) {
      var en0 = (IMMDeviceEnumerator)new MMDeviceEnumerator();
      IMMDeviceCollection col0; if (en0.EnumAudioEndpoints(0, 1, out col0) != 0) return -1;
      uint n0; col0.GetCount(out n0);
      for (uint i = 0; i < n0; i++) {
        IMMDevice dev; if (col0.Item(i, out dev) != 0) continue;
        Guid iid = typeof(IAudioSessionManager2).GUID; object o;
        if (dev.Activate(ref iid, 23, IntPtr.Zero, out o) != 0) continue;
        IAudioSessionEnumerator se; if (((IAudioSessionManager2)o).GetSessionEnumerator(out se) != 0) continue;
        int c; se.GetCount(out c);
        for (int k = 0; k < c; k++) {
          IAudioSessionControl2 s; if (se.GetSession(k, out s) != 0) continue;
          uint pid; s.GetProcessId(out pid);
          if (!want.Contains(pid)) continue;
          IntPtr idp; string id = pid.ToString() + ":" + k;
          if (s.GetSessionInstanceIdentifier(out idp) == 0 && idp != IntPtr.Zero) { id = Marshal.PtrToStringUni(idp); Marshal.FreeCoTaskMem(idp); }
          var v = (ISimpleAudioVolume)s; Guid g = Guid.Empty;
          if (!heldIds.Contains(id)) {
            bool orig; v.GetMute(out orig);
            heldIds.Add(id);
            held.Add(new Held { Pid = pid, Orig = orig, Vol = v, Id = id });
          }
          v.SetMute(true, ref g);
          bool m; v.GetMute(out m); if (m) hit++;
        }
      }
      return hit;
    }
    var en = (IMMDeviceEnumerator)new MMDeviceEnumerator();
    IMMDeviceCollection col; if (en.EnumAudioEndpoints(0, 1, out col) != 0) return -1;
    uint n; col.GetCount(out n);
    for (uint i = 0; i < n; i++) {
      IMMDevice dev; if (col.Item(i, out dev) != 0) continue;
      Guid iid = typeof(IAudioSessionManager2).GUID; object o;
      if (dev.Activate(ref iid, 23, IntPtr.Zero, out o) != 0) continue;
      IAudioSessionEnumerator se; if (((IAudioSessionManager2)o).GetSessionEnumerator(out se) != 0) continue;
      int c; se.GetCount(out c);
      for (int k = 0; k < c; k++) {
        IAudioSessionControl2 s; if (se.GetSession(k, out s) != 0) continue;
        uint pid; s.GetProcessId(out pid);
        if (seen != null) seen.Add(pid.ToString());
        if (!want.Contains(pid)) continue;
        var v = (ISimpleAudioVolume)s; Guid g = Guid.Empty;
        if (mute) v.SetMute(true, ref g);
        bool m; v.GetMute(out m); if (m || !mute) hit++;
      }
    }
    return hit;
  }
}
'@ }

function Get-Tree([int]$RootPid) {
  $all = @(Get-CimInstance Win32_Process -Property ProcessId, ParentProcessId, Name)
  $set = New-Object System.Collections.Generic.List[int]
  $set.Add($RootPid)
  $grew = $true
  while ($grew) {
    $grew = $false
    foreach ($p in $all) { if ($set.Contains([int]$p.ParentProcessId) -and -not $set.Contains([int]$p.ProcessId)) { $set.Add([int]$p.ProcessId); $grew = $true } }
  }
  return $set
}

function Stop-Tree([int]$RootPid, [string]$GameProc) {
  $tree = Get-Tree $RootPid
  $game = @(Get-Process -Name $GameProc -ErrorAction SilentlyContinue | Where-Object { $tree.Contains($_.Id) })
  foreach ($g in $game) { try { [void]$g.CloseMainWindow() } catch {} }
  $deadline = (Get-Date).AddSeconds(15)
  while ((Get-Date) -lt $deadline -and @($game | Where-Object { -not $_.HasExited }).Count) { Start-Sleep -Milliseconds 500; foreach ($g in $game) { $g.Refresh() } }
  $tree = Get-Tree $RootPid
  $order = @($tree | Sort-Object -Descending)
  foreach ($id in $order) { try { Stop-Process -Id $id -Force -ErrorAction Stop } catch {} }
  return [pscustomobject]@{ Closed = @($game | Where-Object { $_.HasExited }).Count; Killed = $order.Count }
}

function Read-New($st) {
  if (-not (Test-Path -LiteralPath $st.Path)) { return @() }
  $fs = [IO.File]::Open($st.Path, 'Open', 'Read', 'ReadWrite, Delete')
  try {
    if ($fs.Length -lt $st.Pos) { $st.Pos = 0; $st.Rest = '' }
    $len = [int][Math]::Min($fs.Length - $st.Pos, 64MB)
    if ($len -le 0) { return @() }
    [void]$fs.Seek($st.Pos, 'Begin')
    $buf = New-Object byte[] $len
    $n = $fs.Read($buf, 0, $len)
    $st.Pos += $n
  } finally { $fs.Close() }
  $parts = ($st.Rest + [Text.Encoding]::UTF8.GetString($buf, 0, $n)) -split "`n"
  $st.Rest = $parts[-1]
  if ($parts.Count -lt 2) { return @() }
  return $parts[0..($parts.Count - 2)]
}

function Test-ScriptNetwork([string]$Path) {
  $txt = Get-Content -LiteralPath $Path -Raw
  $hits = @()
  foreach ($m in [regex]::Matches($txt, '(?i)https?://[^\s''"]+|\b[a-z0-9-]+\.(com|net|org|io|gg|live|ubi|epicgames)\b|TOMBOX_ALLOW_NETWORK|\b(?!127\.0\.0\.1\b)(\d{1,3}\.){3}\d{1,3}\b')) { $hits += $m.Value }
  $redir = [regex]::Match($txt, "TOMBOX_NET_REDIRECT\s*=\s*'([^']*)'").Groups[1].Value
  $backend = [regex]::Match($txt, "TOMBOX_NET_BACKEND\s*=\s*'([^']*)'").Groups[1].Value
  [pscustomobject]@{ Hits = @($hits | Sort-Object -Unique); Redirect = $redir; Backend = $backend }
}

function Set-BatchEnv {
  Remove-Item Env:TOMBOX_ALLOW_NETWORK, Env:TOMBOX_NET_LOCAL_PORT -ErrorAction SilentlyContinue
  $env:TOMBOX_NET_REDIRECT = '127.0.0.1'
  $env:TOMBOX_NET_BACKEND = 'builtin'
  $env:TOMBOX_FN_GAMESERVER = '127.0.0.1:7777'
  if ($DbgLayer) {
    $env:TOMBOX_D3D12_DEBUG = '1'
    $env:TOMBOX_AGILITY_PATH = $Agility
    $env:TOMBOX_AGILITY_VERSION = "$AgilityVersion"
  } else {
    Remove-Item Env:TOMBOX_D3D12_DEBUG, Env:TOMBOX_AGILITY_PATH, Env:TOMBOX_AGILITY_VERSION -ErrorAction SilentlyContinue
  }
}

function Get-ArgList($t, [switch]$Dry) {
  $a = @('-NoProfile', '-File', (Join-Path $Root $t.Script))
  foreach ($k in $t.Args.Keys) { $a += "-$k"; $a += "$($t.Args[$k])" }
  if ($t.Xone -and $DbgLayer) { $a += '-Agility'; $a += $Agility; $a += '-AgilityVersion'; $a += "$AgilityVersion"; $a += '-D3d12Debug' }
  if ($Dry) { $a += '-DryRun' } else { $a += '-Seconds'; $a += "$($t.Seconds + 120)" }
  return $a
}

function Format-Arg([string]$s) { if ($s -match '[\s"]') { return '"' + ($s -replace '"', '\"') + '"' }; return $s }

New-Item -ItemType Directory -Force -Path $RunRoot | Out-Null
$sb = New-Object System.Text.StringBuilder
function W([string]$s = '') { [void]$sb.AppendLine($s) }

Say "batch test $Mode -> $RunRoot" 'Cyan'
$oldEnv = @{}
foreach ($v in 'TOMBOX_ALLOW_NETWORK', 'TOMBOX_NET_LOCAL_PORT', 'TOMBOX_NET_REDIRECT', 'TOMBOX_NET_BACKEND', 'TOMBOX_FN_GAMESERVER', 'TOMBOX_D3D12_DEBUG', 'TOMBOX_AGILITY_PATH', 'TOMBOX_AGILITY_VERSION') { $oldEnv[$v] = [Environment]::GetEnvironmentVariable($v) }
Set-BatchEnv

$pre = New-Object System.Collections.Generic.List[string]
$mdbg = Join-Path $Root 'target\release\mdbg.exe'
if (-not (Test-Path $mdbg)) { $pre.Add("mdbg.exe missing at $mdbg (titles cannot launch)") }
if ($DbgLayer) {
  foreach ($f in 'D3D12Core.dll', 'd3d12SDKLayers.dll') { if (-not (Test-Path (Join-Path $Agility $f))) { $pre.Add("D3D12 debug layer requested but $f is missing in $Agility") } }
}
try { $seen = New-Object System.Collections.Generic.List[string]; $r = [TbAudio]::Sessions([uint32[]]@(), $false, $seen); Say "audio mute check: WASAPI session enumeration ok ($($seen.Count) sessions visible)" }
catch { $pre.Add("audio mute unavailable: $($_.Exception.Message) (titles would be audible; pass aborted)") }

$build = 'skipped (-SkipBuild)'
if (-not $SkipBuild) {
  Say 'rebuild_release.ps1 (cargo build --release -j 6)' 'Cyan'
  $bl = Join-Path $RunRoot 'rebuild_release.txt'
  & powershell.exe -NoProfile -File (Join-Path $Root 'tools\dev\rebuild_release.ps1') *> $bl
  $bcode = $LASTEXITCODE
  $btxt = Get-Content $bl
  $btxt | ForEach-Object { Say "  $_" 'DarkGray' }
  $build = "exit $bcode; " + (($btxt | Where-Object { $_ -match 'DLL crates|FAIL|ERROR|NO DLL|all crates built' }) -join '; ')
}

$results = @()
foreach ($t in $Titles) {
  Say "=== $($t.Name)" 'Cyan'
  $dir = Join-Path $RunRoot $t.Slug
  New-Item -ItemType Directory -Force -Path $dir | Out-Null
  $res = [ordered]@{ Title = $t; Dir = $dir; Status = ''; Notes = (New-Object System.Collections.Generic.List[string]); Live = [ordered]@{}; Plan = @(); Net = $null; Exit = $null; Elapsed = 0; Dumps = @(); Analysis = $null; Tail = @() }
  $script = Join-Path $Root $t.Script
  $res.Net = Test-ScriptNetwork $script
  if ($res.Net.Hits.Count) { $res.Notes.Add("run script references external hosts/addresses: $($res.Net.Hits -join ', ')") }
  if ($res.Net.Redirect -ne '127.0.0.1' -or $res.Net.Backend -ne 'builtin') { $res.Notes.Add("run script network env is redirect='$($res.Net.Redirect)' backend='$($res.Net.Backend)', expected 127.0.0.1/builtin") }
  if (-not $t.Exe -or -not (Test-Path -LiteralPath $t.Exe)) { $res.Notes.Add("title exe not found in the dump ($($t.Exe))") }
  $busy = @(Get-Process -Name $t.Proc -ErrorAction SilentlyContinue)
  if ($busy.Count) { $res.Notes.Add("$($t.Proc).exe already running (pid $($busy.Id -join ',')): another run owns it, not touched") }

  $planTxt = Join-Path $dir 'stage_plan.txt'
  $pa = Get-ArgList $t -Dry
  & powershell.exe @pa *> $planTxt
  $plan = @(Get-Content $planTxt)
  $res.Plan = @($plan | Where-Object { $_ -match '^\s+(missing|stage|skip|script)\s' })
  $missing = @($res.Plan | Where-Object { $_ -match '^\s+missing\s' })
  foreach ($m in $missing) { $res.Notes.Add("staging: $($m.Trim())") }
  Say ("  staging plan: {0} stage, {1} script, {2} skip, {3} missing" -f @($res.Plan | Where-Object { $_ -match '^\s+stage' }).Count, @($res.Plan | Where-Object { $_ -match '^\s+script' }).Count, @($res.Plan | Where-Object { $_ -match '^\s+skip' }).Count, $missing.Count)
  if ($t.Slug -eq 'fortnite-play') {
    $gs = @($plan | Where-Object { $_ -match 'TOMBOX_FN_GAMESERVER=' }) -join ''
    if ($gs -notmatch '127\.0\.0\.1:7777') { $res.Notes.Add("Fortnite game server is not loopback: $gs") }
  }
  $la = Get-ArgList $t
  $cmd = 'powershell.exe ' + (($la | ForEach-Object { Format-Arg $_ }) -join ' ')
  Say "  launch: $cmd" 'DarkGray'
  Say "  timeout $($t.Seconds)s; logs $($t.LogDir)" 'DarkGray'

  if ($DryRun) {
    $res.Status = 'dry run: not launched'
    if (Test-Path (Join-Path $t.LogDir 'tombox_compat.log')) {
      foreach ($f in Get-SourceFiles $t.LogDir) { if ($f.Src -ne 'launcher' -and (Test-Path $f.Path)) { Copy-Item $f.Path $dir } }
      $res.Analysis = Invoke-Analyze $dir $t.Kind
      $res.Exit = $res.Analysis.Exit
      $res.Notes.Add("analysis below is a parser check on the PREVIOUS logs in $($t.LogDir) (last written $((Get-Item (Join-Path $t.LogDir 'tombox_compat.log')).LastWriteTime)), not this pass")
    } else { $res.Notes.Add("no previous logs in $($t.LogDir) to parser-check") }
    $results += [pscustomobject]$res
    continue
  }
  if ($busy.Count -or $pre.Count) {
    $res.Status = if ($busy.Count) { 'not run: title already running' } else { 'not run: preflight failed' }
    $results += [pscustomobject]$res
    continue
  }

  foreach ($f in Get-SourceFiles $t.LogDir) { if ($f.Src -ne 'launcher') { Remove-Item -LiteralPath $f.Path -Force -ErrorAction SilentlyContinue } }
  $launcher = Join-Path $dir 'launcher.txt'
  $start = Get-Date
  $ps = Start-Process -FilePath 'powershell.exe' -ArgumentList $la -RedirectStandardOutput $launcher -RedirectStandardError (Join-Path $dir 'launcher.err.txt') -WindowStyle Hidden -PassThru
  $streams = @(foreach ($f in Get-SourceFiles $t.LogDir) { if ($f.Src -ne 'launcher') { [pscustomobject]@{ Src = $f.Src; Path = $f.Path; Pos = 0L; Rest = '' } } })
  $lastBeat = $start
  $lastFps = ''
  $muted = $false
  $timedOut = $false
  $tick = 0
  $tree = Get-Tree $ps.Id
  $nextLog = $start
  $nextTree = $start
  while ($true) {
    $el = ((Get-Date) - $start).TotalSeconds
    Start-Sleep -Milliseconds $(if ($el -lt 120) { 100 } else { 500 })
    $el = ((Get-Date) - $start).TotalSeconds
    if ((Get-Date) -ge $nextTree) { $tree = Get-Tree $ps.Id; $nextTree = (Get-Date).AddSeconds($(if ($el -lt 120) { 1 } else { 2 })) }
    $pids = @($tree) + @(Get-Process -Name $t.Proc -ErrorAction SilentlyContinue | Where-Object { $_.StartTime -ge $start } | ForEach-Object { $_.Id })
    try { $mh = [TbAudio]::Sessions([uint32[]]$pids, $true, $null); if ($mh -gt 0 -and -not $muted) { $muted = $true; Say ("  +{0:N1}s audio session muted (original state saved)" -f $el) } } catch {}
    if ((Get-Date) -lt $nextLog) { $ps.Refresh(); if ($ps.HasExited) { break }; continue }
    $nextLog = (Get-Date).AddMilliseconds(500)
    foreach ($s in $streams) {
      foreach ($l in Read-New $s) {
        foreach ($m in $MilestoneDefs[$t.Kind]) {
          if ($res.Live.Contains($m.Name)) { continue }
          if (($m.Src -eq 'any' -or $m.Src -eq $s.Src) -and $m.Rx.IsMatch($l)) { $res.Live[$m.Name] = [Math]::Round($el, 1); Say ("  +{0:N0}s milestone: {1}" -f $el, $m.Name) 'Green' }
        }
        if ($s.Src -eq 'ods') { $u = $UiRx.Match($l); if ($u.Success) { Say ("  +{0:N0}s UI State {1} -> {2}" -f $el, $u.Groups[1].Value, $u.Groups[2].Value) 'Green' } }
        if ($s.Src -eq 'compat') { $fm = $FpsRx.Match($l); if ($fm.Success) { $lastFps = $fm.Groups[1].Value } }
        if ($s.Src -eq 'trace' -and $l -match 'firstChance=0|ACCESS_VIOLATION|EXIT_PROCESS|\[HANG\]|too many exceptions') { Say ("  +{0:N0}s {1}" -f $el, $l.Trim()) 'Yellow' }
      }
    }
    if (((Get-Date) - $lastBeat).TotalSeconds -ge 60) { $lastBeat = Get-Date; Say ("  +{0:N0}s running; fps {1}; muted {2}" -f $el, $(if ($lastFps) { $lastFps } else { '-' }), $muted) 'DarkGray' }
    $ps.Refresh()
    if ($ps.HasExited) { break }
    if ($el -ge $t.Seconds) { $timedOut = $true; break }
  }
  $res.Elapsed = [Math]::Round(((Get-Date) - $start).TotalSeconds, 1)
  if ($timedOut) {
    $k = Stop-Tree $ps.Id $t.Proc
    $res.Notes.Add("timeout after $($t.Seconds)s: window close requested, $($k.Closed) game process(es) closed, $($k.Killed) process(es) in the run tree stopped")
  } else {
    $left = @(Get-Process -Name $t.Proc -ErrorAction SilentlyContinue | Where-Object { $_.StartTime -ge $start })
    foreach ($g in $left) { try { Stop-Process -Id $g.Id -Force } catch {} }
  }
  Start-Sleep -Seconds 2
  try { foreach ($m in [TbAudio]::Restore()) { $res.Notes.Add("audio: $m") } } catch { $res.Notes.Add("audio: restoring mute state failed: $($_.Exception.Message)") }
  if (-not $muted) { $res.Notes.Add('audio: no audio session of the run was seen, nothing muted or restored') }
  foreach ($f in Get-SourceFiles $t.LogDir) { if ($f.Src -ne 'launcher' -and (Test-Path -LiteralPath $f.Path)) { Copy-Item -LiteralPath $f.Path $dir -Force } }
  $dumpDirs = @((Join-Path $env:LOCALAPPDATA 'CrashDumps'), $t.LogDir) + $t.Saved
  $res.Dumps = @(foreach ($d in $dumpDirs) { if (Test-Path -LiteralPath $d) { Get-ChildItem -LiteralPath $d -Recurse -File -Include *.dmp, *.mdmp -ErrorAction SilentlyContinue | Where-Object { $_.LastWriteTime -ge $start } } })
  foreach ($d in $res.Dumps) { Copy-Item -LiteralPath $d.FullName $dir -Force }
  $res.Analysis = Invoke-Analyze $dir $t.Kind
  $res.Exit = $res.Analysis.Exit
  $compat = Join-Path $dir 'tombox_compat.log'
  if (Test-Path $compat) { $res.Tail = @(Get-Content $compat -Tail 40) }
  $crashed = $res.Exit -and $res.Exit -notmatch '^0 '
  $res.Status = if ($timedOut) { if ($crashed) { 'timeout (title exit code set)' } else { 'ran to timeout' } } elseif ($crashed) { "crashed (exit $($res.Exit))" } elseif ($res.Exit) { "exited (exit $($res.Exit))" } else { 'ended without an exit record' }
  Say "  result: $($res.Status) after $($res.Elapsed)s" $(if ($crashed) { 'Red' } else { 'Cyan' })
  $results += [pscustomobject]$res
}

foreach ($v in $oldEnv.Keys) { [Environment]::SetEnvironmentVariable($v, $oldEnv[$v]) }
try { if ([TbAudio]::HeldCount) { foreach ($m in [TbAudio]::Restore()) { Say "audio: $m" } } } catch {}

W "# Batch test report"
W
W "- Mode: **$Mode**"
W "- Started: $($BatchStart.ToString('yyyy-MM-dd HH:mm:ss')), finished: $((Get-Date).ToString('yyyy-MM-dd HH:mm:ss'))"
W "- Run folder: ``$RunRoot`` (all copied logs, stage plans, dumps)"
W "- Release build: $(ConvertTo-MdText $build)"
W "- Network: forced offline/private (TOMBOX_NET_REDIRECT=127.0.0.1, TOMBOX_NET_BACKEND=builtin, TOMBOX_ALLOW_NETWORK removed, Fortnite game server 127.0.0.1:7777); enforced in-process by compat-common net_guard"
W "- D3D12 debug layer: $(if ($DbgLayer) { "ON via Agility SDK $AgilityVersion ($Agility); FPS is measured with the layer on" } else { 'off (-NoD3d12Debug)' }). DRED: not enabled by any compat crate; only device-removal log lines are captured"
W "- Audio: each title's WASAPI session is muted from the batch (polled every 0.1 s for the first 2 min, then 0.5 s; each session's prior mute state is restored after the title exits); input: none injected; windows: shown by the title"
foreach ($p in $pre) { W "- **Preflight problem:** $(ConvertTo-MdText $p)" }
W
W "| Title | Result | Furthest milestone | FPS min/avg/max (last, p99 ms) | Exit | Defect groups |"
W "|---|---|---|---|---|---|"
foreach ($r in $results) {
  $a = $r.Analysis
  $order = @($MilestoneDefs[$r.Title.Kind] | ForEach-Object { $_.Name })
  $far = '-'
  $fps = '-'
  $groups = 0
  if ($a) {
    foreach ($n in $order) { if ($a.Milestones.Contains($n)) { $far = $n } }
    if ($a.Fps.Count) { $st = $a.Fps | Measure-Object -Minimum -Maximum -Average; $fps = "{0:N1}/{1:N1}/{2:N1} ({3:N1}, {4})" -f $st.Minimum, $st.Average, $st.Maximum, $a.Fps[$a.Fps.Count - 1], $(if ($a.P99) { $a.P99 } else { '-' }) }
    foreach ($c in $a.Defects.Keys) { $groups += $a.Defects[$c].Count }
  }
  W "| $(ConvertTo-MdText $r.Title.Name) | $(ConvertTo-MdText $r.Status) | $far | $fps | $(ConvertTo-MdText $(if ($r.Exit) { $r.Exit } else { '-' })) | $groups |"
}
W
foreach ($r in $results) {
  $t = $r.Title
  W "## $($t.Name)"
  W
  W "- Status: $(ConvertTo-MdText $r.Status)$(if ($r.Elapsed) { " after $($r.Elapsed) s" })"
  W "- Launch: ``powershell.exe $((Get-ArgList $t | Select-Object -Skip 2) -join ' ')`` (timeout $($t.Seconds) s, window visible, no input)"
  W "- Logs: ``$($r.Dir)``"
  foreach ($n in $r.Notes) { W "- $(ConvertTo-MdText $n)" }
  if ($r.Dumps.Count) { W "- Crash dumps: $(($r.Dumps | ForEach-Object { '`' + $_.Name + '`' }) -join ', ')" } elseif (-not $DryRun) { W '- Crash dumps: none written (mdbg does not write minidumps; none in CrashDumps or the title Saved folders)' }
  W
  $a = $r.Analysis
  if (-not $a) { W '_No logs analysed._'; W; continue }
  W '### Milestones'
  W
  W '| Milestone | Reached | Live time | Evidence |'
  W '|---|---|---|---|'
  foreach ($m in $MilestoneDefs[$t.Kind]) {
    $ev = if ($a.Milestones.Contains($m.Name)) { $a.Milestones[$m.Name] } else { '' }
    $lt = if ($r.Live.Contains($m.Name)) { "+$($r.Live[$m.Name]) s" } else { '' }
    W "| $($m.Name) | $(if ($ev) { 'yes' } else { 'no' }) | $lt | $ev |"
  }
  if ($t.Kind -eq 'generic') { W; W '_login/menu/match milestones for this title are keyword heuristics; check the evidence line._' }
  W
  if ($a.Ui.Count) {
    W '### UI State timeline'
    W
    foreach ($u in $a.Ui) { W "- $(ConvertTo-MdText $u)" }
    W
  }
  W "### Defects ($(($a.Lines.Keys | ForEach-Object { "$_ $($a.Lines[$_]) lines" }) -join ', '))"
  W
  foreach ($c in $a.Defects.Keys) {
    $b = $a.Defects[$c]
    if (-not $b.Count) { continue }
    $tot = 0; foreach ($e in $b.Values) { $tot += $e.Count }
    W "#### $c ($($b.Count) distinct, $tot lines)"
    W
    W '| Count | First at | Sample |'
    W '|---|---|---|'
    $i = 0
    foreach ($e in ($b.Values | Sort-Object Count -Descending)) {
      if ($i -ge $MaxEntriesPerCategory) { W "| | | ... $($b.Count - $i) more distinct entries in the logs |"; break }
      $s = $e.Sample; if ($s.Length -gt 300) { $s = $s.Substring(0, 300) + '...' }
      W "| $($e.Count) | $($e.Where) | ``$(ConvertTo-MdText $s)`` |"
      $i++
    }
    W
  }
  if ($r.Tail.Count -and $r.Status -match 'crash|exit code') {
    W '### Compat log tail at exit'
    W
    W '```'
    foreach ($l in $r.Tail) { W ($l -replace '```', "'''") }
    W '```'
    W
  }
}
W '## Harness notes'
W
W '- mdbg terminates the title after more than 400 exceptions or when MDBG_LIMIT (set to 86400 s by the run scripts) expires; such a stop shows as "too many exceptions" or "trace limit" under Crashes and hangs.'
W '- Defects are grouped by category, then deduplicated after replacing numbers, hex values, hashes and quoted names; "First at" is the file:line of the first occurrence in the copied logs.'
W '- First-chance exceptions with codes 0x406D1388, 0x40010006 and 0x4001000A (thread naming, debug print) are not listed.'
[IO.File]::WriteAllText($Report, $sb.ToString(), (New-Object System.Text.UTF8Encoding $false))
Copy-Item $Report (Join-Path $RunRoot 'batch_report.md') -Force
Say "report: $Report" 'Cyan'
