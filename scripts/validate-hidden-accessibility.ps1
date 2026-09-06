# Run with Windows PowerShell 5.1 -Mta. Only the spawned probe is operated.
param(
    [Parameter(Mandatory = $true)][string]$ExePath,
    [string]$OutputDirectory = 'target/performance/native-hidden-a11y',
    [ValidateSet('Full', 'Semantics')][string]$Mode = 'Full',
    [switch]$Retry,
    [switch]$InitialFailure,
    [switch]$Auxiliary
)
$ErrorActionPreference = 'Stop'
if ($InitialFailure -and -not $Retry) { throw 'InitialFailure requires Retry' }
if ($Auxiliary -and -not $InitialFailure) { throw 'Auxiliary requires InitialFailure' }
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class NanaProbeWindow {
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr window, out Rect rect);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr window, int command);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    private delegate bool EnumCallback(IntPtr window, IntPtr data);
    [DllImport("user32.dll")] private static extern bool EnumWindows(EnumCallback callback, IntPtr data);
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr window, out uint process);
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
}
'@
$exe = (Resolve-Path -LiteralPath $ExePath).Path
$output = [IO.Path]::GetFullPath($OutputDirectory)
[IO.Directory]::CreateDirectory($output) | Out-Null
function Wait-For([scriptblock]$Check, [string]$Description) {
    $deadline = [DateTime]::UtcNow.AddSeconds(15)
    $lastError = ''
    do {
        try { $result = & $Check; if ($result) { return $result } }
        catch { $lastError = $_.Exception.Message }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Timed out waiting for $Description. $lastError"
}
function Find-Named([string]$Name, [IntPtr]$Handle = $probeHandle) {
    $probeWindow = [System.Windows.Automation.AutomationElement]::FromHandle($Handle)
    $condition = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::NameProperty, $Name)
    $probeWindow.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $condition)
}
function Send-Command([string]$Command) {
    # Windows PowerShell's Process.StandardInput writer can prefix its first
    # command with a BOM. The probe protocol is UTF-8 without a preamble.
    $bytes = [Text.Encoding]::UTF8.GetBytes($Command + "`n")
    $stream = $probeProcess.StandardInput.BaseStream
    $stream.Write($bytes, 0, $bytes.Length)
    $stream.Flush()
}
function Snapshot-Nodes([IntPtr]$Handle = $probeHandle) {
    $probeWindow = [System.Windows.Automation.AutomationElement]::FromHandle($Handle)
    $elements = $probeWindow.FindAll([System.Windows.Automation.TreeScope]::Descendants,
        [System.Windows.Automation.Condition]::TrueCondition)
    $items = @()
    for ($index = 0; $index -lt $elements.Count; $index++) {
        $current = $elements.Item($index).Current
        $items += [pscustomobject]@{
            Name = $current.Name; Type = $current.ControlType.ProgrammaticName
            Focused = $current.HasKeyboardFocus; Offscreen = $current.IsOffscreen
            Bounds = $current.BoundingRectangle.ToString()
        }
    }
    $items
}
function Capture-Probe([string]$Name) {
    if ($Mode -ne 'Full') { return }
    [NanaProbeWindow]::SetForegroundWindow($probeHandle) | Out-Null
    Start-Sleep -Milliseconds 100
    $rect = New-Object NanaProbeWindow+Rect
    if (-not [NanaProbeWindow]::GetWindowRect($probeHandle, [ref]$rect)) { throw 'Cannot read probe bounds' }
    $bitmap = New-Object Drawing.Bitmap(($rect.Right - $rect.Left), ($rect.Bottom - $rect.Top))
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    try {
        $graphics.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $bitmap.Size)
        $bitmap.Save((Join-Path $output $Name), [Drawing.Imaging.ImageFormat]::Png)
    } finally { $graphics.Dispose(); $bitmap.Dispose() }
}
$probeProcess = New-Object Diagnostics.Process
$probeProcess.StartInfo.FileName = $exe
if ($Retry) { $probeProcess.StartInfo.Arguments = '--retry' }
if ($InitialFailure) { $probeProcess.StartInfo.Arguments += ' --initial-failure' }
if ($Auxiliary) { $probeProcess.StartInfo.Arguments += ' --auxiliary' }
$probeProcess.StartInfo.UseShellExecute = $false
$probeProcess.StartInfo.CreateNoWindow = $true
$probeProcess.StartInfo.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
$probeProcess.StartInfo.RedirectStandardInput = $true
$probeProcess.StartInfo.RedirectStandardOutput = $true
$probeProcess.StartInfo.RedirectStandardError = $true
$started = $false
$editorName = if ($Retry) { 'Updated editor' } else { 'Visible editor' }
$failure = $null
$report = [ordered]@{ passed = $false; platform = 'Windows UI Automation'; mode = $Mode; retry = [bool]$Retry; executable = $exe }
$report.initial_failure = [bool]$InitialFailure
$report.auxiliary = [bool]$Auxiliary
$probeHandle = [IntPtr]::Zero
$targetTitle = if ($Auxiliary) { 'NanaUI Auxiliary Probe' } else { 'NanaUI Accessibility Probe' }
try {
    if (-not $probeProcess.Start()) { throw 'Probe did not start' }
    $started = $true
    $stdout = $probeProcess.StandardOutput.ReadToEndAsync()
    $stderr = $probeProcess.StandardError.ReadToEndAsync()
    $probeHandle = Wait-For {
        $probeProcess.Refresh()
        if ($probeProcess.HasExited) { throw "Probe exited with $($probeProcess.ExitCode)" }
        $handle = [NanaProbeWindow]::Find($probeProcess.Id, $targetTitle)
        if ($handle -ne [IntPtr]::Zero) { $handle }
    } 'probe window'
    $probeWindow = [System.Windows.Automation.AutomationElement]::FromHandle($probeHandle)
    $report.window = [ordered]@{
        Name = $probeWindow.Current.Name
        Class = $probeWindow.Current.ClassName
        ProcessId = $probeWindow.Current.ProcessId
        Handle = $probeHandle.ToInt64()
    }
    Capture-Probe 'initial.png'
    if ($InitialFailure) {
        $report.before_first_present_root = $probeWindow.Current.ControlType.ProgrammaticName
        if ($probeWindow.Current.ControlType -ne [System.Windows.Automation.ControlType]::Window) { throw 'Missing native window root before first presentation' }
        $report.before_first_present = @(Snapshot-Nodes)
        if ($report.before_first_present.Count -ne 0) { throw 'Unpresented document leaked into native accessibility' }
        if ($Auxiliary) {
            $primaryHandle = [NanaProbeWindow]::Find($probeProcess.Id, 'NanaUI Accessibility Probe')
            $primaryEditor = Wait-For { Find-Named 'Primary editor' $primaryHandle } 'primary editor while auxiliary fails'
            $primaryValue = [System.Windows.Automation.ValuePattern]$primaryEditor.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern)
            $primaryValue.SetValue('Primary during failure')
            Wait-For { $primaryValue.Current.Value -eq 'Primary during failure' } 'primary update while auxiliary fails' | Out-Null
            $report.primary_during_failure = @(Snapshot-Nodes $primaryHandle)
            if (@(Snapshot-Nodes).Count -ne 0) { throw 'Primary update published the failed auxiliary document' }
        }
        Send-Command 'recover-initial'
    }
    $editor = Wait-For { Find-Named $editorName } 'visible editor in native tree'
    if ($Retry) {
        Wait-For { Find-Named 'Recovered item' } 'node added during failed encoding' | Out-Null
        Wait-For { Find-Named 'Recovered viewport' } 'successful frame transaction' | Out-Null
        $report.recovered = @(Snapshot-Nodes)
        $report.recovered_editor_name = $editor.Current.Name
    }
    if (Find-Named 'Container metadata') { throw 'Hidden container metadata leaked into native tree' }
    if ($editor.Current.ControlType -ne [System.Windows.Automation.ControlType]::Edit) { throw 'Editor is not a native Edit control' }
    $report.initial = @(Snapshot-Nodes)
    Send-Command 'show'
    Wait-For { Find-Named 'Container metadata' } 'container before focus' | Out-Null
    $report.shown_before_focus = @(Snapshot-Nodes)
    Send-Command 'hide'
    Wait-For { -not (Find-Named 'Container metadata') } 'container hidden before focus' | Out-Null
    if ($Mode -eq 'Full') {
        [NanaProbeWindow]::ShowWindow($probeHandle, 9) | Out-Null
        [NanaProbeWindow]::SetForegroundWindow($probeHandle) | Out-Null
        Wait-For { [NanaProbeWindow]::GetForegroundWindow() -eq $probeHandle } 'probe foreground window' | Out-Null
        $editor.SetFocus()
        Wait-For { $editor.Current.HasKeyboardFocus } 'editor focus' | Out-Null
        Wait-For { [System.Windows.Automation.AutomationElement]::FocusedElement.Current.Name -eq $editorName } 'global UI Automation focus' | Out-Null
    }
    $valuePattern = [System.Windows.Automation.ValuePattern]$editor.GetCurrentPattern(
        [System.Windows.Automation.ValuePattern]::Pattern)
    $valuePattern.SetValue('Native UIA value')
    Wait-For { $valuePattern.Current.Value -eq 'Native UIA value' } 'native value update' | Out-Null
    $report.edited = @(Snapshot-Nodes)
    $report.edited_root = [ordered]@{ Name = $probeWindow.Current.Name; Handle = $probeHandle.ToInt64(); Type = $probeWindow.Current.ControlType.ProgrammaticName }
    Capture-Probe 'edited.png'
    Send-Command 'show'
    Wait-For { Find-Named 'Container metadata' } 'shown container metadata' | Out-Null
    $report.shown = @(Snapshot-Nodes)
    Send-Command 'hide'
    Wait-For { -not (Find-Named 'Container metadata') } 'hidden container metadata removal' | Out-Null
    $editor = Wait-For { Find-Named $editorName } 'editor after parent hides'
    $valuePattern = [System.Windows.Automation.ValuePattern]$editor.GetCurrentPattern(
        [System.Windows.Automation.ValuePattern]::Pattern)
    if ($valuePattern.Current.Value -ne 'Native UIA value') { throw 'Visibility transition lost edited value' }
    $report.hidden_again = @(Snapshot-Nodes)
    Capture-Probe 'hidden-again.png'
    if ($Auxiliary) {
        if ($primaryValue.Current.Value -ne 'Primary during failure') { throw 'Auxiliary updates changed the primary value' }
        Send-Command 'close-auxiliary'
        Wait-For { [NanaProbeWindow]::Find($probeProcess.Id, $targetTitle) -eq [IntPtr]::Zero } 'auxiliary close' | Out-Null
        $primaryValue.SetValue('Primary after close')
        Wait-For { $primaryValue.Current.Value -eq 'Primary after close' } 'primary update after auxiliary close' | Out-Null
        $report.primary_after_close = @(Snapshot-Nodes $primaryHandle)
    }
    $report.passed = $true
} catch {
    $failure = $_
    $report.error = $_.Exception.Message
    if ($probeWindow) {
        try { $report.failure_tree = @(Snapshot-Nodes) } catch {}
        try { Capture-Probe 'failure.png' } catch {}
    }
} finally {
    if ($started) {
        if (-not $probeProcess.HasExited) {
            Send-Command 'quit'
            if (-not $probeProcess.WaitForExit(5000)) { $probeProcess.Kill(); $probeProcess.WaitForExit() }
        }
        $capturedOutput = $stdout.GetAwaiter().GetResult()
        [IO.File]::WriteAllText((Join-Path $output 'stdout.txt'), $capturedOutput)
        [IO.File]::WriteAllText((Join-Path $output 'stderr.txt'), $stderr.GetAwaiter().GetResult())
        $report.exit_code = $probeProcess.ExitCode
        if ($report.passed -and $report.exit_code -ne 0) {
            $report.passed = $false
            $report.error = "Probe did not exit cleanly: $($report.exit_code)"
        }
        $states = @($capturedOutput -split '\r?\n' | Where-Object { $_.StartsWith('{') } | ForEach-Object { $_ | ConvertFrom-Json })
        $report.presented_states = $states
        if ($Auxiliary -and $report.passed) {
            foreach ($value in @('Primary during failure', 'Primary after close')) {
                if (-not ($states | Where-Object { $_.event -eq 'primary_state' -and $_.value -eq $value })) {
                    $report.passed = $false; $report.error = 'Primary did not present the expected state across auxiliary failure/close'
                }
            }
            if (-not ($states | Where-Object { $_.event -eq 'window_closed' -and $_.window -eq 1 })) {
                $report.passed = $false; $report.error = 'Auxiliary close was not delivered to application state'
            }
        }
        if ($Retry -and $report.passed) {
            $failed = @($states | Where-Object { $_.event -eq 'producer_failed' } | ForEach-Object { $_.phase })
            $expectedFailed = if ($InitialFailure) { @(4, 1, 2) } else { @(1, 2) }
            $invalidSubmit = @($states | Where-Object { $_.event -eq 'producer_submitted' -and $_.phase -ne 0 -and $_.phase -ne 3 })
            $invalidPresent = @($states | Where-Object { $_.event -eq 'state' -and $_.phase -ne 0 -and $_.phase -ne 3 })
            $recoveredSubmit = @($states | Where-Object { $_.event -eq 'producer_submitted' -and $_.phase -eq 3 })
            if (($failed -join ',') -ne ($expectedFailed -join ',') -or $invalidSubmit.Count -ne 0 -or $invalidPresent.Count -ne 0 -or $recoveredSubmit.Count -eq 0) {
                $report.passed = $false
                $report.error = 'Resource failure/submission lifecycle did not match the two failed frames'
            }
        }
        if ($report.passed -and -not ($states | Where-Object { $_.event -eq 'state' -and ($Mode -eq 'Semantics' -or $_.focused) -and $_.value -eq 'Native UIA value' })) {
            $report.passed = $false
            $report.error = 'Runtime did not report presenting the required edited state'
        }
    }
    [IO.File]::WriteAllText((Join-Path $output 'report.json'), ($report | ConvertTo-Json -Depth 8))
    $probeProcess.Dispose()
}
if ($failure) { throw $failure }
if (-not $report.passed) { throw $report.error }
$report | ConvertTo-Json -Depth 8
