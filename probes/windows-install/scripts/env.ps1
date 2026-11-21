# Records the runner environment. Facts only.
param([string]$Out)
$ErrorActionPreference = 'Continue'
& {
  "ImageOS=$env:ImageOS"
  "ImageVersion=$env:ImageVersion"
  "RUNNER_OS=$env:RUNNER_OS RUNNER_ARCH=$env:RUNNER_ARCH RUNNER_ENVIRONMENT=$env:RUNNER_ENVIRONMENT"
  $cv = Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion'
  "ProductName=$($cv.ProductName) DisplayVersion=$($cv.DisplayVersion) CurrentBuild=$($cv.CurrentBuild) UBR=$($cv.UBR) InstallationType=$($cv.InstallationType)"
  "OSVersion=$([Environment]::OSVersion.VersionString)"
  "LOCALAPPDATA=$env:LOCALAPPDATA"
  "TEMP=$env:TEMP"
  "GITHUB_WORKSPACE=$env:GITHUB_WORKSPACE"
  "whoami=$(whoami)"
  $id = [Security.Principal.WindowsIdentity]::GetCurrent()
  "elevated=$((New-Object Security.Principal.WindowsPrincipal $id).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator))"
  "pwsh=$($PSVersionTable.PSVersion)"
  "powershell.exe=$(powershell.exe -NoProfile -Command '$PSVersionTable.PSVersion.ToString()')"
  "--- rustc"
  rustc -Vv
  cargo -V
  "--- volumes (Get-Volume)"
  Get-Volume | Sort-Object DriveLetter | Format-Table DriveLetter, FileSystemLabel, FileSystem, FileSystemType, DriveType, Size, SizeRemaining -AutoSize | Out-String -Width 200
  foreach ($d in 'C:', 'D:') {
    "--- fsutil fsinfo volumeinfo $d"
    fsutil fsinfo volumeinfo $d 2>&1
  }
  "--- Defender"
  try { Get-MpComputerStatus | Format-List AMServiceEnabled, AntivirusEnabled, RealTimeProtectionEnabled, OnAccessProtectionEnabled | Out-String } catch { "Get-MpComputerStatus failed: $_" }
  try { "DisableRealtimeMonitoring=$((Get-MpPreference).DisableRealtimeMonitoring)" } catch { "Get-MpPreference failed: $_" }
  "--- long paths / dev mode"
  "LongPathsEnabled=$((Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\FileSystem').LongPathsEnabled)"
  "--- CRT DLLs present in System32"
  Get-ChildItem "$env:WINDIR\System32\vcruntime140*.dll", "$env:WINDIR\System32\msvcp140*.dll", "$env:WINDIR\System32\ucrtbase.dll" -ErrorAction SilentlyContinue | ForEach-Object { "$($_.Name) $($_.VersionInfo.FileVersion)" }
} 2>&1 | Tee-Object -FilePath $Out
