$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
function Expand-Bundle($appinstaller, $dest) {
    $file = "$env:RUNNER_TEMP\$(Split-Path $dest -Leaf).appinstaller"
    Invoke-WebRequest -UseBasicParsing $appinstaller -OutFile $file
    $xml = [xml](Get-Content -Raw $file)
    $uri = $xml.AppInstaller.MainBundle.Uri
    Write-Host "bundle $uri"
    $bundle = "$env:RUNNER_TEMP\$(Split-Path $dest -Leaf).zip"
    Invoke-WebRequest -UseBasicParsing $uri -OutFile $bundle
    $outer = "$dest-bundle"
    Expand-Archive $bundle $outer -Force
    $msix = Get-ChildItem $outer -Filter *.msix | Where-Object { $_.Name -match 'x64|amd64' } | Select-Object -First 1
    if (-not $msix) { $msix = Get-ChildItem $outer -Filter *.msix | Select-Object -First 1 }
    Write-Host "package $($msix.Name)"
    Copy-Item $msix.FullName "$dest.zip"
    Expand-Archive "$dest.zip" $dest -Force
}
Expand-Bundle 'https://aka.ms/ttd/download' 'C:\ttdpkg'
Expand-Bundle 'https://aka.ms/windbg/download' 'C:\windbgpkg'
Get-ChildItem C:\ttdpkg -Recurse -Include TTD.exe,TTDRecord*.dll | Select-Object FullName | Format-Table -AutoSize | Out-String -Width 300 | Write-Host
Get-ChildItem C:\windbgpkg -Recurse -Include cdb.exe,TTDReplay*.dll,JsProvider.dll,TTD.exe | Select-Object FullName | Format-Table -AutoSize | Out-String -Width 300 | Write-Host
Get-PSDrive C,D | Format-Table -AutoSize | Out-String | Write-Host
