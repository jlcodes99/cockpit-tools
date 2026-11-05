#Requires -Version 5.1
# Run in an elevated Windows PowerShell session. Uses isolated temporary fixtures.
$ErrorActionPreference = 'Stop'
$tool = Join-Path $PSScriptRoot 'codex-sandbox-owner.ps1'
$root = Join-Path ([IO.Path]::GetTempPath()) ('cockpit-sandbox-test-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $root | Out-Null
function Assert($condition, [string]$message) { if (-not $condition) { throw $message } }
try {
    $homeDir = Join-Path $root 'Codex Home with spaces'
    $bin = Join-Path $homeDir '.sandbox-bin'
    New-Item -ItemType Directory -Path $bin -Force | Out-Null
    $child = Join-Path $bin 'runner-fixture.txt'
    [IO.File]::WriteAllText($child, 'preserve me')
    $ownerBackup = Join-Path $root 'owner.json'
    $original = Get-Acl -LiteralPath $bin
    $originalOwner = $original.GetOwner([Security.Principal.SecurityIdentifier]).Value
    $sections = [Security.AccessControl.AccessControlSections]::Access
    $originalDacl = $original.GetSecurityDescriptorSddlForm($sections)
    $childSddl = (Get-Acl -LiteralPath $child).Sddl

    $report = & $tool -CodexHome $homeDir
    Assert ($report.OwnerSid -eq $originalOwner) 'Inspection owner mismatch'
    Assert (-not [string]::IsNullOrWhiteSpace($report.Identity)) 'Missing directory identity'
    & $tool -CodexHome $homeDir -Repair -BackupPath $ownerBackup -WhatIf | Out-Null
    Assert (-not (Test-Path -LiteralPath $ownerBackup)) 'WhatIf created a backup'
    Assert ((Get-Acl -LiteralPath $bin).Sddl -eq $original.Sddl) 'WhatIf mutated ACL'

    $report = & $tool -CodexHome $homeDir -Repair -BackupPath $ownerBackup -Confirm:$false
    Assert ($report.OwnerSid -eq 'S-1-5-18') 'Repair did not set SYSTEM owner'
    Assert ((Get-Acl -LiteralPath $bin).GetSecurityDescriptorSddlForm($sections) -eq $originalDacl) 'Repair changed DACL'
    Assert ((Get-Acl -LiteralPath $child).Sddl -eq $childSddl) 'Repair changed child security'
    Assert ([IO.File]::ReadAllText($child) -eq 'preserve me') 'Repair changed child content'
    $saved = [IO.File]::ReadAllText($ownerBackup)
    & $tool -CodexHome $homeDir -Repair -BackupPath $ownerBackup -Confirm:$false | Out-Null
    Assert ([IO.File]::ReadAllText($ownerBackup) -eq $saved) 'Idempotent repair overwrote backup'

    # A backup for a different file identity must never be applied.
    $bad = ($saved | ConvertFrom-Json)
    $bad.identity = 'different-file-id'
    $wrongBackup = Join-Path $root 'wrong.json'
    $bad | ConvertTo-Json | Set-Content -LiteralPath $wrongBackup -Encoding UTF8
    $rejected = $false
    try { & $tool -CodexHome $homeDir -RestoreBackup $wrongBackup -Confirm:$false | Out-Null } catch { $rejected = $true }
    Assert $rejected 'Accepted mismatched directory identity'
    & $tool -CodexHome $homeDir -RestoreBackup $ownerBackup -Confirm:$false | Out-Null
    Assert ((Get-Acl -LiteralPath $bin).Sddl -eq $original.Sddl) 'Rollback did not preserve original security'

    # Backup collision must fail before changing ownership.
    $rejected = $false
    try { & $tool -CodexHome $homeDir -Repair -BackupPath $ownerBackup -Confirm:$false | Out-Null } catch { $rejected = $true }
    Assert $rejected 'Overwrote an existing backup'
    Assert ((Get-Acl -LiteralPath $bin).Sddl -eq $original.Sddl) 'Failed backup changed ownership'

    # Junctions anywhere in the path must be rejected by the native open.
    $link = Join-Path $root 'junction'
    New-Item -ItemType Junction -Path $link -Target $homeDir | Out-Null
    $rejected = $false
    try { & $tool -CodexHome $link | Out-Null } catch { $rejected = $true }
    Assert $rejected 'Followed a junction'
    [IO.Directory]::Delete($link)
    $linkedHome = Join-Path $root 'linked-bin-home'
    New-Item -ItemType Directory -Path $linkedHome | Out-Null
    $linkedBin = Join-Path $linkedHome '.sandbox-bin'
    New-Item -ItemType Junction -Path $linkedBin -Target $bin | Out-Null
    try {
        $rejected = $false
        try { & $tool -CodexHome $linkedHome | Out-Null } catch { $rejected = $true }
        Assert $rejected 'Followed final directory junction'
    } finally { [IO.Directory]::Delete($linkedBin) }
    $rejected = $false
    try { & $tool -CodexHome ($homeDir + '\..\' + (Split-Path $homeDir -Leaf)) | Out-Null } catch { $rejected = $true }
    Assert $rejected 'Accepted relative segments'
    Write-Output 'PASS: inspection, WhatIf, repair, DACL/child preservation, idempotence, rollback, backup collision, identity mismatch, junction and traversal rejection'
} finally {
    # Delete only this freshly allocated fixture directory; never follow the junction.
    $resolved = [IO.Path]::GetFullPath($root)
    if ($resolved.StartsWith([IO.Path]::GetFullPath([IO.Path]::GetTempPath()), [StringComparison]::OrdinalIgnoreCase) -and (Split-Path $resolved -Leaf) -match '^cockpit-sandbox-test-[0-9a-f]{32}$') {
        $link = Join-Path $resolved 'junction'
        if (Test-Path -LiteralPath $link) { [IO.Directory]::Delete($link) }
        Remove-Item -LiteralPath $resolved -Recurse -Force
    }
}
