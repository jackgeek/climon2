# Creates a ReFS volume R: on a virtual disk so the filesystem probes can run
# on ReFS. The runner's own volumes are both NTFS.
param([string]$Out)
$ErrorActionPreference = 'Continue'
& {
  $vhd = 'D:\refs-probe.vhdx'
  $dp = "$env:RUNNER_TEMP\refs.diskpart"
  Set-Content $dp "create vdisk file=`"$vhd`" maximum=20480 type=expandable", "select vdisk file=`"$vhd`"", 'attach vdisk'
  diskpart /s $dp 2>&1
  try {
    $disk = Get-Disk | Where-Object { $_.PartitionStyle -eq 'RAW' } | Select-Object -First 1
    "raw disk: number=$($disk.Number) size=$($disk.Size) name=$($disk.FriendlyName)"
    $disk | Initialize-Disk -PartitionStyle GPT -PassThru |
      New-Partition -UseMaximumSize -DriveLetter R |
      Format-Volume -FileSystem ReFS -NewFileSystemLabel refsprobe -Confirm:$false |
      Format-List DriveLetter, FileSystem, FileSystemType, Size | Out-String
  } catch { "REFS SETUP FAILED: $_" }
  "--- fsutil fsinfo volumeinfo R:"
  fsutil fsinfo volumeinfo R: 2>&1
  "--- fsutil fsinfo refsinfo R:"
  fsutil fsinfo refsinfo R: 2>&1
} 2>&1 | Tee-Object -FilePath $Out
