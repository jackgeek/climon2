# Runs once in the guest through VirtualBox guest control, before the VM has SSH.
# Enables OpenSSH Server (preinstalled on Server 2025) with key-only admin login.
param([Parameter(Mandatory)] [string] $PublicKey)
$ErrorActionPreference = "Stop"

$keys = "$env:ProgramData\ssh\administrators_authorized_keys"
New-Item -ItemType Directory -Force "$env:ProgramData\ssh" | Out-Null
Set-Service -Name sshd -StartupType Automatic
Start-Service sshd  # first start generates host keys and sshd_config
Set-Content -Path $keys -Value $PublicKey -Encoding ascii
icacls.exe $keys /inheritance:r /grant "*S-1-5-32-544:F" /grant "*S-1-5-18:F" | Out-Null
$config = "$env:ProgramData\ssh\sshd_config"
(Get-Content $config) -replace '^#?PasswordAuthentication .*', 'PasswordAuthentication no' |
    Set-Content $config -Encoding ascii
Restart-Service sshd
if (-not (Get-NetFirewallRule -Name OpenSSH-Server-In-TCP -ErrorAction SilentlyContinue)) {
    New-NetFirewallRule -Name OpenSSH-Server-In-TCP -DisplayName "OpenSSH Server (sshd)" `
        -Enabled True -Direction Inbound -Protocol TCP -Action Allow -LocalPort 22 | Out-Null
}
"bootstrap complete"
