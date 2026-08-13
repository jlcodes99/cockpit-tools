Set-StrictMode -Version Latest

function Get-AccountConversionMaintenanceReadiness {
    param(
        [Parameter(Mandatory = $true)][bool]$MaintenancePaused,
        [AllowNull()][object[]]$ActiveRunIds = @(),
        [AllowNull()][string]$ActiveManualSlot = $null,
        [AllowNull()][object[]]$Leases = @()
    )

    $normalizedActiveRunIds = @($ActiveRunIds | Where-Object { $null -ne $_ })
    $normalizedLeases = @($Leases | Where-Object { $null -ne $_ })
    $manualDetached = [string]::IsNullOrWhiteSpace($ActiveManualSlot)
    $ready = (
        $MaintenancePaused -and
        $normalizedActiveRunIds.Count -eq 0 -and
        $manualDetached -and
        $normalizedLeases.Count -eq 0
    )

    return [ordered]@{
        maintenanceReady = $ready
        maintenancePaused = $MaintenancePaused
        activeRunIds = $normalizedActiveRunIds
        activeManualSlot = if ($manualDetached) { $null } else { $ActiveManualSlot }
        leaseCount = $normalizedLeases.Count
        reason = if ($ready) {
            'The conversion run is durably paused; its account/challenge checkpoints are preserved while scheduler, manual worker, and lease are quiescent.'
        }
        else {
            'Pause the conversion run through its maintenance endpoint and wait until scheduler, manual worker, and lease are quiescent.'
        }
    }
}
