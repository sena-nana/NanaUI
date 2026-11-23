# Real-window input acceptance for IME composition (#271) and pen input (#276).
#
# Run with Windows PowerShell 5.1:
#   powershell -NoProfile -ExecutionPolicy Bypass -Mta -File scripts/validate-input-acceptance.ps1 `
#     -ExePath target/release/examples/input-acceptance-probe.exe
#
# It starts input-acceptance-probe, operates only that window, and writes the
# probe's event trace, UI Automation snapshots, screenshots and summary.json
# to -OutputDirectory. The IME part types through the installed system IMEs
# (Microsoft Pinyin 0804 in Chinese mode, Microsoft IME Japanese 0411) with SendInput key
# strokes, so preedit, candidates and commit come from the real IME. The pen
# part injects synthetic pen frames with InjectSyntheticPointerInput: it
# exercises the native WM_POINTER path NanaUI reads, not a physical digitizer.
# A physical pen run uses the same probe by hand (see docs/reference/input-conformance.md).
# Leave the keyboard and mouse alone while it runs: injected key strokes go
# to the foreground window, and typing at the same time mixes into the trace.
# It stops typing as soon as the probe loses the foreground.
param(
    [Parameter(Mandatory = $true)][string]$ExePath,
    [string]$OutputDirectory = 'target/acceptance/input',
    [ValidateSet('All', 'Ime', 'Pen')][string]$Part = 'All'
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
Add-Type @'
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

public static class NanaInputProbe {
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct Point { public int X, Y; }
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr window, out Rect rect);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr window, out Rect rect);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr window, ref Point point);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] public static extern IntPtr LoadKeyboardLayout(string id, uint flags);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr window, uint message, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern IntPtr GetKeyboardLayout(uint thread);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window, out uint process);
    private delegate bool EnumCallback(IntPtr window, IntPtr data);
    [DllImport("user32.dll")] private static extern bool EnumWindows(EnumCallback callback, IntPtr data);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] private static extern int GetWindowText(IntPtr window, StringBuilder text, int count);
    public static IntPtr Find(int process, string title) {
        IntPtr result = IntPtr.Zero;
        EnumWindows(delegate(IntPtr window, IntPtr data) {
            uint owner; GetWindowThreadProcessId(window, out owner);
            if (owner != process) return true;
            var text = new StringBuilder(512); GetWindowText(window, text, text.Capacity);
            if (text.ToString() != title) return true;
            result = window; return false;
        }, IntPtr.Zero);
        return result;
    }
    [DllImport("user32.dll")] static extern bool AttachThreadInput(uint attach, uint to, bool on);
    [DllImport("user32.dll")] static extern bool BringWindowToTop(IntPtr window);
    [DllImport("kernel32.dll")] static extern uint GetCurrentThreadId();
    // SetForegroundWindow is refused while another process holds the
    // foreground lock; sharing that thread's input state lifts the refusal.
    public static bool ForceForeground(IntPtr window) {
        if (SetForegroundWindow(window) && GetForegroundWindow() == window) return true;
        uint process;
        uint foreground = GetWindowThreadProcessId(GetForegroundWindow(), out process);
        uint current = GetCurrentThreadId();
        AttachThreadInput(current, foreground, true);
        BringWindowToTop(window);
        SetForegroundWindow(window);
        AttachThreadInput(current, foreground, false);
        return GetForegroundWindow() == window;
    }
    public static string Layout(IntPtr window) {
        uint process;
        uint thread = GetWindowThreadProcessId(window, out process);
        return GetKeyboardLayout(thread).ToInt64().ToString("X8");
    }
    public static IntPtr CurrentHkl(IntPtr window) {
        uint process;
        return GetKeyboardLayout(GetWindowThreadProcessId(window, out process));
    }
    public static void RestoreHkl(IntPtr window, IntPtr hkl) {
        PostMessage(window, 0x0050, IntPtr.Zero, hkl);
    }
    public static void SwitchLayout(IntPtr window, string klid) {
        IntPtr hkl = LoadKeyboardLayout(klid, 1);
        PostMessage(window, 0x0050, IntPtr.Zero, hkl); // WM_INPUTLANGCHANGEREQUEST
    }

    // --- keyboard -------------------------------------------------------
    [StructLayout(LayoutKind.Sequential)] struct KeyboardInput { public ushort Vk; public ushort Scan; public uint Flags; public uint Time; public IntPtr Extra; }
    [StructLayout(LayoutKind.Sequential)] struct MouseInput { public int Dx; public int Dy; public uint Data; public uint Flags; public uint Time; public IntPtr Extra; }
    [StructLayout(LayoutKind.Explicit)] struct InputUnion { [FieldOffset(0)] public KeyboardInput Key; [FieldOffset(0)] public MouseInput Mouse; }
    [StructLayout(LayoutKind.Sequential)] struct Input { public uint Type; public InputUnion U; }
    [DllImport("user32.dll", SetLastError=true)] static extern uint SendInput(uint count, Input[] inputs, int size);
    [DllImport("user32.dll")] static extern uint MapVirtualKey(uint code, uint map);
    public static IntPtr Target = IntPtr.Zero;
    public static void Key(ushort vk) {
        // Key strokes go to the foreground window: never type into anything but the probe.
        if (Target != IntPtr.Zero && GetForegroundWindow() != Target) throw new Exception("probe lost the foreground; stopped typing");
        var inputs = new Input[2];
        ushort scan = (ushort)MapVirtualKey(vk, 0);
        inputs[0].Type = 1; inputs[0].U.Key.Vk = vk; inputs[0].U.Key.Scan = scan;
        inputs[1].Type = 1; inputs[1].U.Key.Vk = vk; inputs[1].U.Key.Scan = scan; inputs[1].U.Key.Flags = 2;
        if (SendInput(2, inputs, Marshal.SizeOf(typeof(Input))) != 2) throw new Exception("SendInput key failed " + Marshal.GetLastWin32Error());
        Thread.Sleep(60);
    }
    [StructLayout(LayoutKind.Sequential)] struct LastInput { public uint Size; public uint Time; }
    [DllImport("user32.dll")] static extern bool GetLastInputInfo(ref LastInput info);
    // Whether the system accepts injected key strokes from this process: some
    // endpoint policies drop them silently while SendInput still returns success.
    public static bool KeyboardInjectionWorks() {
        var before = new LastInput(); before.Size = 8; GetLastInputInfo(ref before);
        Thread.Sleep(30);
        Key(0x87); // F24: types nothing, and unlike Shift does not toggle Pinyin's Chinese/English mode
        var after = new LastInput(); after.Size = 8; GetLastInputInfo(ref after);
        return after.Time != before.Time;
    }
    public static void Type(string letters) {
        foreach (char c in letters.ToUpperInvariant()) Key((ushort)c);
    }
    public static void MouseMove(int x, int y) {
        SetCursorPos(x, y);
        var inputs = new Input[1];
        inputs[0].Type = 0; inputs[0].U.Mouse.Flags = 0x0001; // MOUSEEVENTF_MOVE, relative 0
        SendInput(1, inputs, Marshal.SizeOf(typeof(Input)));
        Thread.Sleep(30);
    }
    public static void MouseClick(int x, int y) {
        SetCursorPos(x, y);
        var inputs = new Input[2];
        inputs[0].Type = 0; inputs[0].U.Mouse.Flags = 0x0002;
        inputs[1].Type = 0; inputs[1].U.Mouse.Flags = 0x0004;
        SendInput(2, inputs, Marshal.SizeOf(typeof(Input)));
        Thread.Sleep(80);
    }

    // --- synthetic pen --------------------------------------------------
    [StructLayout(LayoutKind.Sequential)]
    public struct PointerInfo {
        public uint PointerType; public uint PointerId; public uint FrameId; public uint PointerFlags;
        public IntPtr SourceDevice; public IntPtr HwndTarget;
        public Point PixelLocation; public Point HimetricLocation; public Point PixelLocationRaw; public Point HimetricLocationRaw;
        public uint Time; public uint HistoryCount; public int InputData; public uint KeyStates;
        public ulong PerformanceCount; public int ButtonChangeType;
    }
    [StructLayout(LayoutKind.Sequential)]
    public struct PenInfo {
        public PointerInfo Pointer; public uint PenFlags; public uint PenMask;
        public uint Pressure; public uint Rotation; public int TiltX; public int TiltY;
    }
    [StructLayout(LayoutKind.Explicit, Size = 152)]
    public struct PointerTypeInfo { [FieldOffset(0)] public uint Type; [FieldOffset(8)] public PenInfo Pen; }
    [DllImport("user32.dll", SetLastError=true)] static extern IntPtr CreateSyntheticPointerDevice(uint type, uint maxCount, uint feedback);
    [DllImport("user32.dll", SetLastError=true)] static extern bool InjectSyntheticPointerInput(IntPtr device, PointerTypeInfo[] info, uint count);
    [DllImport("user32.dll")] static extern void DestroySyntheticPointerDevice(IntPtr device);
    static IntPtr pen = IntPtr.Zero;
    public const uint InRange = 0x2, InContact = 0x4, FirstButton = 0x10, SecondButton = 0x20, Primary = 0x2000,
        Down = 0x10000, Update = 0x20000, Up = 0x40000, Canceled = 0x8000;
    public const uint Barrel = 0x1, Inverted = 0x2, Eraser = 0x4;
    public static void PenOpen() {
        if (pen != IntPtr.Zero) return;
        pen = CreateSyntheticPointerDevice(3, 1, 1); // PT_PEN, POINTER_FEEDBACK_DEFAULT
        if (pen == IntPtr.Zero) throw new Exception("CreateSyntheticPointerDevice failed " + Marshal.GetLastWin32Error());
    }
    public static void PenClose() { if (pen != IntPtr.Zero) { DestroySyntheticPointerDevice(pen); pen = IntPtr.Zero; } }
    // button change: 0 none, 1 first down, 2 first up, 3 second down, 4 second up
    public static void PenFrame(int x, int y, uint flags, uint penFlags, uint pressure, int tiltX, int tiltY, int buttonChange) {
        var info = new PointerTypeInfo[1];
        info[0].Type = 3;
        info[0].Pen.Pointer.PointerType = 3;
        info[0].Pen.Pointer.PointerId = 0;
        info[0].Pen.Pointer.PointerFlags = flags;
        info[0].Pen.Pointer.PixelLocation.X = x; info[0].Pen.Pointer.PixelLocation.Y = y;
        info[0].Pen.Pointer.ButtonChangeType = buttonChange;
        info[0].Pen.PenFlags = penFlags;
        info[0].Pen.PenMask = 0x1 | 0x4 | 0x8; // pressure, tilt x, tilt y
        info[0].Pen.Pressure = pressure;
        info[0].Pen.TiltX = tiltX; info[0].Pen.TiltY = tiltY;
        if (!InjectSyntheticPointerInput(pen, info, 1)) throw new Exception("InjectSyntheticPointerInput failed " + Marshal.GetLastWin32Error() + " flags=0x" + flags.ToString("X"));
        Thread.Sleep(16);
    }

    // --- probe process --------------------------------------------------
    static readonly List<string> lines = new List<string>();
    static Process probe;
    public static Process Start(string exe, string log) {
        var info = new ProcessStartInfo(exe);
        info.UseShellExecute = false; info.RedirectStandardInput = true; info.RedirectStandardOutput = true;
        info.StandardOutputEncoding = new UTF8Encoding(false);
        info.WorkingDirectory = Path.GetDirectoryName(exe);
        probe = Process.Start(info);
        var writer = new StreamWriter(log, false, new UTF8Encoding(false));
        writer.AutoFlush = true;
        var reader = new Thread(() => {
            string line;
            while ((line = probe.StandardOutput.ReadLine()) != null) {
                lock (lines) lines.Add(line);
                writer.WriteLine(line);
            }
            writer.Dispose();
        });
        reader.IsBackground = true; reader.Start();
        return probe;
    }
    public static int Count() { lock (lines) return lines.Count; }
    public static string[] Since(int start) { lock (lines) return lines.GetRange(start, lines.Count - start).ToArray(); }
    public static void Send(string command) {
        var bytes = new UTF8Encoding(false).GetBytes(command + "\n");
        probe.StandardInput.BaseStream.Write(bytes, 0, bytes.Length);
        probe.StandardInput.BaseStream.Flush();
    }
}
'@

[NanaInputProbe]::SetProcessDPIAware() | Out-Null
$exe = (Resolve-Path -LiteralPath $ExePath).Path
$output = [IO.Path]::GetFullPath($OutputDirectory)
[IO.Directory]::CreateDirectory($output) | Out-Null
$results = [ordered]@{}
$failures = New-Object System.Collections.Generic.List[string]

function Wait-For([scriptblock]$Check, [string]$Description, [int]$Seconds = 10) {
    $deadline = [DateTime]::UtcNow.AddSeconds($Seconds)
    do {
        $value = & $Check
        if ($value) { return $value }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Timed out waiting for $Description"
}
function Events([int]$Start) {
    [NanaInputProbe]::Since($Start) | ForEach-Object { $_ | ConvertFrom-Json }
}
function Inputs([int]$Start) { Events $Start | Where-Object { $_.event -eq 'input' } | ForEach-Object { $_.payload } }
function Editor-State() {
    Events 0 | Where-Object { $_.event -eq 'editor' } | Select-Object -Last 1 | ForEach-Object { $_.state }
}
function Check([string]$Name, [bool]$Pass, $Detail) {
    $results[$Name] = [ordered]@{ pass = $Pass; detail = $Detail }
    if (-not $Pass) { $failures.Add($Name) }
    Write-Host ("{0,-44} {1}" -f $Name, $(if ($Pass) { 'pass' } else { 'FAIL' }))
}
function Shot([string]$Name, [int]$ExtraBelow = 0) {
    [NanaInputProbe+Rect]$rect = New-Object NanaInputProbe+Rect
    [NanaInputProbe]::GetWindowRect($script:handle, [ref]$rect) | Out-Null
    $width = $rect.Right - $rect.Left; $height = $rect.Bottom - $rect.Top + $ExtraBelow
    $bitmap = New-Object System.Drawing.Bitmap $width, $height
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $graphics.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $bitmap.Size)
    $bitmap.Save((Join-Path $output "$Name.png"), [System.Drawing.Imaging.ImageFormat]::Png)
    $graphics.Dispose(); $bitmap.Dispose()
}
function Client-Point([double]$LogicalX, [double]$LogicalY) {
    $scale = [NanaInputProbe]::GetDpiForWindow($script:handle) / 96.0
    $point = New-Object NanaInputProbe+Point
    $point.X = [int]($LogicalX * $scale); $point.Y = [int]($LogicalY * $scale)
    [NanaInputProbe]::ClientToScreen($script:handle, [ref]$point) | Out-Null
    $point
}
function Focus-Probe() {
    Wait-For { [NanaInputProbe]::ForceForeground($script:handle) } 'probe foreground' | Out-Null
    [NanaInputProbe]::Send('focus')
    Wait-For { (Editor-State).focused } 'editor focus' | Out-Null
}
function Editor-Uia() {
    $window = [System.Windows.Automation.AutomationElement]::FromHandle($script:handle)
    $condition = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::NameProperty, 'IME editor')
    $window.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $condition)
}

$process = [NanaInputProbe]::Start($exe, (Join-Path $output 'trace.jsonl'))
try {
    $script:handle = Wait-For {
        $found = [NanaInputProbe]::Find($process.Id, 'NanaUI Input Acceptance Probe')
        if ($found -ne [IntPtr]::Zero) { $found }
    } 'probe window' 20
    Wait-For { Editor-State } 'first presented frame' 20 | Out-Null
    [NanaInputProbe]::Target = $script:handle
    $originalHkl = [NanaInputProbe]::CurrentHkl($script:handle)
    $results['environment'] = [ordered]@{
        os = (Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion' | ForEach-Object { "$($_.ProductName) $($_.DisplayVersion) build $($_.CurrentBuild).$($_.UBR)" })
        dpi = [NanaInputProbe]::GetDpiForWindow($script:handle)
        adapter = (Events 0 | Where-Object { $_.event -eq 'adapter' } | Select-Object -First 1).backend
    }

    $keyboardWorks = $false
    if ($Part -in 'All', 'Ime') {
        Focus-Probe
        $keyboardWorks = [NanaInputProbe]::KeyboardInjectionWorks()
        if (-not $keyboardWorks) {
            $results['ime'] = [ordered]@{ pass = $null; detail = 'not executed: the system drops injected key strokes from this process (SendInput reports success, last-input time does not move); run the IME steps by hand' }
            Write-Host 'ime: not executed (keyboard injection blocked)'
        }
    }
    if ($Part -in 'All', 'Ime' -and $keyboardWorks) {
        $imes = @(
            @{ name = 'pinyin'; klid = '00000804'; on = @(); keys = 'nihao'; convert = @(0x20); expect = '你好' },
            # VK_IME_ON puts Microsoft IME (Japanese) in Hiragana; Space converts, Enter commits.
            @{ name = 'japanese'; klid = '00000411'; on = @(0x16); keys = 'konnnichiha'; convert = @(0x20, 0x0D); expect = $null }
        )
        foreach ($ime in $imes) {
            Focus-Probe
            [NanaInputProbe]::Send('clear')
            Wait-For { (Editor-State).value -eq '' } 'cleared editor' | Out-Null
            [NanaInputProbe]::SwitchLayout($script:handle, $ime.klid)
            Wait-For { ([NanaInputProbe]::Layout($script:handle)).EndsWith($ime.klid.Substring(4)) } "$($ime.name) layout" | Out-Null
            Start-Sleep -Milliseconds 300
            foreach ($vk in $ime.on) { [NanaInputProbe]::Key([uint16]$vk) }
            $start = [NanaInputProbe]::Count()
            [NanaInputProbe]::Type($ime.keys)
            Start-Sleep -Milliseconds 400
            Shot "ime-$($ime.name)-preedit" 260
            $preedit = @(Inputs $start | Where-Object { $_.kind -eq 'composition' -and $_.op -eq 'update' })
            $layoutDuringPreedit = Editor-State
            foreach ($vk in $ime.convert) { [NanaInputProbe]::Key([uint16]$vk); Start-Sleep -Milliseconds 250 }
            Shot "ime-$($ime.name)-candidates" 260
            Start-Sleep -Milliseconds 300
            # A second Enter only for Japanese if the conversion is still open.
            $commits = @(Inputs $start | Where-Object { ($_.kind -eq 'composition' -and $_.op -eq 'commit') -or $_.kind -eq 'text' })
            if ($commits.Count -eq 0 -and $ime.name -eq 'japanese') { [NanaInputProbe]::Key(0x0D); Start-Sleep -Milliseconds 300 }
            $commits = @(Inputs $start | Where-Object { ($_.kind -eq 'composition' -and $_.op -eq 'commit') -or $_.kind -eq 'text' })
            $state = Wait-For { $s = Editor-State; if ($s.value -ne '') { $s } } "$($ime.name) committed value" 5
            $uia = Editor-Uia
            $uiaValue = $null; $uiaType = $null
            if ($uia) {
                $uiaType = $uia.Current.ControlType.ProgrammaticName
                $pattern = $null
                if ($uia.TryGetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern, [ref]$pattern)) { $uiaValue = $pattern.Current.Value }
            }
            Shot "ime-$($ime.name)-committed"
            Check "ime.$($ime.name).preedit_observed" ($preedit.Count -gt 0) ($preedit | Select-Object -Last 3)
            Check "ime.$($ime.name).commit_observed" ($commits.Count -gt 0) $commits
            $expectedOk = if ($ime.expect) { $state.value -eq $ime.expect } else { $state.value -match '\p{IsCJKUnifiedIdeographs}|\p{IsHiragana}' }
            Check "ime.$($ime.name).committed_value" ($expectedOk -and $state.value -notmatch '[a-z]') $state.value
            Check "ime.$($ime.name).focus_kept" ([bool]$state.focused) $state
            Check "ime.$($ime.name).uia_edit_role" ($uiaType -eq 'ControlType.Edit') $uiaType
            Check "ime.$($ime.name).uia_value" ($uiaValue -eq $state.value) $uiaValue
        }

        # Focus loss in the middle of a composition: the preedit must end, the
        # editor must not keep a half-composed value, focus must be reported.
        Focus-Probe
        [NanaInputProbe]::Send('clear')
        [NanaInputProbe]::SwitchLayout($script:handle, '00000804')
        Start-Sleep -Milliseconds 300
        $start = [NanaInputProbe]::Count()
        [NanaInputProbe]::Type('ceshi')
        Start-Sleep -Milliseconds 300
        $other = New-Object System.Windows.Forms.Form
        $other.Text = 'focus thief'; $other.StartPosition = 'Manual'; $other.Location = New-Object System.Drawing.Point(0, 0)
        $other.Show(); $other.Activate(); [System.Windows.Forms.Application]::DoEvents()
        Start-Sleep -Milliseconds 500
        $events = @(Inputs $start)
        $focusLost = @($events | Where-Object { $_.kind -eq 'focus' -and -not $_.focused })
        $compositionEnded = @($events | Where-Object { $_.kind -eq 'composition' -and ($_.op -eq 'End' -or $_.op -eq 'commit' -or ($_.op -eq 'update' -and $_.text -eq '')) })
        Check 'ime.focus_loss.focus_event' ($focusLost.Count -gt 0) $focusLost
        Check 'ime.focus_loss.composition_closed' ($compositionEnded.Count -gt 0) ($events | Where-Object { $_.kind -eq 'composition' } | Select-Object -Last 3)
        $other.Close(); $other.Dispose()
        Focus-Probe
        $afterRefocus = Editor-State
        Check 'ime.focus_loss.refocus' ([bool]$afterRefocus.focused) $afterRefocus
    }

    if ($Part -in 'All', 'Pen') {
        Focus-Probe
        [NanaInputProbe]::PenOpen()
        $origin = Client-Point 40 120
        $x = $origin.X; $y = $origin.Y
        $IR = [NanaInputProbe]::InRange; $IC = [NanaInputProbe]::InContact; $UP = [NanaInputProbe]::Update
        $P = [NanaInputProbe]::Primary; $F = [NanaInputProbe]::FirstButton

        # 1. Hover in range, no contact.
        $start = [NanaInputProbe]::Count()
        for ($i = 0; $i -lt 8; $i++) { [NanaInputProbe]::PenFrame($x + 10 * $i, $y, $IR -bor $UP -bor $P, 0, 0, 0, 0, 0) }
        Start-Sleep -Milliseconds 200
        $hover = @(Inputs $start | Where-Object { $_.kind -eq 'pointer' -and $_.pointer_type -eq 'Pen' })
        Check 'pen.hover.delivered' ($hover.Count -gt 0) ($hover | Select-Object -First 2)
        Check 'pen.hover.no_buttons_no_pressure' (@($hover | Where-Object { $_.buttons -ne 0 -or $_.pressure -ne 0 }).Count -eq 0) ($hover | Select-Object -Last 1)

        # 2. Contact with a pressure ramp, then lift.
        $start = [NanaInputProbe]::Count()
        $ramp = @(1, 64, 128, 256, 512, 768, 1024, 768, 256, 64)
        [NanaInputProbe]::PenFrame($x + 80, $y, $IR -bor $IC -bor $F -bor $P -bor [NanaInputProbe]::Down, 0, $ramp[0], 10, -5, 1)
        for ($i = 1; $i -lt $ramp.Count; $i++) {
            [NanaInputProbe]::PenFrame($x + 80 + 12 * $i, $y + 4 * $i, $IR -bor $IC -bor $F -bor $UP -bor $P, 0, $ramp[$i], 10, -5, 0)
        }
        [NanaInputProbe]::PenFrame($x + 200, $y + 40, $IR -bor $P -bor [NanaInputProbe]::Up, 0, 0, 0, 0, 2)
        Start-Sleep -Milliseconds 250
        $stroke = @(Inputs $start | Where-Object { $_.kind -eq 'pointer' -and $_.pointer_type -eq 'Pen' })
        $down = @($stroke | Where-Object { $_.phase -eq 'Down' })
        $moves = @($stroke | Where-Object { $_.phase -eq 'Move' -and $_.buttons -ne 0 })
        $upEvents = @($stroke | Where-Object { $_.phase -eq 'Up' })
        $pressures = @($down + $moves | ForEach-Object { [double]$_.pressure })
        $expected = @($ramp | ForEach-Object { $_ / 1024.0 })
        $inRange = @($pressures | Where-Object { $_ -lt 0 -or $_ -gt 1 }).Count -eq 0
        $jumps = @(); for ($i = 1; $i -lt $pressures.Count; $i++) { $jumps += [math]::Abs($pressures[$i] - $pressures[$i - 1]) }
        Check 'pen.contact.down_up' ($down.Count -eq 1 -and $upEvents.Count -eq 1) @{ down = $down; up = $upEvents }
        Check 'pen.pressure.range_0_1' ($inRange -and $pressures.Count -gt 0) $pressures
        Check 'pen.pressure.first_contact_matches_device' ($down.Count -eq 1 -and [math]::Abs([double]$down[0].pressure - $expected[0]) -lt 0.01) @{ reported = $down.pressure; device = $expected[0] }
        Check 'pen.pressure.tracks_ramp' ($pressures.Count -ge 5 -and [math]::Abs(($pressures | Measure-Object -Maximum).Maximum - 1.0) -lt 0.01) @{ reported = $pressures; device = $expected }
        Check 'pen.tilt.delivered' (@($stroke | Where-Object { $_.tilt_x -eq 10 -and $_.tilt_y -eq -5 }).Count -gt 0) ($stroke | Select-Object -First 1)
        Check 'pen.up.releases_buttons' ($upEvents.Count -eq 1 -and $upEvents[0].buttons -eq 0) $upEvents

        # 2b. A contact whose first frame reports zero pressure, as some
        # digitizers do: the reported pressure must not jump to a default.
        $start = [NanaInputProbe]::Count()
        [NanaInputProbe]::PenFrame($x + 80, $y + 20, $IR -bor $IC -bor $F -bor $P -bor [NanaInputProbe]::Down, 0, 0, 0, 0, 1)
        [NanaInputProbe]::PenFrame($x + 90, $y + 20, $IR -bor $IC -bor $F -bor $UP -bor $P, 0, 40, 0, 0, 0)
        [NanaInputProbe]::PenFrame($x + 100, $y + 20, $IR -bor $IC -bor $F -bor $UP -bor $P, 0, 80, 0, 0, 0)
        [NanaInputProbe]::PenFrame($x + 100, $y + 20, $IR -bor $P -bor [NanaInputProbe]::Up, 0, 0, 0, 0, 2)
        Start-Sleep -Milliseconds 250
        $zero = @(Inputs $start | Where-Object { $_.kind -eq 'pointer' -and $_.pointer_type -eq 'Pen' -and $_.buttons -ne 0 })
        $zeroPressures = @($zero | ForEach-Object { [double]$_.pressure })
        Check 'pen.pressure.zero_contact_no_jump' ($zeroPressures.Count -ge 2 -and $zeroPressures[0] -le $zeroPressures[1] -and $zeroPressures[0] -lt 0.05) @{ reported = $zeroPressures; device = @(0, (40 / 1024.0), (80 / 1024.0)) }

        # 3. Barrel button: pressed while hovering, then a barrel-held contact.
        $start = [NanaInputProbe]::Count()
        [NanaInputProbe]::PenFrame($x + 40, $y + 60, $IR -bor $UP -bor $P, [NanaInputProbe]::Barrel, 0, 0, 0, 0)
        [NanaInputProbe]::PenFrame($x + 50, $y + 60, $IR -bor $UP -bor $P, [NanaInputProbe]::Barrel, 0, 0, 0, 0)
        [NanaInputProbe]::PenFrame($x + 50, $y + 60, $IR -bor $IC -bor [NanaInputProbe]::SecondButton -bor $P -bor [NanaInputProbe]::Down, [NanaInputProbe]::Barrel, 400, 0, 0, 3)
        [NanaInputProbe]::PenFrame($x + 60, $y + 64, $IR -bor $IC -bor [NanaInputProbe]::SecondButton -bor $UP -bor $P, [NanaInputProbe]::Barrel, 420, 0, 0, 0)
        [NanaInputProbe]::PenFrame($x + 60, $y + 64, $IR -bor $P -bor [NanaInputProbe]::Up, 0, 0, 0, 0, 4)
        Start-Sleep -Milliseconds 250
        $barrel = @(Inputs $start | Where-Object { $_.kind -eq 'pointer' -and $_.pointer_type -eq 'Pen' })
        $barrelDown = @($barrel | Where-Object { $_.phase -eq 'Down' })
        Check 'pen.barrel.contact_maps_to_secondary' ($barrelDown.Count -eq 1 -and $barrelDown[0].button -eq 2) $barrelDown
        Check 'pen.barrel.release_clears_buttons' (@($barrel | Where-Object { $_.phase -eq 'Up' -and $_.buttons -eq 0 }).Count -eq 1) ($barrel | Where-Object { $_.phase -eq 'Up' })

        # 4. Cancellation mid-stroke (POINTER_FLAG_CANCELED on the lift).
        $start = [NanaInputProbe]::Count()
        [NanaInputProbe]::PenFrame($x + 20, $y + 100, $IR -bor $IC -bor $F -bor $P -bor [NanaInputProbe]::Down, 0, 300, 0, 0, 1)
        [NanaInputProbe]::PenFrame($x + 40, $y + 100, $IR -bor $IC -bor $F -bor $UP -bor $P, 0, 320, 0, 0, 0)
        [NanaInputProbe]::PenFrame($x + 40, $y + 100, $P -bor [NanaInputProbe]::Up -bor [NanaInputProbe]::Canceled, 0, 0, 0, 0, 2)
        Start-Sleep -Milliseconds 250
        $cancel = @(Inputs $start | Where-Object { $_.kind -eq 'pointer' -and $_.pointer_type -eq 'Pen' })
        Check 'pen.cancel.reported_as_cancel' (@($cancel | Where-Object { $_.phase -eq 'Cancel' }).Count -eq 1) ($cancel | Select-Object -Last 2)
        Check 'pen.cancel.no_stuck_buttons' ($cancel.Count -gt 0 -and $cancel[-1].buttons -eq 0) ($cancel | Select-Object -Last 1)

        # 5. Focus loss during a stroke: the stroke must end without stuck buttons.
        $start = [NanaInputProbe]::Count()
        [NanaInputProbe]::PenFrame($x + 20, $y + 140, $IR -bor $IC -bor $F -bor $P -bor [NanaInputProbe]::Down, 0, 300, 0, 0, 1)
        [NanaInputProbe]::PenFrame($x + 40, $y + 140, $IR -bor $IC -bor $F -bor $UP -bor $P, 0, 320, 0, 0, 0)
        $thief = New-Object System.Windows.Forms.Form
        $thief.Text = 'focus thief'; $thief.StartPosition = 'Manual'; $thief.Location = New-Object System.Drawing.Point(0, 0)
        $thief.Show(); $thief.Activate(); [System.Windows.Forms.Application]::DoEvents()
        Start-Sleep -Milliseconds 300
        [NanaInputProbe]::PenFrame($x + 60, $y + 140, $IR -bor $P -bor [NanaInputProbe]::Up, 0, 0, 0, 0, 2)
        Start-Sleep -Milliseconds 250
        $thief.Close(); $thief.Dispose()
        $focusStroke = @(Inputs $start)
        Check 'pen.focus_loss.focus_event' (@($focusStroke | Where-Object { $_.kind -eq 'focus' -and -not $_.focused }).Count -gt 0) ($focusStroke | Where-Object { $_.kind -eq 'focus' })
        $lastPen = @($focusStroke | Where-Object { $_.kind -eq 'pointer' -and $_.pointer_type -eq 'Pen' }) | Select-Object -Last 1
        Check 'pen.focus_loss.stroke_terminated' ($lastPen -and ($lastPen.phase -in 'Up', 'Cancel') -and $lastPen.buttons -eq 0) $lastPen

        # 6. Leave range, then the mouse: no pen state leaks into mouse events.
        Focus-Probe
        $start = [NanaInputProbe]::Count()
        [NanaInputProbe]::PenFrame($x + 60, $y + 160, $IR -bor $UP -bor $P, 0, 0, 0, 0, 0)
        [NanaInputProbe]::PenFrame($x + 60, $y + 160, $UP -bor $P, 0, 0, 0, 0, 0)
        Start-Sleep -Milliseconds 200
        $leave = @(Inputs $start | Where-Object { $_.kind -eq 'pointer_leave' })
        Check 'pen.leave_range.pointer_leave' ($leave.Count -gt 0) $leave
        $start = [NanaInputProbe]::Count()
        for ($i = 0; $i -lt 5; $i++) { [NanaInputProbe]::MouseMove($x + 100 + 7 * $i, $y + 30) }
        Start-Sleep -Milliseconds 200
        $mouse = @(Inputs $start | Where-Object { $_.kind -eq 'pointer' })
        Check 'pen.crosstalk.mouse_is_mouse' ($mouse.Count -gt 0 -and @($mouse | Where-Object { $_.pointer_type -ne 'Mouse' }).Count -eq 0) ($mouse | Select-Object -First 1)
        Check 'pen.crosstalk.mouse_has_no_pen_buttons' (@($mouse | Where-Object { $_.buttons -ne 0 -or $_.pressure -ne 0 }).Count -eq 0) ($mouse | Select-Object -Last 1)
        [NanaInputProbe]::PenClose()
        Shot 'pen-final'
    }
}
finally {
    $summary = [ordered]@{ passed = ($failures.Count -eq 0); failures = $failures; checks = $results }
    $summary | ConvertTo-Json -Depth 8 | Set-Content -Encoding UTF8 (Join-Path $output 'summary.json')
    try { [NanaInputProbe]::PenClose() } catch {}
    try { if ($originalHkl) { [NanaInputProbe]::RestoreHkl($script:handle, $originalHkl) } } catch {}
    try { [NanaInputProbe]::Send('quit') } catch {}
    if (-not $process.WaitForExit(10000)) { $process.Kill() }
}
Write-Host "summary: $(Join-Path $output 'summary.json')"
if ($failures.Count -gt 0) { exit 1 }
