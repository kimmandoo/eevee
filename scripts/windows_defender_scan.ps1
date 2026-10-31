#requires -Version 7.0
# Scan the exact release inputs without exclusions or automatic remediation.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Binary,
    [Parameter(Mandatory)][string]$Archive,
    [Parameter(Mandatory)][string]$Output
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
$reportDirectory = [IO.Path]::GetFullPath($Output)
New-Item -ItemType Directory -Force -Path $reportDirectory | Out-Null
$report = [ordered]@{
    status = 'failed'
    startedUtc = [DateTime]::UtcNow.ToString('o')
    files = @()
    error = $null
}
try {
    $inputs = @((Resolve-Path -LiteralPath $Binary).Path, (Resolve-Path -LiteralPath $Archive).Path)
    foreach ($file in $inputs) {
        if (-not (Test-Path -LiteralPath $file -PathType Leaf)) { throw "Not a file: $file" }
        $report.files += [ordered]@{
            path = $file
            sha256 = (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant()
            scanExitCode = $null
        }
    }
    # Hosted runners disable several protections by default. Only strengthen the
    # disposable CI machine; this script must never reconfigure a user's device.
    if ($env:GITHUB_ACTIONS -ne 'true') { throw 'Run release qualification only on a disposable GitHub Actions runner.' }
    Set-MpPreference -DisableRealtimeMonitoring $false -DisableBehaviorMonitoring $false `
        -DisableIOAVProtection $false -DisableArchiveScanning $false -DisableBlockAtFirstSeen $false `
        -PUAProtection Enabled -MAPSReporting Advanced -SubmitSamplesConsent SendSafeSamples
    Update-MpSignature -ErrorAction Stop
    $status = Get-MpComputerStatus
    if (-not $status.AMServiceEnabled -or -not $status.AntivirusEnabled -or
        $status.AMRunningMode -ne 'Normal' -or -not $status.RealTimeProtectionEnabled -or
        -not $status.BehaviorMonitorEnabled -or -not $status.IoavProtectionEnabled) {
        throw 'Microsoft Defender antivirus, real-time, behavior and downloaded-file protections must be active; no scan waiver is allowed.'
    }
    $report.engine = $status.AMEngineVersion
    $report.platform = $status.AMProductVersion
    $preferences = Get-MpPreference
    if ($preferences.PUAProtection -ne 1 -or $preferences.DisableArchiveScanning -or
        $preferences.MAPSReporting -eq 0 -or $preferences.DisableBlockAtFirstSeen) {
        throw 'Defender PUA, archive and cloud protections must be enabled.'
    }
    $report.puaProtection = [int]$preferences.PUAProtection
    $report.cloudProtection = [int]$preferences.MAPSReporting
    $report.realtimeProtection = $status.RealTimeProtectionEnabled
    $report.signatures = $status.AntivirusSignatureVersion
    $report.signaturesUpdatedUtc = $status.AntivirusSignatureLastUpdated.ToUniversalTime().ToString('o')
    $report.signatureStatus = [string](Get-AuthenticodeSignature -LiteralPath $inputs[0]).Status
    $platformRoot = Join-Path $env:ProgramData 'Microsoft/Windows Defender/Platform'
    $scanner = Join-Path $platformRoot ($status.AMProductVersion + '-0/MpCmdRun.exe')
    if (-not (Test-Path -LiteralPath $scanner -PathType Leaf)) {
        $scanner = Join-Path $env:ProgramFiles 'Windows Defender/MpCmdRun.exe'
    }
    if (-not (Test-Path -LiteralPath $scanner -PathType Leaf)) { throw 'Microsoft Defender scanner is unavailable.' }
    $report.scanner = $scanner
    foreach ($entry in $report.files) {
        $log = Join-Path $reportDirectory ((Split-Path $entry.path -Leaf) + '.scan.txt')
        # Custom non-remediating scans ignore exclusions and scan archives. A detection
        # must not be remediated into exit 0 and mistaken for a clean release.
        & $scanner -Scan -ScanType 3 -File $entry.path -DisableRemediation 2>&1 |
            Tee-Object -FilePath $log
        $entry.scanExitCode = $LASTEXITCODE
        if ($entry.scanExitCode -ne 0) {
            throw "Defender detected a threat or could not scan $($entry.path) (exit $($entry.scanExitCode)); see $log"
        }
        $after = (Get-FileHash -LiteralPath $entry.path -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($after -ne $entry.sha256) { throw "Release input changed during scanning: $($entry.path)" }
    }
    $report.status = 'passed'
} catch {
    $report.error = $_.Exception.Message
    throw
} finally {
    $report.finishedUtc = [DateTime]::UtcNow.ToString('o')
    $report | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $reportDirectory 'defender.json') -Encoding utf8
}
