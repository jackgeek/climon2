# Probe 8: does Mark-of-the-Web (the Zone.Identifier stream) reach extracted or
# downloaded files?
param([string]$Exe, [string]$Out)
$ErrorActionPreference = 'Continue'

function Streams([string]$Path) {
  if (-not (Test-Path -LiteralPath $Path)) { return 'MISSING FILE' }
  $s = Get-Item -LiteralPath $Path -Stream * | ForEach-Object { $_.Stream }
  $z = Get-Content -LiteralPath $Path -Stream Zone.Identifier -ErrorAction SilentlyContinue
  if ($z) { "streams=[$($s -join ',')] Zone.Identifier={$($z -join ' | ')}" } else { "streams=[$($s -join ',')] Zone.Identifier=ABSENT" }
}
function Report([string]$Label, [string]$Dir) {
  "--- $Label"
  $found = Get-ChildItem -LiteralPath $Dir -Recurse -File -ErrorAction SilentlyContinue
  if (-not $found) { "  no files extracted" }
  foreach ($f in $found) { "  $($f.Name): $(Streams $f.FullName)" }
}

& {
  $w = "$env:LOCALAPPDATA\climon2-probe-p8"
  Remove-Item -Recurse -Force $w -ErrorAction SilentlyContinue
  New-Item -ItemType Directory -Force "$w\src" | Out-Null
  Copy-Item $Exe "$w\src\climon2.exe"
  Set-Content "$w\src\install.ps1" 'Write-Host hello'
  $zip = "$w\pkg.zip"
  Compress-Archive -Path "$w\src\*" -DestinationPath $zip
  Set-Content -LiteralPath $zip -Stream Zone.Identifier -Value '[ZoneTransfer]', 'ZoneId=3', 'HostUrl=https://example.invalid/pkg.zip'
  "zip: $(Streams $zip)"
  "source exe before zipping: $(Streams "$w\src\climon2.exe")"
  "Archive module in 5.1: $(powershell.exe -NoProfile -Command '(Get-Command Expand-Archive).Module.Version.ToString()')"
  "Archive module in 7: $((Get-Command Expand-Archive).Module.Version) (pwsh $($PSVersionTable.PSVersion))"

  powershell.exe -NoProfile -Command "Expand-Archive -LiteralPath '$zip' -DestinationPath '$w\out-ps51'" 2>&1
  Report 'Windows PowerShell 5.1 Expand-Archive' "$w\out-ps51"

  pwsh -NoProfile -Command "Expand-Archive -LiteralPath '$zip' -DestinationPath '$w\out-pwsh7'" 2>&1
  Report 'pwsh 7 Expand-Archive' "$w\out-pwsh7"

  New-Item -ItemType Directory -Force "$w\out-tar" | Out-Null
  "tar: $((& "$env:WINDIR\System32\tar.exe" --version) -join ' ')"
  & "$env:WINDIR\System32\tar.exe" -xf $zip -C "$w\out-tar" 2>&1
  Report 'tar.exe -xf' "$w\out-tar"

  New-Item -ItemType Directory -Force "$w\out-dotnet" | Out-Null
  Add-Type -AssemblyName System.IO.Compression.FileSystem
  [System.IO.Compression.ZipFile]::ExtractToDirectory($zip, "$w\out-dotnet")
  Report '.NET ZipFile.ExtractToDirectory (pwsh 7)' "$w\out-dotnet"

  # Explorer's own extraction, driven through the Shell.Application COM object.
  # The runner has no interactive desktop, so this may not work; time-boxed.
  New-Item -ItemType Directory -Force "$w\out-shell" | Out-Null
  $job = Start-Job -ScriptBlock {
    param($zip, $dest)
    $sh = New-Object -ComObject Shell.Application
    $sh.NameSpace($dest).CopyHere($sh.NameSpace($zip).Items(), 0x614)
    Start-Sleep -Seconds 5
  } -ArgumentList $zip, "$w\out-shell"
  if (Wait-Job $job -Timeout 60) { Receive-Job $job 2>&1 } else { 'Shell.Application extraction TIMED OUT after 60 s'; Stop-Job $job }
  Report 'Explorer zip folder (Shell.Application CopyHere)' "$w\out-shell"

  "=== downloads"
  $url = 'https://api.nuget.org/v3-flatcontainer/microsoft.windows.console.conpty/1.25.261109001/microsoft.windows.console.conpty.nuspec'
  "url=$url"
  powershell.exe -NoProfile -Command "Invoke-WebRequest -UseBasicParsing -Uri '$url' -OutFile '$w\dl-ps51.bin'" 2>&1
  "Windows PowerShell 5.1 Invoke-WebRequest -UseBasicParsing -OutFile: $(Streams "$w\dl-ps51.bin")"
  powershell.exe -NoProfile -Command "Invoke-RestMethod -Uri '$url' -OutFile '$w\dl-ps51-irm.bin'" 2>&1
  "Windows PowerShell 5.1 Invoke-RestMethod -OutFile: $(Streams "$w\dl-ps51-irm.bin")"
  pwsh -NoProfile -Command "Invoke-WebRequest -Uri '$url' -OutFile '$w\dl-pwsh7.bin'" 2>&1
  "pwsh 7 Invoke-WebRequest -OutFile: $(Streams "$w\dl-pwsh7.bin")"
  & "$env:WINDIR\System32\curl.exe" -sSL -o "$w\dl-curl.bin" $url 2>&1
  "curl.exe -o: $(Streams "$w\dl-curl.bin")"
  try {
    Start-BitsTransfer -Source $url -Destination "$w\dl-bits.bin" -ErrorAction Stop
    "Start-BitsTransfer: $(Streams "$w\dl-bits.bin")"
  } catch { "Start-BitsTransfer could not run: $_" }
} 2>&1 | Tee-Object -FilePath $Out
