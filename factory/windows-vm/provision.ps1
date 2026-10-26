# Runs once in the guest over SSH, with internet access, from C:\provision, which holds the
# installers (hash-checked on the host) and factory/containers/requirements.txt.
# Matches the GitHub windows-2025 runner where the acceptance tests depend on it.
param([Parameter(Mandatory)] [string] $RustVersion)
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"
Set-Location C:\provision

function Run([string] $Exe, [string[]] $Arguments, [int[]] $Ok = @(0)) {
    $p = Start-Process -FilePath $Exe -ArgumentList $Arguments -Wait -PassThru -NoNewWindow
    if ($Ok -notcontains $p.ExitCode) { throw "$Exe exited with $($p.ExitCode)" }
}
function Add-MachinePath([string] $Dir) {
    $path = [Environment]::GetEnvironmentVariable("Path", "Machine")
    if (($path -split ";") -notcontains $Dir) {
        [Environment]::SetEnvironmentVariable("Path", "$Dir;$path", "Machine")
    }
    $env:Path = "$Dir;$env:Path"
}

# Builds and test runs are much slower with real-time scanning; the VM holds nothing else.
Set-MpPreference -DisableRealtimeMonitoring $true -ErrorAction SilentlyContinue
Add-MpPreference -ExclusionPath "C:\" -ErrorAction SilentlyContinue
Stop-Service wuauserv -ErrorAction SilentlyContinue
Set-Service wuauserv -StartupType Disabled

Run .\python-3.12.10-amd64.exe @("/quiet", "InstallAllUsers=1", "PrependPath=1", "Include_test=0", "TargetDir=C:\Python312")
Add-MachinePath "C:\Python312"
Add-MachinePath "C:\Python312\Scripts"
Run C:\Python312\python.exe @("-m", "pip", "install", "-q", "--disable-pip-version-check", "--require-hashes", "--no-deps", "-r", "requirements.txt")

Run msiexec.exe @("/i", "C:\provision\PowerShell-7.6.9-win-x64.msi", "/qn", "/norestart", "ADD_PATH=1")

Run .\vs_buildtools.exe @("--quiet", "--wait", "--norestart", "--nocache", "--add", "Microsoft.VisualStudio.Workload.VCTools", "--includeRecommended") -Ok @(0, 3010)

[Environment]::SetEnvironmentVariable("RUSTUP_HOME", "C:\rust\rustup", "Machine")
[Environment]::SetEnvironmentVariable("CARGO_HOME", "C:\rust\cargo", "Machine")
$env:RUSTUP_HOME = "C:\rust\rustup"
$env:CARGO_HOME = "C:\rust\cargo"
Run .\rustup-init.exe @("-y", "--no-modify-path", "--profile", "minimal", "--default-toolchain", $RustVersion)
Add-MachinePath "C:\rust\cargo\bin"

New-Item -ItemType Directory -Force C:\factory | Out-Null
"provision complete"
