# Usage: run-acceptance.ps1 <unpack|collect|build|test> <subject-dir> <pytest args...>
# Runs one phase of an acceptance run inside the factory's Windows VM, under C:\factory.
# `build` exits 97 when the build fails, printing the log tail.
$Phase = $args[0]
$Subject = $args[1]
$Rest = @($args | Select-Object -Skip 2)
$env:RUSTUP_HOME = 'C:\rust\rustup'
$env:CARGO_HOME = 'C:\rust\cargo'
$env:CARGO_TARGET_DIR = 'C:\factory\target'
$env:PATH = "C:\Python312;C:\Python312\Scripts;C:\rust\cargo\bin;$env:PATH"
$env:PYTHONUTF8 = '1'
$env:PYTHONIOENCODING = 'utf-8'
[Console]::OutputEncoding = [Text.Encoding]::UTF8
$python = 'C:\Python312\python.exe'
$pytest = @('-m', 'pytest', '-p', 'no:cacheprovider', '--rootdir=.')

switch ($Phase) {
  'unpack' {
    New-Item -ItemType Directory -Force C:\factory\work | Out-Null
    tar.exe -xf C:\factory\work.tar -C C:\factory\work
    if ($LASTEXITCODE) { exit $LASTEXITCODE }
    if (Test-Path C:\factory\crates.tar) {
      New-Item -ItemType Directory -Force C:\factory\vendor | Out-Null
      tar.exe -xf C:\factory\crates.tar -C C:\factory\vendor
      if ($LASTEXITCODE) { exit $LASTEXITCODE }
      Set-Content -Encoding ascii C:\factory\cargo-config.toml @(
        '[source.crates-io]', 'replace-with = "vendored-sources"',
        '[source.vendored-sources]', 'directory = "C:/factory/vendor/crates"')
    }
    exit 0
  }
  'collect' {
    Set-Location C:\factory\work
    & $python @pytest --collect-only -q @Rest
    exit $LASTEXITCODE
  }
  'build' {
    Set-Location "C:\factory\work\$Subject" -ErrorAction SilentlyContinue
    if (-not $?) { "no such subject: $Subject"; exit 97 }
    $config = if (Test-Path C:\factory\cargo-config.toml) { '--config C:\factory\cargo-config.toml' } else { '' }
    cmd.exe /c "cargo build --release --locked --offline $config > C:\factory\build.log 2>&1"
    if ($LASTEXITCODE) {
      $log = Get-Content -Raw C:\factory\build.log
      $log.Substring([Math]::Max(0, $log.Length - 20000))
      exit 97
    }
    exit 0
  }
  'test' {
    Set-Location C:\factory\work
    $env:CLIMON2_BIN_DIR = 'C:\factory\target\release'
    & $python @pytest --junitxml=C:\factory\report.xml @Rest
    exit $LASTEXITCODE
  }
  default { "unknown phase: $Phase"; exit 2 }
}
