# Installs climon2 on x86_64 Windows (docs/windows-distribution.md, "The
# download script"). Run it as
#
#     irm <release URL>/install.ps1 | iex
#
# or, to pass options, as a script block:
#
#     & ([scriptblock]::Create((irm <release URL>/install.ps1))) -Dir <path> -NoModifyPath
#
# It works under Windows PowerShell 5.1 and PowerShell 7. Everything runs
# inside one script block, so `iex` leaves no variable or preference behind in
# the caller's session, and it never calls `exit`, which would close the
# caller's window. `@args` passes the script block form's options through.
& {
    [CmdletBinding()]
    param([string]$Dir, [switch]$NoModifyPath)

    $ErrorActionPreference = 'Stop'
    $ProgressPreference = 'SilentlyContinue'
    $Version = '0.1.0'
    $BaseUrl = $env:CLIMON2_RELEASE_BASE_URL
    if (-not $BaseUrl) { $BaseUrl = 'https://github.com/jackgeek/climon2/releases/download' }

    function Fail([string]$Message) { throw "climon2-install.ps1: error: $Message" }

    $Architecture = $env:PROCESSOR_ARCHITEW6432
    if (-not $Architecture) { $Architecture = $env:PROCESSOR_ARCHITECTURE }
    if ($Architecture -ne 'AMD64') { Fail 'supported platform: x86_64 Windows' }

    $Name = "climon2-$Version-windows-x86_64-msvc"
    $Archive = "$Name.zip"
    $Work = Join-Path ([IO.Path]::GetTempPath()) ("climon2-install-" + [Guid]::NewGuid())
    New-Item -ItemType Directory -Path $Work | Out-Null
    try {
        $Zip = Join-Path $Work $Archive
        $Sums = Join-Path $Work 'SHA256SUMS'
        try {
            Invoke-WebRequest -UseBasicParsing -Uri "$BaseUrl/v$Version/$Archive" -OutFile $Zip
        } catch { Fail "archive download failed: $_" }
        try {
            Invoke-WebRequest -UseBasicParsing -Uri "$BaseUrl/v$Version/SHA256SUMS" -OutFile $Sums
        } catch { Fail "checksum download failed: $_" }
        $Expected = @(Get-Content -LiteralPath $Sums | ForEach-Object {
            $Fields = $_ -split '  ', 2
            if ($Fields.Count -eq 2 -and $Fields[1] -ceq $Archive) { $Fields[0] }
        })
        if ($Expected.Count -ne 1) { Fail 'archive checksum must occur exactly once' }
        # .NET rather than Get-FileHash and Expand-Archive, which are script
        # modules: Windows PowerShell started from PowerShell 7 inherits a
        # module path where it cannot load them.
        $Stream = [IO.File]::OpenRead($Zip)
        try {
            $Hash = [Security.Cryptography.SHA256]::Create().ComputeHash($Stream)
        } finally { $Stream.Dispose() }
        $Actual = -join ($Hash | ForEach-Object { $_.ToString('x2') })
        if ($Actual -ne $Expected[0]) { Fail 'archive checksum verification failed' }
        try {
            Add-Type -AssemblyName System.IO.Compression.FileSystem
            [IO.Compression.ZipFile]::ExtractToDirectory($Zip, $Work)
        } catch { Fail "archive extraction failed: $_" }
        $Installer = Join-Path (Join-Path $Work $Name) 'install.exe'
        if (-not (Test-Path -LiteralPath $Installer -PathType Leaf)) {
            Fail 'archive installer is missing'
        }
        $Options = @()
        if ($Dir) { $Options += @('--dir', $Dir) }
        if ($NoModifyPath) { $Options += '--no-modify-path' }
        & $Installer @Options
        if ($LASTEXITCODE -ne 0) { Fail "installer failed with status $LASTEXITCODE" }
    } finally {
        Remove-Item -LiteralPath $Work -Recurse -Force -ErrorAction SilentlyContinue
    }
} @args
