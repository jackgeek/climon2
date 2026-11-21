# Probe 10: registry type of the user PATH, and a type-preserving edit.
# Runs under both Windows PowerShell 5.1 and pwsh 7. Restores the value.
param([string]$Out)
$ErrorActionPreference = 'Continue'
$noExpand = [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames

function Show([string]$Label) {
  $k = Get-Item 'HKCU:\Environment'
  if ($k.GetValueNames() -notcontains 'Path') { "${Label}: Path value ABSENT"; return }
  "${Label}: kind=$($k.GetValueKind('Path'))"
  "${Label}: raw=$($k.GetValue('Path', $null, $noExpand))"
  "${Label}: expanded=$($k.GetValue('Path'))"
}

& {
  "shell=$($PSVersionTable.PSVersion) edition=$($PSVersionTable.PSEdition)"
  "HKCU\Environment value names: $((Get-Item 'HKCU:\Environment').GetValueNames() -join ', ')"
  Show 'before'
  "reg query: $((reg query HKCU\Environment /v Path 2>&1) -join ' ')"
  $m = Get-Item 'HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager\Environment'
  "machine Path kind=$($m.GetValueKind('Path'))"

  $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true)
  $had = $key.GetValueNames() -contains 'Path'
  if ($had) {
    $origKind = $key.GetValueKind('Path')
    $orig = $key.GetValue('Path', $null, $noExpand)
  } else {
    $origKind = [Microsoft.Win32.RegistryValueKind]::ExpandString
    $orig = ''
  }
  $entry = '%LOCALAPPDATA%\climon2-probe\bin'
  $new = if ($orig) { $orig.TrimEnd(';') + ';' + $entry } else { $entry }

  "--- write back with RegistryKey.SetValue(name, value, <original kind>)"
  $key.SetValue('Path', $new, $origKind)
  Show 'after SetValue'
  $back = $key.GetValue('Path', $null, $noExpand)
  "after SetValue: kind preserved=$($key.GetValueKind('Path') -eq $origKind) raw equals what was written=$($back -ceq $new) original is an exact prefix=$($back.StartsWith($orig.TrimEnd(';')))"

  "--- [Environment]::SetEnvironmentVariable('Path', <same unexpanded string>, 'User')"
  [Environment]::SetEnvironmentVariable('Path', $new, 'User')
  Show 'after SetEnvironmentVariable'
  "after SetEnvironmentVariable: kind preserved=$($key.GetValueKind('Path') -eq $origKind) raw equals what was written=$($key.GetValue('Path', $null, $noExpand) -ceq $new)"

  "--- [Environment]::GetEnvironmentVariable('Path','User') (what a naive read returns)"
  "GetEnvironmentVariable: $([Environment]::GetEnvironmentVariable('Path', 'User'))"

  "--- restore"
  if ($had) { $key.SetValue('Path', $orig, $origKind) } else { $key.DeleteValue('Path') }
  Show 'restored'
  if ($had) { "restored: identical=$(($key.GetValue('Path', $null, $noExpand) -ceq $orig) -and ($key.GetValueKind('Path') -eq $origKind))" }
  $key.Close()
} 2>&1 | Tee-Object -FilePath $Out
