[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$DatabasePath,

    [Parameter(Mandatory = $true)]
    [string]$SummonPoolJson,

    [switch]$Apply,

    [switch]$SkipBackup
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$replayEngine = Join-Path $PSScriptRoot "Enable-AllBanners.ps1"
if (-not (Test-Path -LiteralPath $replayEngine -PathType Leaf)) {
    throw "Replay engine not found: $replayEngine"
}

$currentNonReprintPoolIds = @(
    1,
    2,
    34111,
    34121,
    34131,
    34141,
    34151,
    34161,
    34191
)

& $replayEngine `
    -DatabasePath $DatabasePath `
    -SummonPoolJson $SummonPoolJson `
    -IncludePoolId $currentNonReprintPoolIds `
    -Apply:$Apply `
    -SkipBackup:$SkipBackup
