[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$SourceDatabase,

    [Parameter(Mandatory = $true)]
    [string]$SummonPoolJson
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$replayScript = Join-Path $PSScriptRoot "Enable-CurrentNonReprintBanners.ps1"
if (-not (Test-Path -LiteralPath $replayScript -PathType Leaf)) {
    throw "Replay script not found: $replayScript"
}

$databasePath = (Resolve-Path -LiteralPath $SourceDatabase).Path
$summonPoolPath = (Resolve-Path -LiteralPath $SummonPoolJson).Path
$sqlite = (Get-Command sqlite3.exe -ErrorAction Stop).Source
$expectedPoolIds = @(1, 2, 34111, 34121, 34131, 34141, 34151, 34161, 34191)
$expectedPoolList = $expectedPoolIds -join ','

$tempDirectory = Join-Path ([System.IO.Path]::GetTempPath()) ("sonetto-current-banner-test-" + [guid]::NewGuid().ToString("N"))
$null = New-Item -ItemType Directory -Path $tempDirectory
$tempDatabase = Join-Path $tempDirectory "sonetto-test.db"

function Invoke-Sqlite {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Database,

        [Parameter(Mandatory = $true)]
        [string]$Statement
    )

    $databaseDirectory = Split-Path -Parent $Database
    $databaseName = Split-Path -Leaf $Database

    Push-Location $databaseDirectory
    try {
        $output = & $sqlite $databaseName $Statement 2>&1
        if ($LASTEXITCODE -ne 0) {
            throw "sqlite3 failed: $($output -join [Environment]::NewLine)"
        }
        return @($output)
    }
    finally {
        Pop-Location
    }
}

function Assert-ExactSchedule {
    $output = Invoke-Sqlite -Database $tempDatabase -Statement @"
SELECT COUNT(*) || '|' ||
       COALESCE(SUM(CASE
           WHEN online_time <= strftime('%s','now')
            AND offline_time > strftime('%s','now')
           THEN 1 ELSE 0 END), 0)
FROM banner_schedule;
SELECT GROUP_CONCAT(pool_id, ',')
FROM (SELECT pool_id FROM banner_schedule ORDER BY pool_id);
"@

    if ($output.Count -ne 2) {
        throw "Unexpected verification output: $($output -join [Environment]::NewLine)"
    }

    if ($output[0].ToString() -ne "9|9") {
        throw "Expected 9 scheduled and active pools, got $($output[0])"
    }

    if ($output[1].ToString() -ne $expectedPoolList) {
        throw "Expected pool IDs $expectedPoolList, got $($output[1])"
    }
}

try {
    $escapedTempDatabase = $tempDatabase.Replace("'", "''")
    Invoke-Sqlite -Database $databasePath -Statement ".backup '$escapedTempDatabase'" | Out-Null

    & $replayScript -DatabasePath $tempDatabase -SummonPoolJson $summonPoolPath -Apply -SkipBackup
    Assert-ExactSchedule

    & $replayScript -DatabasePath $tempDatabase -SummonPoolJson $summonPoolPath -Apply -SkipBackup
    Assert-ExactSchedule

    Write-Output "PASS: current non-reprint replay is idempotent with exact pool IDs $expectedPoolList"
}
finally {
    if (Test-Path -LiteralPath $tempDirectory) {
        Remove-Item -LiteralPath $tempDirectory -Recurse -Force
    }
}
