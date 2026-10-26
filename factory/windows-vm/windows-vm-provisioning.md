# Provisioning the local Windows VM

How the factory's local Windows Server 2025 test VM is built, and what went wrong the first time. Read this before you rebuild the VM, for example when the 180-day evaluation expires.

## Prerequisites

- macOS host (x86_64) with VirtualBox 7.2+ and `VBoxManage` on `PATH`.
- About 150 GB of free disk space and 16 GB of spare RAM. The VM gets 16 GB RAM and 8 CPUs.
- Internet access during `create` and `provision` only.
- Rancher Desktop running when the factory uses the VM. The locked crates come from the container vendor cache.

## Steps

Run these from the repository root, one at a time, and check each one before starting the next.

```sh
factory/windows-vm/create-vm.sh create      # ISO download + unattended install, ~30-45 min
factory/windows-vm/create-vm.sh provision   # SSH, Python, pytest, PowerShell 7, VS Build Tools, Rust; ~1 h
factory/windows-vm/create-vm.sh isolate     # host-only network, then the "clean" snapshot
```

Long runs: redirect output to a log, such as `> /tmp/vm-provision.log 2>&1`, and watch the log.

### 1. create

- Generates `~/VMs/climon2-win/password` (mode 600) and `id_ed25519`, and downloads the evaluation ISO (fwlink `linkid=2293312`, build 26100.1742).
- Creates VM `climon2-win`:
  - EFI firmware, a 128 GB VDI on SATA;
  - nic1 is NAT with port forward `127.0.0.1:2222 → 22`.
- Starts `VBoxManage unattended install`:
  - image index 4, Datacenter (Desktop Experience), the same edition as GitHub's `windows-2025` runners;
  - user `climon2`, with Guest Additions;
  - the post-install command turns UAC off (`EnableLUA=0`).
- Done when `VBoxManage showvminfo climon2-win --machinereadable | grep GuestAdditionsRunLevel` shows `3`, and a screenshot shows the Server Manager desktop.

### 2. provision

1. **Reset the guest.** This applies the UAC change and restarts VBoxService (see Gotchas 1 and 2).
2. **Wait for guest control.** Run `cmd.exe /c ver` through guest control until it answers.
3. **Bootstrap SSH through guest control** (`bootstrap.ps1`):
   - starts sshd (Automatic startup);
   - writes the public key, copied in as a file, to `administrators_authorized_keys` with admin/SYSTEM-only ACLs;
   - sets `PasswordAuthentication no`;
   - opens port 22 for every firewall profile.
4. **Remove Defender with DISM** over SSH, then reboot over SSH and wait for SSH again.
5. **Run `provision.ps1` over SSH** from `C:\provision`. It:
   - disables Windows Update;
   - installs Python 3.12.10 to `C:\Python312`, then the hash-pinned `requirements.txt`;
   - installs PowerShell 7.6.9;
   - installs VS Build Tools (VCTools workload);
   - installs rustup with toolchain 1.96.0 under `C:\rust` (the `RUSTUP_HOME`/`CARGO_HOME` paths), and creates `C:\factory`.
6. **Verify.** Expect `climon2-win\climon2`, the Python, cargo and pwsh versions, and `cl.exe` usage text.

   ```sh
   S="ssh -i $HOME/VMs/climon2-win/id_ed25519 -o IdentitiesOnly=yes -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -p 2222 climon2@127.0.0.1"
   $S whoami
   $S "C:\Python312\python.exe --version && C:\rust\cargo\bin\cargo --version && pwsh -v"
   $S "\"C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe\" -products * -property installationPath"
   ```

### 3. isolate

- Creates host-only network `climon2-net` (192.168.57.0/24) and moves nic1 onto it.
- Gives the guest the static IP `192.168.57.10/24` with no gateway, shuts down cleanly and cold-boots to prove the address persists.
- Checks that the guest has no internet: `Test-NetConnection 1.1.1.1 -Port 443` must fail.
- Shuts down cleanly and takes the offline snapshot `clean`. Every run cold-boots it (about a minute).
- After this, the factory finds the VM automatically: `WindowsVM.available()` checks for the snapshot.
- Verify with `cd factory && uv run pytest -m windows_vm`.

## Gotchas (all already fixed in the scripts)

1. **VBoxService did not start on the first boot after setup.** Guest control failed with "The guest execution service is not ready (yet)" even though `GuestAdditionsRunLevel` was 3.
   - Fix: restart the guest once before using guest control.
2. **Guest control gets a UAC-filtered token** (Medium integrity). `Set-Service sshd`, and even `shutdown /r`, failed with "Access is denied".
   - Fix: turn UAC off. This is the post-install command; GitHub's runners also run without UAC.
   - On an already-installed VM, run `reg add HKLM\Software\Microsoft\Windows\CurrentVersion\Policies\System /v EnableLUA /t REG_DWORD /d 0 /f` from an elevated shell, then restart.
3. **ACPI shutdown (`acpipowerbutton`) was ignored** while the user was logged in, which left the VM session locked.
   - Before SSH exists, use `VBoxManage controlvm climon2-win reset`. Only do this while nothing is installing.
   - Once SSH works, use `ssh … "shutdown /r /t 0"`.
4. **Guest control splits arguments on spaces.** Passing the SSH public key as a parameter broke it into three.
   - Fix: copy the `.pub` file in and pass its path (`-PublicKeyFile`).
5. **The preinstalled `OpenSSH-Server-In-TCP` firewall rule only covers the Private profile**, and the VM's NIC is on Public. The symptom was SSH connecting, then "Connection timed out during banner exchange".
   - Fix: `Set-NetFirewallRule -Name OpenSSH-Server-In-TCP -Profile Any`.
6. **The SSH default shell is already `cmd.exe`.** Wrapping commands in another `cmd /c "…"` mangles the quoting; the error was `'ver"' is not recognized`.
   - Fix: send plain cmd command lines, such as `ver` or `if not exist C:\provision mkdir C:\provision`.
7. **Defender's AMSI blocks any PowerShell that disables Defender** (`ScriptContainedMaliciousContent`), whether it is in a script file or on the command line.
   - Fix: remove the feature with `dism /online /disable-feature /featurename:Windows-Defender /remove /norestart`, then reboot.
   - Exit code 3010 means success with a reboot needed. Over ssh it shows as 194.
8. **Don't edit repository files while a factory implementation attempt is running.** The factory snapshots everything outside `climon2/` when an attempt starts, and reverts any change when the attempt ends. That reverted these script fixes once, and deleted `build.sh`.
9. **`VBoxManage controlvm … screenshotpng` sometimes fails** with `NS_ERROR_FAILURE` while the display is asleep.
   - Fix: send a key first (`keyboardputscancode 2a aa`, a Shift press), then retry.

10. **Guest control passes everything after `--` as arguments**; it supplies argv[0] itself. `-- shutdown.exe /s /t 0` printed shutdown's usage text, and `-- powershell.exe -Command …` ran the whole line as a nested command.
    - Fix: the `guest` helper takes `<exe> <args…>` with no argv[0].
11. **Guest control does not preserve quoting** of a PowerShell script argument (parser errors such as "Missing closing ')'").
    - Fix: `guest_ps` sends scripts as `-EncodedCommand` (base64 of UTF-16LE).
12. **Restoring a live snapshot left this guest frozen** (the clock stopped, no ARP replies, guest control stuck at "starting").
    - Fix: take an offline snapshot after a clean shutdown; runs cold-boot it.
13. **A hard power-off soon after a configuration change loses it.** The static IP vanished (DHCP again) because the registry had not been flushed to disk.
    - Fix: `shut_down` stops the guest with `Stop-Computer -Force` through guest control, and only forces a power-off after 5 minutes.
14. **Two hard power-offs in a row left Windows at the Recovery screen** ("Windows didn't load correctly"), so no services started.
    - Fix: `provision.ps1` runs `bcdedit /set {current} recoveryenabled No` and `bootstatuspolicy IgnoreAllFailures`. Recover a stuck VM with `VBoxManage controlvm climon2-win reset`.
15. **The first network adapter may not be `Up` yet** when guest control first answers after boot.
    - Fix: the isolate script polls `Get-NetAdapter` for up to 2 minutes.

## Debugging inside the guest without SSH

- Screenshot: `VBoxManage controlvm climon2-win screenshotpng /tmp/vm.png`.
- Type into the console:
  - `VBoxManage controlvm climon2-win keyboardputstring '…'`, then `keyboardputscancode 1c 9c` for Enter;
  - Win+R is `keyboardputscancode e0 5b 13 93 e0 db`;
  - on a UAC prompt, press Left (`e0 4b e0 cb`) then Enter to choose Yes.
- Run a command through guest control:
  `VBoxManage guestcontrol climon2-win run --username climon2 --passwordfile ~/VMs/climon2-win/password --wait-stdout --wait-stderr --exe 'C:\Windows\System32\cmd.exe' -- /c ver`
- `/VirtualBox/GuestInfo/*` guest properties (`VBoxManage guestproperty enumerate climon2-win`) only appear while VBoxService is running.

## Rebuilding

- The evaluation lasts 180 days from install. Restoring the snapshot does not reset it.
- To rebuild:
  1. `VBoxManage unregistervm climon2-win --delete`
  2. Optionally keep `~/VMs/climon2-win` so the ISO and installers aren't downloaded again.
  3. Run the three stages again.
