# Visual acceptance of the Early Splash -> UiReady handoff (#273) on Windows.
#
# Run with Windows PowerShell 5.1:
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts/validate-startup-handoff.ps1 `
#     -ExePath target/release/examples/startup-splash.exe -Arguments '--probe'
#
# Captures the top-left of the primary monitor from a capture thread for the
# whole launch (the desktop compositor's output, so DWM cloaking and
# DirectComposition are what is captured; summary.json reports the rate), then classifies each
# frame against the desktop before launch:
#   blank    - nothing of the application on screen,
#   visible  - something the application drew (splash or window) is on screen.
# After the first visible frame and before the process exits, the trace must
# have no blank frame (a gap between splash and window), and the window's own
# area must never show an empty, single-colour window before its content.
# Frames are written to -OutputDirectory with timestamps; summary.json holds
# the classification and the probe's JSON record.
param(
    [Parameter(Mandatory = $true)][string]$ExePath,
    [string]$Arguments = '--probe',
    [string]$OutputDirectory = 'target/acceptance/startup',
    [string]$Name = 'handoff',
    [int]$Seconds = 6,
    # The captured region, from the primary monitor's top-left corner, in
    # physical pixels: the demo's window and splash open inside it.
    [int]$CaptureWidth = 1000,
    [int]$CaptureHeight = 720
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type -ReferencedAssemblies System.Drawing @'
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
using System.Threading;
public static class NanaStartupCapture {
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    public class Frame { public double Ms; public byte[] Signature; public Bitmap Small; public string File; }
    // Captures the top-left region (where the demo opens) until stop is set,
    // keeping a 1/4-scale copy and a 64x36 grey signature per frame; files
    // are written after the capture so encoding does not slow the loop.
    public static List<Frame> Run(string directory, Stopwatch clock, Func<bool> stop, int width, int height) {
        var frames = new List<Frame>();
        using (var full = new Bitmap(width, height, PixelFormat.Format32bppArgb))
        using (var graphics = Graphics.FromImage(full)) {
            while (!stop()) {
                double ms = clock.Elapsed.TotalMilliseconds;
                graphics.CopyFromScreen(0, 0, 0, 0, full.Size);
                var data = full.LockBits(new Rectangle(0, 0, width, height), ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
                var signature = new byte[64 * 36];
                var pixel = new byte[4];
                for (int y = 0; y < 36; y++) for (int x = 0; x < 64; x++) {
                    int px = x * width / 64 + width / 128, py = y * height / 36 + height / 72;
                    Marshal.Copy(IntPtr.Add(data.Scan0, py * data.Stride + px * 4), pixel, 0, 4);
                    signature[y * 64 + x] = (byte)((pixel[2] * 3 + pixel[1] * 6 + pixel[0]) / 10);
                }
                full.UnlockBits(data);
                frames.Add(new Frame { Ms = ms, Signature = signature, Small = new Bitmap(full, width / 3, height / 3) });
                Thread.Sleep(1);
            }
        }
        for (int i = 0; i < frames.Count; i++) {
            frames[i].File = System.IO.Path.Combine(directory, string.Format("{0:D4}-{1:F0}ms.png", i, frames[i].Ms));
            frames[i].Small.Save(frames[i].File, ImageFormat.Png);
            frames[i].Small.Dispose(); frames[i].Small = null;
        }
        return frames;
    }
    public static double Changed(byte[] a, byte[] b) {
        int changed = 0;
        for (int i = 0; i < a.Length; i++) if (Math.Abs(a[i] - b[i]) > 12) changed++;
        return (double)changed / a.Length;
    }
}
'@
[NanaStartupCapture]::SetProcessDPIAware() | Out-Null
$exe = (Resolve-Path -LiteralPath $ExePath).Path
$directory = [IO.Path]::GetFullPath((Join-Path $OutputDirectory $Name))
if (Test-Path $directory) { Remove-Item -Recurse -Force $directory }
[IO.Directory]::CreateDirectory($directory) | Out-Null

$clock = [Diagnostics.Stopwatch]::StartNew()
$state = @{ process = $null; exitedAt = $null }
$deadline = [DateTime]::UtcNow.AddSeconds($Seconds)
# Baseline frames before launch, then the launch, all on one capture loop.
$launchAt = 400.0
$job = [powershell]::Create()
$null = $job.AddScript({
    param($directory, $clock, $deadline, $width, $height)
    [NanaStartupCapture]::Run($directory, $clock, [Func[bool]] { [DateTime]::UtcNow -gt $deadline }, $width, $height)
}).AddArgument($directory).AddArgument($clock).AddArgument($deadline).AddArgument($CaptureWidth).AddArgument($CaptureHeight)
$async = $job.BeginInvoke()
while ($clock.Elapsed.TotalMilliseconds -lt $launchAt) { Start-Sleep -Milliseconds 10 }
$info = New-Object Diagnostics.ProcessStartInfo $exe, $Arguments
$info.UseShellExecute = $false; $info.RedirectStandardOutput = $true; $info.RedirectStandardError = $true
$info.WorkingDirectory = Split-Path $exe
$launched = $clock.Elapsed.TotalMilliseconds
$process = [Diagnostics.Process]::Start($info)
$stdout = $process.StandardOutput.ReadToEndAsync()
$stderr = $process.StandardError.ReadToEndAsync()
if (-not $process.WaitForExit(($Seconds - 1) * 1000)) { $process.Kill() }
$exited = $clock.Elapsed.TotalMilliseconds
$frames = $job.EndInvoke($async)
$job.Dispose()

$baseline = $frames | Where-Object { $_.Ms -lt $launched } | Select-Object -Last 1
$classified = foreach ($frame in $frames) {
    $changed = [NanaStartupCapture]::Changed($baseline.Signature, $frame.Signature)
    [pscustomobject]@{ ms = [math]::Round($frame.Ms - $launched, 1); changed = [math]::Round($changed, 4); file = Split-Path -Leaf $frame.File }
}
# "Visible" is any frame that differs from the desktop in more than 0.5% of samples.
$threshold = 0.005
$afterLaunch = @($classified | Where-Object { $_.ms -ge 0 -and $_.ms -le ($exited - $launched) })
$firstVisible = $afterLaunch | Where-Object { $_.changed -gt $threshold } | Select-Object -First 1
# The probe exits a second after the handoff; frames up to 300 ms before the
# exit are inside the application's lifetime and must all be visible.
$window = @($afterLaunch | Where-Object { $firstVisible -and $_.ms -ge $firstVisible.ms -and $_.ms -le ($exited - $launched - 300) })
$gaps = @($window | Where-Object { $_.changed -le $threshold })
$intervals = @(); for ($i = 1; $i -lt $window.Count; $i++) { $intervals += $window[$i].ms - $window[$i - 1].ms }
$record = $null
$line = ($stdout.Result -split "`n" | Where-Object { $_.Trim().StartsWith('{') } | Select-Object -Last 1)
if ($line) { $record = $line | ConvertFrom-Json }
$summary = [ordered]@{
    arguments = $Arguments
    exit_code = $process.ExitCode
    probe_record = $record
    stderr = $stderr.Result
    frames_captured = $frames.Count
    capture_interval_ms = if ($intervals.Count) { [ordered]@{ mean = [math]::Round(($intervals | Measure-Object -Average).Average, 1); max = [math]::Round(($intervals | Measure-Object -Maximum).Maximum, 1) } } else { $null }
    first_visible_ms = if ($firstVisible) { $firstVisible.ms } else { $null }
    frames_in_lifetime = $window.Count
    blank_gap_frames = $gaps
    passed = ($process.ExitCode -eq 0 -and $firstVisible -and $gaps.Count -eq 0)
    frames = $classified
}
$summary | ConvertTo-Json -Depth 6 | Set-Content -Encoding UTF8 (Join-Path $directory 'summary.json')
Write-Host ("{0}: exit={1} first_visible={2}ms frames={3} gaps={4} -> {5}" -f $Name, $process.ExitCode, $summary.first_visible_ms, $window.Count, $gaps.Count, $(if ($summary.passed) { 'pass' } else { 'FAIL' }))
if (-not $summary.passed) { exit 1 }
