param(
    [ValidateRange(1, 3600)][int]$Seconds = 60,
    [ValidateSet(1, 4, 16)][int[]]$TextureNodes = @(1, 4, 16),
    [switch]$IndependentTextures,
    [switch]$AllocationCounts,
    [string]$OutputDirectory = "target/performance/high-refresh-gpu",
    [string]$Binary = "target/release/nana-gpu-scene-benchmark.exe"
)

$ErrorActionPreference = "Stop"
$repo = Split-Path -Parent $PSScriptRoot
$binary = if ([IO.Path]::IsPathRooted($Binary)) {
    [IO.Path]::GetFullPath($Binary)
} else {
    [IO.Path]::GetFullPath((Join-Path $repo $Binary))
}
if (-not (Test-Path -LiteralPath $binary)) {
    throw "GPU benchmark binary not found at $binary. Build nana-gpu-scene-benchmark in release mode with gpu,bundled-fonts first."
}
if ($IndependentTextures -and $OutputDirectory -eq "target/performance/high-refresh-gpu") {
    $OutputDirectory = "target/performance/high-refresh-gpu-independent"
}
if ($AllocationCounts -and -not $PSBoundParameters.ContainsKey("OutputDirectory")) {
    $OutputDirectory += "-allocations"
}
$fixture = if ($IndependentTextures) { "high-refresh-independent-gpu" } else { "high-refresh-gpu" }
$output = [IO.Path]::GetFullPath((Join-Path $repo $OutputDirectory))
[IO.Directory]::CreateDirectory($output) | Out-Null
$machine = @{
    cpu = @(Get-CimInstance Win32_Processor | Select-Object -ExpandProperty Name)
    video = @(Get-CimInstance Win32_VideoController | Select-Object Name, CurrentHorizontalResolution, CurrentVerticalResolution, CurrentRefreshRate)
    os = [Environment]::OSVersion.VersionString
    source_commit = (git -C $repo rev-parse HEAD)
    source_dirty = [bool](git -C $repo status --porcelain)
    note = "Offscreen GPU completion serialized. Monitor modes are enumeration, not measured Surface intervals."
}

# Run cases serially; no concurrent build/test is started by this script.
foreach ($count in $TextureNodes) {
    $report = Join-Path $output "gpu-$count.json"
    $scenario = Join-Path $repo "perf/fixtures/$fixture-$count.json"
    $arguments = '--scenario "{0}" --gpu-timestamps --sample-seconds {1} --output "{2}"' -f $scenario, $Seconds, $report
    if ($AllocationCounts) { $arguments += " --allocation-counts" }
    $started = [DateTime]::UtcNow.ToString("o")
    $process = Start-Process -FilePath $binary -ArgumentList $arguments -WorkingDirectory $repo -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $output "gpu-$count.stdout.txt") -RedirectStandardError (Join-Path $output "gpu-$count.stderr.txt")
    $peakWorkingSet = 0L
    $sampledPrivate = 0L
    $observations = 0
    while (-not $process.HasExited) {
        $process.Refresh()
        if (-not $process.HasExited) {
            $peakWorkingSet = [Math]::Max($peakWorkingSet, $process.PeakWorkingSet64)
            $sampledPrivate = [Math]::Max($sampledPrivate, $process.PrivateMemorySize64)
            $observations++
        }
        Start-Sleep -Milliseconds 100
    }
    $process.WaitForExit()
    if ($process.ExitCode -ne 0) {
        throw "GPU case $count exited $($process.ExitCode); see report and stderr in $output"
    }
    $memory = @{
        started_utc = $started
        machine = $machine
        process_peak_working_set_bytes = $peakWorkingSet
        maximum_sampled_private_bytes = $sampledPrivate
        observation_count = $observations
        sample_period_ms = 100
        note = "Whole benchmark process including warmup and diagnostic sample storage; excludes separate GPU memory. Private bytes are a sampled maximum, not an allocation count."
    }
    $memory | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath (Join-Path $output "gpu-$count.memory.json") -Encoding utf8
    $result = Get-Content -LiteralPath $report -Raw | ConvertFrom-Json
    Write-Output ([pscustomobject]@{
        texture_nodes = $count
        frames = $result.frames
        cpu_prepare_p95_ms = $result.sampling.framework_cpu_prepare_ms.p95
        ui_gpu_p95_ms = $result.gpu_timestamps.ui_composition_ms.p95
        producer_gpu_p95_ms = $result.gpu_timestamps.producer_ms.p95
        maximum_framework_allocation_calls = $result.framework_thread_allocations.maximum_per_frame.calls
        maximum_framework_requested_bytes = $result.framework_thread_allocations.maximum_per_frame.requested_bytes
    })
}
