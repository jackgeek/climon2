# Local Windows VM

An optional VirtualBox VM running the Windows Server 2025 evaluation. When it exists, the factory runs the Implementer's `factory_run_windows_tests` here instead of on Windows CI: it is faster and allows 10 runs per attempt. Windows CI is still the gate for every candidate.

## Build it

Needs VirtualBox (with `VBoxManage` on `PATH`) and about 150 GB of disk.

```sh
factory/windows-vm/create-vm.sh create      # unattended install, about 30 minutes
factory/windows-vm/create-vm.sh provision   # SSH, Python + pytest, PowerShell 7, VS Build Tools, Rust
factory/windows-vm/create-vm.sh isolate     # host-only network, then the "clean" snapshot
```

State lives outside the repository in `~/VMs/climon2-win` (override with `CLIMON2_VM_HOME`): the ISO, pinned installers, the generated password and the SSH key.

## How a run works

`climon2_factory/windows_vm.py` takes a lock, powers the VM off, restores `clean`, and starts it headless. It then uploads the workspace, the locked crates and `run-acceptance.ps1` over SSH to `192.168.57.10`, and runs the phases `unpack`, `collect`, `build` (offline, from the crates the Linux container already vendored) and `test`. Finally it copies `report.xml` back and powers the VM off. The VM is on a host-only network with no gateway, so tests cannot reach the internet.

## Expiry

The evaluation runs for 180 days from the install date. Restoring the snapshot does not reset it. When it expires, delete the VM and run all three stages again.
