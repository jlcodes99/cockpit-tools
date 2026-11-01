#Requires -Version 5.1
<#
.SYNOPSIS
Diagnose the Codex Windows sandbox directory, or explicitly repair its owner.
.DESCRIPTION
Read-only by default. -Repair changes only .sandbox-bin's owner to LocalSystem,
which runs Codex's provisioning service. Existing DACLs and children are untouched.
An owner backup is required and written before any mutation. See the linked guide.
#>
[CmdletBinding(SupportsShouldProcess = $true, DefaultParameterSetName = 'Inspect')]
param(
    [Parameter()][string]$CodexHome = $(if ($env:CODEX_HOME) { $env:CODEX_HOME } else { Join-Path $env:USERPROFILE '.codex' }),
    [Parameter(Mandatory = $true, ParameterSetName = 'Repair')][switch]$Repair,
    [Parameter(Mandatory = $true, ParameterSetName = 'Restore')][string]$RestoreBackup,
    [Parameter(Mandatory = $true, ParameterSetName = 'Repair')][string]$BackupPath
)
$ErrorActionPreference = 'Stop'
if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) { throw 'Windows is required.' }

# Use a no-reparse, no-share-delete directory handle for both inspection and owner
# changes. Path checks followed by icacls/Set-Acl would leave a reparse race during UAC.
if (-not ('CockpitSandbox.DirectoryOwner' -as [type])) {
Add-Type -TypeDefinition @"
using System;
using System.ComponentModel;
using System.IO;
using System.Runtime.InteropServices;
using System.Security.Principal;
using System.Text;
using Microsoft.Win32.SafeHandles;
namespace CockpitSandbox {
public sealed class DirectoryOwner : IDisposable {
    readonly SafeFileHandle handle;
    [StructLayout(LayoutKind.Sequential)] struct UnicodeString { public ushort Length, MaximumLength; public IntPtr Buffer; }
    [StructLayout(LayoutKind.Sequential)] struct ObjectAttributes { public int Length; public IntPtr Root, Name; public uint Attributes; public IntPtr Security, Quality; }
    [StructLayout(LayoutKind.Sequential)] struct IoStatus { public IntPtr Status, Information; }
    [StructLayout(LayoutKind.Sequential, Pack=4)] struct TokenPrivileges { public uint Count; public long Luid; public uint Attributes; }
    [StructLayout(LayoutKind.Sequential)] struct FileInfo { public uint Attributes; public System.Runtime.InteropServices.ComTypes.FILETIME Creation, Access, Write; public uint Volume, SizeHigh, SizeLow, Links, IndexHigh, IndexLow; }
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern uint QueryDosDevice(string name, StringBuilder target, int length);
    [DllImport("ntdll.dll")] static extern int NtCreateFile(out IntPtr file, uint access, ref ObjectAttributes attributes, out IoStatus status, IntPtr allocation, uint fileAttributes, uint sharing, uint disposition, uint options, IntPtr ea, uint eaLength);
    [DllImport("ntdll.dll")] static extern uint RtlNtStatusToDosError(int status);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool GetFileInformationByHandle(SafeFileHandle file, out FileInfo info);
    [DllImport("advapi32.dll")] static extern uint GetSecurityInfo(SafeFileHandle handle, uint type, uint info, out IntPtr owner, out IntPtr group, out IntPtr dacl, out IntPtr sacl, out IntPtr descriptor);
    [DllImport("advapi32.dll")] static extern uint SetSecurityInfo(SafeFileHandle handle, uint type, uint info, byte[] owner, IntPtr group, IntPtr dacl, IntPtr sacl);
    [DllImport("kernel32.dll")] static extern IntPtr LocalFree(IntPtr memory);
    [DllImport("kernel32.dll")] static extern IntPtr GetCurrentProcess();
    [DllImport("advapi32.dll", SetLastError=true)] static extern bool OpenProcessToken(IntPtr process, uint access, out IntPtr token);
    [DllImport("advapi32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern bool LookupPrivilegeValue(string system, string name, out long luid);
    [DllImport("advapi32.dll", SetLastError=true)] static extern bool AdjustTokenPrivileges(IntPtr token, bool disable, ref TokenPrivileges value, uint length, out TokenPrivileges previous, out uint returned);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr handle);
    static void Check(uint error) { if (error != 0) throw new Win32Exception((int)error); }
    public DirectoryOwner(string path, bool writable) {
        // Only absolute, local disk paths. Reject relative segments and ADS before normalization.
        if (path.Length < 3 || !Char.IsLetter(path[0]) || path[1] != ':' || path[2] != '\\') throw new ArgumentException("Use an absolute local drive path.");
        foreach (string part in path.Substring(3).Split('\\')) if (part == "." || part == ".." || part.Contains(":")) throw new ArgumentException("Relative segments and streams are not allowed.");
        var device = new StringBuilder(32768);
        if (QueryDosDevice(path.Substring(0,2), device, device.Capacity) == 0) throw new Win32Exception();
        if (!System.Text.RegularExpressions.Regex.IsMatch(device.ToString(), @"^\\Device\\HarddiskVolume[0-9]+$")) throw new ArgumentException("Use a local disk, without mapped drives.");
        string native = device.ToString() + path.Substring(2);
        IntPtr text = Marshal.StringToHGlobalUni(native), name = IntPtr.Zero;
        try {
            var u = new UnicodeString { Length = checked((ushort)(native.Length * 2)), MaximumLength = checked((ushort)(native.Length * 2 + 2)), Buffer = text };
            name = Marshal.AllocHGlobal(Marshal.SizeOf(typeof(UnicodeString))); Marshal.StructureToPtr(u, name, false);
            var a = new ObjectAttributes { Length = Marshal.SizeOf(typeof(ObjectAttributes)), Name = name, Attributes = 0x1040 }; // OBJ_DONT_REPARSE | OBJ_CASE_INSENSITIVE
            IntPtr file; IoStatus status;
            int result = NtCreateFile(out file, writable ? 0xA0080u : 0x20080u, ref a, out status, IntPtr.Zero, 0, 3, 1, 0x4001, IntPtr.Zero, 0); // READ_CONTROL | FILE_READ_ATTRIBUTES (+ WRITE_OWNER), directory, backup intent
            if (result < 0) Check(RtlNtStatusToDosError(result));
            handle = new SafeFileHandle(file, true);
        } finally { if (name != IntPtr.Zero) Marshal.FreeHGlobal(name); Marshal.FreeHGlobal(text); }
    }
    public string OwnerSid {
        get { IntPtr owner, group, dacl, sacl, descriptor; Check(GetSecurityInfo(handle, 1, 1, out owner, out group, out dacl, out sacl, out descriptor));
            try { return new SecurityIdentifier(owner).Value; } finally { LocalFree(descriptor); } }
    }
    public string Identity {
        get { FileInfo info; if (!GetFileInformationByHandle(handle, out info)) throw new Win32Exception(); return info.Volume.ToString("X8") + ":" + info.IndexHigh.ToString("X8") + info.IndexLow.ToString("X8"); }
    }
    public void SetOwner(string sid) { var value = new SecurityIdentifier(sid); var bytes = new byte[value.BinaryLength]; value.GetBinaryForm(bytes,0); Check(SetSecurityInfo(handle, 1, 1, bytes, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero)); }
    public static IDisposable EnableRestorePrivilege() { return new RestorePrivilege(); }
    sealed class RestorePrivilege : IDisposable {
        IntPtr token; TokenPrivileges previous;
        public RestorePrivilege() {
            if (!OpenProcessToken(GetCurrentProcess(), 0x28, out token)) throw new Win32Exception();
            try { long luid; if (!LookupPrivilegeValue(null, "SeRestorePrivilege", out luid)) throw new Win32Exception();
                var value = new TokenPrivileges { Count = 1, Luid = luid, Attributes = 2 }; uint returned;
                if (!AdjustTokenPrivileges(token, false, ref value, (uint)Marshal.SizeOf(typeof(TokenPrivileges)), out previous, out returned)) throw new Win32Exception();
                Check((uint)Marshal.GetLastWin32Error());
            } catch { CloseHandle(token); token = IntPtr.Zero; throw; }
        }
        public void Dispose() { if (token != IntPtr.Zero) { TokenPrivileges unused; uint length; AdjustTokenPrivileges(token, false, ref previous, 0, out unused, out length); CloseHandle(token); token = IntPtr.Zero; } }
    }
    public void Dispose() { handle.Dispose(); }
}
}
"@
}
$target = $CodexHome.TrimEnd('\') + '\.sandbox-bin'
$change = $Repair -or $PSCmdlet.ParameterSetName -eq 'Restore'
$owner = $null
$privilege = $null
try {
    if ($change -and -not $WhatIfPreference) {
        $principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
        if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'Run PowerShell as administrator for owner repair/restore. Inspection needs no elevation.' }
        $privilege = [CockpitSandbox.DirectoryOwner]::EnableRestorePrivilege()
    }
    $owner = [CockpitSandbox.DirectoryOwner]::new($target, ($change -and -not $WhatIfPreference))
    $before = $owner.OwnerSid
    $identity = $owner.get_Identity()
    if ([string]::IsNullOrWhiteSpace($identity)) { throw 'Unable to identify directory; refusing operation.' }
    if ($Repair) {
        if ($before -eq 'S-1-5-18') { Write-Output 'Already owned by SYSTEM; no change.'; return }
        if ($PSCmdlet.ShouldProcess($target, 'Back up current owner and change owner to SYSTEM; preserve DACL and children')) {
            $backup = [ordered]@{ version = 1; target = $target; identity = $identity; ownerSid = $before }
            $bytes = [Text.Encoding]::UTF8.GetBytes(($backup | ConvertTo-Json))
            $stream = [IO.File]::Open($BackupPath, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
            try { $stream.Write($bytes, 0, $bytes.Length); $stream.Flush($true) } finally { $stream.Dispose() }
            $owner.SetOwner('S-1-5-18')
        }
    } elseif ($PSCmdlet.ParameterSetName -eq 'Restore') {
        $backup = Get-Content -LiteralPath $RestoreBackup -Raw | ConvertFrom-Json
        if ($backup.version -ne 1 -or $backup.target -ine $target -or $backup.identity -ne $identity) { throw 'Backup does not match this directory identity; refusing restore.' }
        if ($PSCmdlet.ShouldProcess($target, 'Restore backed-up owner; preserve current DACL and children')) { $owner.SetOwner($backup.ownerSid) }
    }
    [pscustomobject]@{ Target = $target; Identity = $identity; OwnerSid = $owner.OwnerSid; SystemOwned = ($owner.OwnerSid -eq 'S-1-5-18') }
} finally {
    if ($owner) { $owner.Dispose() }
    if ($privilege) { $privilege.Dispose() }
}
