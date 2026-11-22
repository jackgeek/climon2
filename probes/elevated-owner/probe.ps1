# Who owns a new file or folder? Measures the token's default owner and the
# owner Windows actually writes for objects created with and without an
# explicit owner. Read-only for the machine unless -Logon is given, which
# creates a throwaway local administrator and changes nodefaultadminowner
# (CI only: never run -Logon on a machine you keep).
param(
    [switch]$Logon,
    [string]$Root = (Join-Path $env:TEMP "c2-owner-probe")
)
$ErrorActionPreference = 'Stop'

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.ComponentModel;

public static class OwnerProbe {
    [DllImport("advapi32.dll", SetLastError=true)]
    static extern bool OpenProcessToken(IntPtr p, uint access, out IntPtr token);
    [DllImport("kernel32.dll")] static extern IntPtr GetCurrentProcess();
    [DllImport("kernel32.dll", SetLastError=true)] public static extern bool CloseHandle(IntPtr h);
    [DllImport("advapi32.dll", SetLastError=true)]
    static extern bool GetTokenInformation(IntPtr t, int cls, IntPtr buf, int len, out int ret);
    [DllImport("advapi32.dll", SetLastError=true, CharSet=CharSet.Unicode)]
    static extern bool ConvertSidToStringSidW(IntPtr sid, out IntPtr str);
    [DllImport("kernel32.dll")] static extern IntPtr LocalFree(IntPtr p);
    [DllImport("advapi32.dll", SetLastError=true, CharSet=CharSet.Unicode)]
    static extern bool ConvertStringSecurityDescriptorToSecurityDescriptorW(string sddl, uint rev, out IntPtr sd, IntPtr size);
    [DllImport("kernel32.dll", SetLastError=true, CharSet=CharSet.Unicode)]
    static extern bool CreateDirectoryW(string path, IntPtr sa);
    [DllImport("kernel32.dll", SetLastError=true, CharSet=CharSet.Unicode)]
    static extern IntPtr CreateFileW(string path, uint access, uint share, IntPtr sa, uint disp, uint flags, IntPtr tmpl);
    [DllImport("advapi32.dll", SetLastError=true, CharSet=CharSet.Unicode)]
    static extern uint GetNamedSecurityInfoW(string name, int type, uint info, out IntPtr owner, IntPtr g, IntPtr d, IntPtr s, out IntPtr sd);
    [DllImport("advapi32.dll", SetLastError=true, CharSet=CharSet.Unicode)]
    public static extern bool LogonUserW(string user, string domain, string pw, int type, int provider, out IntPtr token);
    [DllImport("advapi32.dll", SetLastError=true)] public static extern bool ImpersonateLoggedOnUser(IntPtr t);
    [DllImport("advapi32.dll", SetLastError=true)] public static extern bool RevertToSelf();

    [StructLayout(LayoutKind.Sequential)]
    struct SECURITY_ATTRIBUTES { public int nLength; public IntPtr sd; public int inherit; }

    public static IntPtr SelfToken() {
        IntPtr t;
        if (!OpenProcessToken(GetCurrentProcess(), 0x0008 /*TOKEN_QUERY*/, out t)) throw new Win32Exception();
        return t;
    }

    static IntPtr Info(IntPtr t, int cls) {
        int len; GetTokenInformation(t, cls, IntPtr.Zero, 0, out len);
        if (len == 0) len = 64;
        IntPtr buf = Marshal.AllocHGlobal(len);
        if (!GetTokenInformation(t, cls, buf, len, out len)) { int e = Marshal.GetLastWin32Error(); Marshal.FreeHGlobal(buf); throw new Win32Exception(e); }
        return buf;
    }

    public static string Sid(IntPtr sid) {
        IntPtr s; if (!ConvertSidToStringSidW(sid, out s)) throw new Win32Exception();
        string r = Marshal.PtrToStringUni(s); LocalFree(s); return r;
    }

    // TokenUser = 1, TokenOwner = 4: both start with a pointer to a SID.
    public static string User(IntPtr t)  { IntPtr b = Info(t, 1); try { return Sid(Marshal.ReadIntPtr(b)); } finally { Marshal.FreeHGlobal(b); } }
    public static string Owner(IntPtr t) { IntPtr b = Info(t, 4); try { return Sid(Marshal.ReadIntPtr(b)); } finally { Marshal.FreeHGlobal(b); } }
    // TokenElevationType = 18: 1 default (no split), 2 full (elevated half), 3 limited.
    public static int ElevationType(IntPtr t) { IntPtr b = Info(t, 18); try { return Marshal.ReadInt32(b); } finally { Marshal.FreeHGlobal(b); } }
    // TokenElevation = 20.
    public static bool Elevated(IntPtr t) { IntPtr b = Info(t, 20); try { return Marshal.ReadInt32(b) != 0; } finally { Marshal.FreeHGlobal(b); } }
    // TokenIntegrityLevel = 25: TOKEN_MANDATORY_LABEL starts with a SID pointer.
    public static string Integrity(IntPtr t) { IntPtr b = Info(t, 25); try { return Sid(Marshal.ReadIntPtr(b)); } finally { Marshal.FreeHGlobal(b); } }
    // TokenLinkedToken = 19. Without SeTcbPrivilege this is an identification
    // token, which can still be queried.
    public static IntPtr Linked(IntPtr t) {
        int len; IntPtr buf = Marshal.AllocHGlobal(IntPtr.Size);
        try {
            if (!GetTokenInformation(t, 19, buf, IntPtr.Size, out len)) return IntPtr.Zero;
            return Marshal.ReadIntPtr(buf);
        } finally { Marshal.FreeHGlobal(buf); }
    }

    static IntPtr Sa(string sddl, out IntPtr sd) {
        sd = IntPtr.Zero;
        if (sddl == null) return IntPtr.Zero;
        if (!ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl, 1, out sd, IntPtr.Zero)) throw new Win32Exception();
        var sa = new SECURITY_ATTRIBUTES { nLength = Marshal.SizeOf(typeof(SECURITY_ATTRIBUTES)), sd = sd, inherit = 0 };
        IntPtr p = Marshal.AllocHGlobal(sa.nLength); Marshal.StructureToPtr(sa, p, false); return p;
    }

    public static void Dir(string path, string sddl) {
        IntPtr sd; IntPtr sa = Sa(sddl, out sd);
        try { if (!CreateDirectoryW(path, sa)) throw new Win32Exception(); }
        finally { if (sa != IntPtr.Zero) { Marshal.FreeHGlobal(sa); LocalFree(sd); } }
    }

    public static void File(string path, string sddl) {
        IntPtr sd; IntPtr sa = Sa(sddl, out sd);
        try {
            IntPtr h = CreateFileW(path, 0x40000000 /*GENERIC_WRITE*/, 0, sa, 1 /*CREATE_NEW*/, 0x80, IntPtr.Zero);
            if (h == new IntPtr(-1)) throw new Win32Exception();
            CloseHandle(h);
        } finally { if (sa != IntPtr.Zero) { Marshal.FreeHGlobal(sa); LocalFree(sd); } }
    }

    public static string OwnerOf(string path) {
        IntPtr owner, sd;
        uint e = GetNamedSecurityInfoW(path, 1 /*SE_FILE_OBJECT*/, 1 /*OWNER_SECURITY_INFORMATION*/, out owner, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero, out sd);
        if (e != 0) throw new Win32Exception((int)e);
        try { return Sid(owner); } finally { LocalFree(sd); }
    }
}
'@

function Get-Name([string]$sid) {
    try { (New-Object System.Security.Principal.SecurityIdentifier $sid).Translate([System.Security.Principal.NTAccount]).Value } catch { $sid }
}

function Get-Policy {
    $sys = Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System'
    $lsa = Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\Lsa'
    [ordered]@{
        EnableLUA                 = $sys.EnableLUA
        FilterAdministratorToken  = $sys.FilterAdministratorToken
        ConsentPromptBehaviorAdmin = $sys.ConsentPromptBehaviorAdmin
        nodefaultadminowner       = $lsa.nodefaultadminowner
    }
}

function Show-Token([string]$label, [IntPtr]$t) {
    $u = [OwnerProbe]::User($t); $o = [OwnerProbe]::Owner($t)
    "token[$label] user=$u ($(Get-Name $u)) rid=$($u.Split('-')[-1])"
    "token[$label] default-owner=$o ($(Get-Name $o))"
    "token[$label] elevation-type=$([OwnerProbe]::ElevationType($t)) elevated=$([OwnerProbe]::Elevated($t)) integrity=$([OwnerProbe]::Integrity($t))"
}

# Creates one object of each kind in $dir as the current thread's token and
# prints each owner. $user is the token user's SID.
function Test-Create([string]$label, [string]$dir, [string]$user) {
    $private = "O:${user}D:P(A;OICI;FA;;;$user)"
    $dacl = "D:P(A;OICI;FA;;;$user)"
    $cases = @(
        @('dir   no-descriptor',       { [OwnerProbe]::Dir("$dir\a-dir", $null) },              "$dir\a-dir"),
        @('file  no-descriptor',       { [OwnerProbe]::File("$dir\b-file", $null) },            "$dir\b-file"),
        @('file  inherited-in-new-dir', { [OwnerProbe]::File("$dir\a-dir\c-file", $null) },     "$dir\a-dir\c-file"),
        @('dir   dacl-only-descriptor', { [OwnerProbe]::Dir("$dir\d-dir", $dacl) },            "$dir\d-dir"),
        @('dir   explicit-owner',      { [OwnerProbe]::Dir("$dir\e-dir", $private) },           "$dir\e-dir"),
        @('file  explicit-owner',      { [OwnerProbe]::File("$dir\f-file", $private) },         "$dir\f-file")
    )
    foreach ($c in $cases) {
        try { & $c[1]; $o = [OwnerProbe]::OwnerOf($c[2]); "create[$label] $($c[0]) owner=$o ($(Get-Name $o))" }
        catch { "create[$label] $($c[0]) FAILED: $($_.Exception.Message)" }
    }
}

"os=$([System.Environment]::OSVersion.VersionString) build=$((Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion').CurrentBuild)"
"policy " + ((Get-Policy).GetEnumerator() | ForEach-Object { "$($_.Key)=$($_.Value)" }) -join ' '

$self = [OwnerProbe]::SelfToken()
Show-Token 'self' $self
$linked = [OwnerProbe]::Linked($self)
if ($linked -ne [IntPtr]::Zero) { Show-Token 'self-linked' $linked; [void][OwnerProbe]::CloseHandle($linked) }
else { "token[self-linked] none" }

$run = Join-Path $Root ("self-" + [guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Force $run | Out-Null
Test-Create 'self' $run ([OwnerProbe]::User($self))
[void][OwnerProbe]::CloseHandle($self)

if (-not $Logon) { Remove-Item -Recurse -Force $run; return }

# CI only. A second, non-RID-500 administrator, logged on afresh under each
# value of nodefaultadminowner. Objects are created while impersonating its
# token, so Windows takes the owner from that token.
$shared = Join-Path $Root 'shared'
New-Item -ItemType Directory -Force $shared | Out-Null
icacls $shared /grant '*S-1-1-0:(OI)(CI)F' | Out-Null
$name = 'c2probe'
$pw = 'P' + [guid]::NewGuid().ToString('N') + '!a1'
net user $name $pw /add /y | Out-Null
net localgroup Administrators $name /add | Out-Null
$lsaKey = 'HKLM:\SYSTEM\CurrentControlSet\Control\Lsa'
try {
    foreach ($value in @($null, 0, 1)) {
        if ($null -eq $value) { Remove-ItemProperty $lsaKey -Name nodefaultadminowner -ErrorAction SilentlyContinue }
        else { Set-ItemProperty $lsaKey -Name nodefaultadminowner -Type DWord -Value $value }
        $tag = if ($null -eq $value) { 'absent' } else { "$value" }
        foreach ($type in @(2, 4)) {   # 2 interactive, 4 batch
            $label = "c2probe nodefaultadminowner=$tag logon-type=$type"
            $t = [IntPtr]::Zero
            if (-not [OwnerProbe]::LogonUserW($name, '.', $pw, $type, 0, [ref]$t)) {
                "token[$label] LogonUser FAILED: $([ComponentModel.Win32Exception]::new([Runtime.InteropServices.Marshal]::GetLastWin32Error()).Message)"
                continue
            }
            Show-Token $label $t
            $l = [OwnerProbe]::Linked($t)
            if ($l -ne [IntPtr]::Zero) { Show-Token "$label linked" $l; [void][OwnerProbe]::CloseHandle($l) }
            $dir = Join-Path $shared ("$tag-$type")
            New-Item -ItemType Directory -Force $dir | Out-Null
            if ([OwnerProbe]::ImpersonateLoggedOnUser($t)) {
                try { Test-Create $label $dir ([OwnerProbe]::User($t)) } finally { [void][OwnerProbe]::RevertToSelf() }
            } else { "create[$label] impersonation FAILED" }
            [void][OwnerProbe]::CloseHandle($t)
        }
    }
} finally {
    Remove-ItemProperty $lsaKey -Name nodefaultadminowner -ErrorAction SilentlyContinue
    net user $name /delete | Out-Null
}
