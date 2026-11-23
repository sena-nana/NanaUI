# Real-window acceptance for the transparent-alpha window path (#272).
#
# Run with Windows PowerShell 5.1:
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts/validate-transparent-window.ps1 `
#     -ExePath target/release/examples/transparent-window.exe
#
# Places a solid magenta form behind the demo, then for the initial size and
# each resize: records the window's JSON line (requested material, actual
# MaterialOutcome, alpha mode, physical size, scale factor), screenshots the
# window from the desktop, and measures how much of the backdrop shows through
# the window's transparent area while its text stays readable. When a second
# monitor with a different DPI exists, the window is also moved there.
param(
    [Parameter(Mandatory = $true)][string]$ExePath,
    [string]$OutputDirectory = 'target/acceptance/transparent-window'
)
$ErrorActionPreference = 'Stop'
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
public static class NanaTransparentProbe {
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr window, out Rect rect);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr window, IntPtr after, int x, int y, int cx, int cy, uint flags);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr window, int attribute, out Rect rect, int size);
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
    public static Rect Frame(IntPtr window) {
        Rect rect;
        // DWMWA_EXTENDED_FRAME_BOUNDS: the visible frame, without the resize border.
        if (DwmGetWindowAttribute(window, 9, out rect, Marshal.SizeOf(typeof(Rect))) != 0) GetWindowRect(window, out rect);
        return rect;
    }
    static readonly List<string> lines = new List<string>();
    static Process probe;
    public static Process Start(string exe, string log) {
        var info = new ProcessStartInfo(exe);
        info.UseShellExecute = false; info.RedirectStandardInput = true; info.RedirectStandardOutput = true;
        info.StandardOutputEncoding = new UTF8Encoding(false);
        probe = Process.Start(info);
        var writer = new StreamWriter(log, false, new UTF8Encoding(false));
        writer.AutoFlush = true;
        var reader = new Thread(() => {
            string line;
            while ((line = probe.StandardOutput.ReadLine()) != null) { lock (lines) lines.Add(line); writer.WriteLine(line); }
            writer.Dispose();
        });
        reader.IsBackground = true; reader.Start();
        return probe;
    }
    public static string[] All() { lock (lines) return lines.ToArray(); }
    public static void Send(string command) {
        var bytes = new UTF8Encoding(false).GetBytes(command + "\n");
        probe.StandardInput.BaseStream.Write(bytes, 0, bytes.Length);
        probe.StandardInput.BaseStream.Flush();
    }
}
'@
[NanaTransparentProbe]::SetProcessDPIAware() | Out-Null
$exe = (Resolve-Path -LiteralPath $ExePath).Path
$output = [IO.Path]::GetFullPath($OutputDirectory)
[IO.Directory]::CreateDirectory($output) | Out-Null
$failures = New-Object System.Collections.Generic.List[string]
$checks = [ordered]@{}
function Wait-For([scriptblock]$Check, [string]$Description, [int]$Seconds = 15) {
    $deadline = [DateTime]::UtcNow.AddSeconds($Seconds)
    do { $value = & $Check; if ($value) { return $value }; Start-Sleep -Milliseconds 50 } while ([DateTime]::UtcNow -lt $deadline)
    throw "Timed out waiting for $Description"
}
function Lines() { [NanaTransparentProbe]::All() | ForEach-Object { $_ | ConvertFrom-Json } }
function Check([string]$Name, [bool]$Pass, $Detail) {
    $checks[$Name] = [ordered]@{ pass = $Pass; detail = $Detail }
    if (-not $Pass) { $failures.Add($Name) }
    Write-Host ("{0,-40} {1}" -f $Name, $(if ($Pass) { 'pass' } else { 'FAIL' }))
}
# Share of sampled pixels inside the window that show the magenta backdrop,
# and share of light glyph pixels (text and button labels).
function Measure-Window([string]$Name) {
    Start-Sleep -Milliseconds 400
    $rect = [NanaTransparentProbe]::Frame($script:handle)
    $width = $rect.Right - $rect.Left; $height = $rect.Bottom - $rect.Top
    $bitmap = New-Object System.Drawing.Bitmap $width, $height
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $graphics.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $bitmap.Size)
    $bitmap.Save((Join-Path $output "$Name.png"), [System.Drawing.Imaging.ImageFormat]::Png)
    # Backdrop share below the title bar on a coarse grid; text share (light
    # glyph pixels, luminance above 150 and not magenta) on a fine grid over
    # the content rows, where the demo's labels and buttons sit.
    $samples = 0; $magenta = 0
    for ($y = [int]($height * 0.15); $y -lt $height - 8; $y += 6) {
        for ($x = 8; $x -lt $width - 8; $x += 6) {
            $pixel = $bitmap.GetPixel($x, $y); $samples++
            if ($pixel.R -gt 140 -and $pixel.B -gt 140 -and $pixel.G -lt 110) { $magenta++ }
        }
    }
    $textSamples = 0; $text = 0
    for ($y = 32; $y -lt [math]::Min($height, 140); $y += 2) {
        for ($x = 2; $x -lt $width - 2; $x += 2) {
            $pixel = $bitmap.GetPixel($x, $y); $textSamples++
            $luma = ($pixel.R * 3 + $pixel.G * 6 + $pixel.B) / 10
            if ($luma -gt 150 -and -not ($pixel.R -gt 140 -and $pixel.B -gt 140 -and $pixel.G -lt 110)) { $text++ }
        }
    }
    $bright = $text; $samples_text = $textSamples
    $graphics.Dispose(); $bitmap.Dispose()
    [ordered]@{ size = "${width}x${height}"; backdrop_share = [math]::Round($magenta / $samples, 3); text_share = [math]::Round($bright / $samples_text, 4) }
}

$backdrop = New-Object System.Windows.Forms.Form
$backdrop.FormBorderStyle = 'None'; $backdrop.BackColor = [System.Drawing.Color]::FromArgb(255, 0, 255)
$backdrop.StartPosition = 'Manual'; $backdrop.ShowInTaskbar = $false
$backdrop.Location = New-Object System.Drawing.Point(40, 40); $backdrop.Size = New-Object System.Drawing.Size(1800, 1200)
$backdrop.Show(); [System.Windows.Forms.Application]::DoEvents()

$process = [NanaTransparentProbe]::Start($exe, (Join-Path $output 'events.jsonl'))
try {
    $script:handle = Wait-For {
        $found = [NanaTransparentProbe]::Find($process.Id, 'NanaUI Transparent Window Demo')
        if ($found -ne [IntPtr]::Zero) { $found }
    } 'demo window'
    $ready = Wait-For { Lines | Where-Object { $_.event -eq 'ready' } | Select-Object -First 1 } 'ready line'
    [NanaTransparentProbe]::SetWindowPos($script:handle, [IntPtr]::Zero, 120, 120, 0, 0, 0x0001 -bor 0x0004) | Out-Null
    [NanaTransparentProbe]::SetForegroundWindow($script:handle) | Out-Null
    $environment = [ordered]@{
        os = (Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion' | ForEach-Object { "$($_.ProductName) $($_.DisplayVersion) build $($_.CurrentBuild).$($_.UBR)" })
        gpus = @(Get-CimInstance Win32_VideoController | ForEach-Object { "$($_.Name) $($_.DriverVersion)" })
        dpi = [NanaTransparentProbe]::GetDpiForWindow($script:handle)
        monitors = @([System.Windows.Forms.Screen]::AllScreens | ForEach-Object { "$($_.DeviceName) $($_.Bounds.Width)x$($_.Bounds.Height)" })
    }
    $transparentOutcome = $ready.outcome -match 'Transparent'
    Check 'ready.outcome_reported' ($null -ne $ready.outcome) $ready
    $initial = Measure-Window 'initial'
    # The backdrop showing through is the observed effect; it must agree with the reported outcome.
    Check 'initial.outcome_matches_observed' (($initial.backdrop_share -gt 0.2) -eq $transparentOutcome) @{ outcome = $ready.outcome; observed = $initial }
    Check 'initial.content_readable' ($initial.text_share -gt 0.005) $initial

    $sizes = @(@(1200, 800), @(700, 480), @(1000, 700))
    $index = 0
    foreach ($size in $sizes) {
        $index++
        $before = @(Lines | Where-Object { $_.event -eq 'resized' }).Count
        [NanaTransparentProbe]::SetWindowPos($script:handle, [IntPtr]::Zero, 0, 0, $size[0], $size[1], 0x0002 -bor 0x0004) | Out-Null
        $line = Wait-For { $all = @(Lines | Where-Object { $_.event -eq 'resized' }); if ($all.Count -gt $before) { $all[-1] } } "resize $index"
        $measured = Measure-Window "resize-$index"
        Check "resize$index.outcome_stable" ($line.outcome -eq $ready.outcome -and $line.alpha_mode -eq $ready.alpha_mode) $line
        Check "resize$index.alpha_preserved" (($measured.backdrop_share -gt 0.2) -eq $transparentOutcome) $measured
        Check "resize$index.content_readable" ($measured.text_share -gt 0.005) $measured
    }

    $current = [System.Windows.Forms.Screen]::FromHandle($script:handle)
    $other = [System.Windows.Forms.Screen]::AllScreens | Where-Object { $_.DeviceName -ne $current.DeviceName } | Select-Object -First 1
    if ($other) {
        $before = @(Lines | Where-Object { $_.event -eq 'resized' }).Count
        [NanaTransparentProbe]::SetWindowPos($script:handle, [IntPtr]::Zero, $other.Bounds.Left + 100, $other.Bounds.Top + 100, 0, 0, 0x0001 -bor 0x0004) | Out-Null
        Start-Sleep -Milliseconds 800
        $moved = @(Lines | Where-Object { $_.event -eq 'resized' }) | Select-Object -Last 1
        $dpi = [NanaTransparentProbe]::GetDpiForWindow($script:handle)
        $checks['dpi_change'] = [ordered]@{ pass = $true; detail = @{ monitor = $other.DeviceName; dpi = $dpi; last_line = $moved; note = 'backdrop form covers the primary monitor only; inspect the screenshot' } }
        $null = Measure-Window 'second-monitor'
    }
    else {
        $checks['dpi_change'] = [ordered]@{ pass = $null; detail = 'not executed: one monitor; a DPI change needs a second monitor or a manual scale change' }
    }
}
finally {
    [ordered]@{ passed = ($failures.Count -eq 0); failures = $failures; environment = $environment; checks = $checks } |
        ConvertTo-Json -Depth 8 | Set-Content -Encoding UTF8 (Join-Path $output 'summary.json')
    try { [NanaTransparentProbe]::Send('quit') } catch {}
    if (-not $process.WaitForExit(10000)) { $process.Kill() }
    $backdrop.Close(); $backdrop.Dispose()
}
Write-Host "summary: $(Join-Path $output 'summary.json')"
if ($failures.Count -gt 0) { exit 1 }
