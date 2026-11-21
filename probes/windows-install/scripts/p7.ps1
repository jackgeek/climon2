# Probe 7: CRT imports of Rust artifacts and the bundled ConPTY files.
param([string]$Root, [string]$Out, [string]$ConptyVersion = '1.25.261109001')
$ErrorActionPreference = 'Continue'
& {
  $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
  $dumpbin = & $vswhere -latest -products * -find 'VC\Tools\MSVC\**\bin\Hostx64\x64\dumpbin.exe' | Select-Object -Last 1
  "dumpbin=$dumpbin"
  "vs=$(& $vswhere -latest -products * -property catalog_productDisplayVersion)"

  $work = "$env:RUNNER_TEMP\conpty"
  New-Item -ItemType Directory -Force $work | Out-Null
  $id = 'microsoft.windows.console.conpty'
  $url = "https://api.nuget.org/v3-flatcontainer/$id/$ConptyVersion/$id.$ConptyVersion.nupkg"
  "nupkg=$url"
  try {
    Invoke-WebRequest -Uri $url -OutFile "$work\conpty.zip"
    "nupkg sha256=$((Get-FileHash "$work\conpty.zip" -Algorithm SHA256).Hash)"
    Expand-Archive -LiteralPath "$work\conpty.zip" -DestinationPath "$work\x" -Force
  } catch { "CONPTY DOWNLOAD FAILED: $_" }

  $files = [ordered]@{
    'rust exe, default'              = "$Root\target\release\winprobe.exe"
    'rust cdylib, default'           = "$Root\target\release\probedll.dll"
    'rust exe, +crt-static'          = "$Root\target-static\release\winprobe.exe"
    'rust cdylib, +crt-static'       = "$Root\target-static\release\probedll.dll"
    "conpty.dll x64 $ConptyVersion"      = "$work\x\runtimes\win-x64\native\conpty.dll"
    "OpenConsole.exe x64 $ConptyVersion" = "$work\x\build\native\runtimes\x64\OpenConsole.exe"
  }
  foreach ($name in $files.Keys) {
    $f = $files[$name]
    "=== $name"
    if (-not (Test-Path -LiteralPath $f)) { "NOT FOUND: $f"; continue }
    "path=$f size=$((Get-Item -LiteralPath $f).Length)"
    $txt = & $dumpbin /nologo /dependents $f
    $deps = $txt | Where-Object { $_ -match '^\s+\S+\.dll\s*$' } | ForEach-Object { $_.Trim() }
    "imports: $($deps -join ', ')"
    $crt = $deps | Where-Object { $_ -match '^(vcruntime140|msvcp140)' }
    "vcruntime140*/msvcp140* imports: $(if ($crt) { $crt -join ', ' } else { 'none' })"
    "ucrt (api-ms-win-crt-* / ucrtbase) imports: $(($deps | Where-Object { $_ -match '^(api-ms-win-crt|ucrtbase)' }).Count)"
  }
} 2>&1 | Tee-Object -FilePath $Out
