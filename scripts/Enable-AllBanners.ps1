[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$DatabasePath,

    [Parameter(Mandatory = $true)]
    [string]$SummonPoolJson,

    [int[]]$IncludePoolId,

    [switch]$Apply,

    [switch]$SkipBackup
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$database = (Resolve-Path -LiteralPath $DatabasePath).Path
$summonPool = (Resolve-Path -LiteralPath $SummonPoolJson).Path
$sqlite = (Get-Command sqlite3.exe -ErrorAction Stop).Source

$json = [System.IO.File]::ReadAllText($summonPool)
$availablePoolIds = @(
    [regex]::Matches($json, '"id"\s*:\s*(\d+)') |
        ForEach-Object { [int]$_.Groups[1].Value } |
        Sort-Object -Unique
)

if ($availablePoolIds.Count -eq 0) {
    throw "No summon pool IDs found in $summonPool"
}

if ($PSBoundParameters.ContainsKey("IncludePoolId")) {
    $poolIds = @($IncludePoolId | Sort-Object -Unique)
    $missingPoolIds = @($poolIds | Where-Object { $availablePoolIds -notcontains $_ })
    if ($missingPoolIds.Count -gt 0) {
        throw "Requested summon pool IDs are missing from $summonPool`: $($missingPoolIds -join ',')"
    }
}
else {
    $poolIds = $availablePoolIds
}

Write-Output "Selected $($poolIds.Count) of $($availablePoolIds.Count) unique summon pool IDs"
if (-not $Apply) {
    Write-Output "Dry run only; rerun with -Apply to update $database"
    return
}

$databaseDirectory = Split-Path -Parent $database
$databaseName = Split-Path -Leaf $database

function Invoke-Sqlite {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Statement
    )

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

$schemaCountOutput = Invoke-Sqlite -Statement @"
SELECT COUNT(*)
FROM sqlite_master
WHERE type = 'table'
  AND name IN ('banner_schedule', 'user_summon_pools');
"@
$schemaCount = [int]($schemaCountOutput | Select-Object -Last 1)
if ($schemaCount -ne 2) {
    throw "Expected banner_schedule and user_summon_pools tables in $database"
}

$backupPath = $null
if (-not $SkipBackup) {
    $timestamp = [DateTime]::UtcNow.ToString("yyyyMMddTHHmmssZ")
    $backupName = "$databaseName.banner-open.$timestamp.bak"
    $backupPath = Join-Path $databaseDirectory $backupName
    $escapedBackupName = $backupName.Replace("'", "''")
    Invoke-Sqlite -Statement ".backup '$escapedBackupName'" | Out-Null

    if (-not (Test-Path -LiteralPath $backupPath -PathType Leaf)) {
        throw "SQLite backup was not created: $backupPath"
    }

    Write-Output "Backup created: $backupPath"
}

$values = ($poolIds | ForEach-Object { "($_)" }) -join ","
$now = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
$sql = @"
BEGIN IMMEDIATE;
CREATE TEMP TABLE desired_banner_schedule(pool_id INTEGER PRIMARY KEY);
INSERT INTO desired_banner_schedule(pool_id) VALUES $values;
DELETE FROM banner_schedule
WHERE pool_id NOT IN (SELECT pool_id FROM desired_banner_schedule);
INSERT INTO banner_schedule(pool_id, online_time, offline_time, created_at, updated_at)
SELECT pool_id, 0, 2147483647, $now, $now
FROM desired_banner_schedule
WHERE 1
ON CONFLICT(pool_id) DO UPDATE SET
    online_time = excluded.online_time,
    offline_time = excluded.offline_time,
    updated_at = excluded.updated_at;
UPDATE user_summon_pools
SET online_time = 0,
    offline_time = 2147483647,
    updated_at = $now
WHERE pool_id IN (SELECT pool_id FROM desired_banner_schedule);
COMMIT;
"@

Invoke-Sqlite -Statement $sql | Out-Null

$verification = Invoke-Sqlite -Statement @"
SELECT COUNT(*) || '|' ||
       COALESCE(SUM(CASE
           WHEN online_time <= strftime('%s','now')
            AND offline_time > strftime('%s','now')
           THEN 1 ELSE 0 END), 0)
FROM banner_schedule;
SELECT GROUP_CONCAT(pool_id, ',')
FROM (SELECT pool_id FROM banner_schedule ORDER BY pool_id);
"@

if ($verification.Count -ne 2) {
    throw "Unexpected verification output: $($verification -join [Environment]::NewLine)"
}

$counts = $verification[0].ToString().Split('|')
$scheduled = [int]$counts[0]
$active = [int]$counts[1]
$expectedIds = $poolIds -join ','
$actualIds = $verification[1].ToString()

if ($scheduled -ne $poolIds.Count -or $active -ne $poolIds.Count) {
    throw "Expected $($poolIds.Count) scheduled and active pools, got $scheduled scheduled and $active active"
}

if ($actualIds -ne $expectedIds) {
    throw "banner_schedule pool IDs do not exactly match summon_pool.json"
}

Write-Output "Verified $scheduled scheduled and $active active summon pools"
