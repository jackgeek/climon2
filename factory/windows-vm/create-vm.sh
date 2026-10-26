#!/bin/bash
# Builds the factory's local Windows test VM in VirtualBox. Rerun from scratch when the
# Server 2025 evaluation expires: delete the VM, then run all three stages again.
#
#   create-vm.sh create      unattended Server 2025 install (about 30 minutes)
#   create-vm.sh provision   SSH, Python, pytest, PowerShell 7, VS Build Tools, Rust
#   create-vm.sh isolate     host-only network, then the "clean" snapshot every run restores
#
# State lives outside the repo in $VM_HOME: the ISO, installers, the VM password and SSH key.
set -euo pipefail

VM=climon2-win
VM_HOME="${CLIMON2_VM_HOME:-$HOME/VMs/climon2-win}"
HERE="$(cd "$(dirname "$0")" && pwd)"
REQUIREMENTS="$HERE/../containers/requirements.txt"
RUST_VERSION=1.96.0
NET=climon2-net
GUEST_IP=192.168.57.10
ISO_URL="https://go.microsoft.com/fwlink/?linkid=2293312&clcid=0x409&culture=en-us&country=us"
# name|url|sha256
INSTALLERS=(
  "python-3.12.10-amd64.exe|https://www.python.org/ftp/python/3.12.10/python-3.12.10-amd64.exe|67b5635e80ea51072b87941312d00ec8927c4db9ba18938f7ad2d27b328b95fb"
  "rustup-init.exe|https://static.rust-lang.org/rustup/archive/1.29.2/x86_64-pc-windows-msvc/rustup-init.exe|00f139da614dced87d3c3d4121c43eebdac3c1bf8a5ee86b9d80dfd71c8d70fd"
  "PowerShell-7.6.9-win-x64.msi|https://github.com/PowerShell/PowerShell/releases/download/v7.6.9/PowerShell-7.6.9-win-x64.msi|50d33c7494aaf82b987cfd4de3f232d04f91fdd4c0deeca8a669cced3e7cf8f9"
  # The Visual Studio bootstrapper is updated in place by Microsoft, so it is not pinned.
  "vs_buildtools.exe|https://aka.ms/vs/17/release/vs_buildtools.exe|"
)

# Guest control only gets a UAC-filtered token, and GitHub's Windows runners run without UAC.
UAC_OFF='reg add HKLM\Software\Microsoft\Windows\CurrentVersion\Policies\System /v EnableLUA /t REG_DWORD /d 0 /f'

SSH_OPTS=(-i "$VM_HOME/id_ed25519" -o IdentitiesOnly=yes -o StrictHostKeyChecking=no
  -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=10)

guest() {  # guest <exe> <args...>: run through guest control (no network needed)
  VBoxManage guestcontrol "$VM" run --username climon2 --passwordfile "$VM_HOME/password" \
    --wait-stdout --wait-stderr --exe "$@"
}

restart_guest() {  # applies the UAC change, and restarts VBoxService if it failed on first boot
  VBoxManage controlvm "$VM" acpipowerbutton
  for _ in $(seq 120); do
    VBoxManage showvminfo "$VM" --machinereadable | grep -q '^VMState="poweroff"' && break
    sleep 5
  done
  VBoxManage startvm "$VM" --type headless
}

wait_for_additions() {
  for _ in $(seq 240); do
    if guest 'C:\Windows\System32\cmd.exe' -- cmd.exe /c ver >/dev/null 2>&1; then return; fi
    sleep 15
  done
  echo "guest additions never came up" >&2; exit 1
}

wait_for_ssh() {  # wait_for_ssh <ssh args...>
  for _ in $(seq 60); do
    if ssh "${SSH_OPTS[@]}" "$@" "cmd /c ver" >/dev/null 2>&1; then return; fi
    sleep 5
  done
  echo "SSH never came up" >&2; exit 1
}

create() {
  mkdir -p "$VM_HOME"
  (umask 077
   [ -f "$VM_HOME/password" ] || LC_ALL=C tr -dc 'A-Za-z0-9' </dev/urandom | head -c 20 >"$VM_HOME/password"
   [ -f "$VM_HOME/id_ed25519" ] || ssh-keygen -q -t ed25519 -N "" -C "$VM" -f "$VM_HOME/id_ed25519")
  [ -f "$VM_HOME/server2025-eval.iso" ] || curl -sSfL --proto =https -o "$VM_HOME/server2025-eval.iso" "$ISO_URL"
  local disk="$HOME/VirtualBox VMs/$VM/$VM.vdi"
  VBoxManage createvm --name "$VM" --ostype Windows2025_64 --register
  VBoxManage modifyvm "$VM" --memory 16384 --cpus 8 --vram 128 --graphicscontroller vboxsvga \
    --firmware efi --nic1 nat --natpf1 "ssh,tcp,127.0.0.1,2222,,22" --audio-enabled off \
    --usb-ohci off --clipboard-mode disabled
  VBoxManage createmedium disk --filename "$disk" --size 131072 --format VDI
  VBoxManage storagectl "$VM" --name SATA --add sata --controller IntelAhci --portcount 2
  VBoxManage storageattach "$VM" --storagectl SATA --port 0 --device 0 --type hdd --medium "$disk"
  VBoxManage storageattach "$VM" --storagectl SATA --port 1 --device 0 --type dvddrive --medium emptydrive
  # Image 4: Datacenter (Desktop Experience), the edition GitHub's windows-2025 runner uses.
  VBoxManage unattended install "$VM" --iso="$VM_HOME/server2025-eval.iso" --image-index=4 \
    --user=climon2 --user-password-file="$VM_HOME/password" --admin-password-file="$VM_HOME/password" \
    --full-user-name=climon2 --hostname=climon2-win.local --install-additions --locale=en_US \
    --country=US --time-zone=UTC --start-vm=headless \
    --post-install-command="$UAC_OFF"
  wait_for_additions
}

provision() {
  local dir="$VM_HOME/installers"
  mkdir -p "$dir"
  for entry in "${INSTALLERS[@]}"; do
    IFS="|" read -r name url sha <<<"$entry"
    [ -f "$dir/$name" ] || curl -sSfL --proto =https -o "$dir/$name" "$url"
    if [ -n "$sha" ] && [ "$(shasum -a 256 "$dir/$name" | cut -d' ' -f1)" != "$sha" ]; then
      echo "$name does not match its pinned hash" >&2; exit 1
    fi
  done
  restart_guest
  wait_for_additions
  VBoxManage guestcontrol "$VM" copyto --username climon2 --passwordfile "$VM_HOME/password" \
    --target-directory 'C:\Users\climon2\' "$HERE/bootstrap.ps1" "$VM_HOME/id_ed25519.pub"
  guest 'C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe' -- powershell.exe \
    -NoProfile -ExecutionPolicy Bypass -File 'C:\Users\climon2\bootstrap.ps1' \
    -PublicKeyFile 'C:\Users\climon2\id_ed25519.pub'
  local target=(-p 2222 climon2@127.0.0.1)
  wait_for_ssh "${target[@]}"
  ssh "${SSH_OPTS[@]}" "${target[@]}" "cmd /c if not exist C:\\provision mkdir C:\\provision"
  scp "${SSH_OPTS[@]}" -P 2222 "$dir"/* "$REQUIREMENTS" "$HERE/provision.ps1" climon2@127.0.0.1:C:/provision/
  ssh "${SSH_OPTS[@]}" "${target[@]}" \
    "powershell -NoProfile -ExecutionPolicy Bypass -File C:\\provision\\provision.ps1 -RustVersion $RUST_VERSION"
  ssh "${SSH_OPTS[@]}" "${target[@]}" "cmd /c rmdir /s /q C:\\provision"
}

isolate() {
  # From here on the guest only ever runs untrusted code without internet access.
  VBoxManage list hostonlynets | grep -q "^Name: *$NET$" ||
    VBoxManage hostonlynet add --name "$NET" --netmask 255.255.255.0 \
      --lower-ip 192.168.57.100 --upper-ip 192.168.57.199 --enable
  VBoxManage controlvm "$VM" acpipowerbutton 2>/dev/null || true
  for _ in $(seq 60); do
    VBoxManage showvminfo "$VM" --machinereadable | grep -q '^VMState="poweroff"' && break
    sleep 5
  done
  VBoxManage modifyvm "$VM" --nic1 hostonlynet --host-only-net1 "$NET" --natpf1 delete ssh 2>/dev/null ||
    VBoxManage modifyvm "$VM" --nic1 hostonlynet --host-only-net1 "$NET"
  VBoxManage startvm "$VM" --type headless
  wait_for_additions
  # A static address without a gateway: the guest can reach this host and nothing else.
  guest 'C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe' -- powershell.exe -NoProfile -Command \
    "\$a = Get-NetAdapter | Where-Object Status -eq Up | Select-Object -First 1; \
     Remove-NetIPAddress -InterfaceIndex \$a.ifIndex -AddressFamily IPv4 -Confirm:\$false -ErrorAction SilentlyContinue; \
     Set-NetIPInterface -InterfaceIndex \$a.ifIndex -Dhcp Disabled; \
     New-NetIPAddress -InterfaceIndex \$a.ifIndex -IPAddress $GUEST_IP -PrefixLength 24 | Out-Null; \
     Set-NetConnectionProfile -InterfaceIndex \$a.ifIndex -NetworkCategory Private"
  wait_for_ssh "climon2@$GUEST_IP"
  if ssh "${SSH_OPTS[@]}" "climon2@$GUEST_IP" \
      "powershell -NoProfile -Command \"(Test-NetConnection 1.1.1.1 -Port 443 -WarningAction SilentlyContinue).TcpTestSucceeded\"" |
      grep -q True; then
    echo "the guest can still reach the internet" >&2; exit 1
  fi
  VBoxManage snapshot "$VM" delete clean 2>/dev/null || true
  # A live snapshot: restoring it resumes a booted guest with SSH already listening.
  VBoxManage snapshot "$VM" take clean --live
}

case "${1:-}" in
  create | provision | isolate) "$1" ;;
  *) echo "usage: $0 create|provision|isolate" >&2; exit 2 ;;
esac
