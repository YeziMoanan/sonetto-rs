[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$PresetPath,

    [Parameter(Mandatory = $true)]
    [string]$DatabasePath,

    [Parameter(Mandatory = $true)]
    [string]$SummonPoolJson,

    [switch]$Apply,

    [switch]$SkipBackup
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$presetFile = (Resolve-Path -LiteralPath $PresetPath).Path
$database = (Resolve-Path -LiteralPath $DatabasePath).Path
$summonPoolFile = (Resolve-Path -LiteralPath $SummonPoolJson).Path
$sqlite = (Get-Command sqlite3.exe -ErrorAction Stop).Source
$strictUtf8 = [System.Text.UTF8Encoding]::new($false, $true)
$databaseDirectory = Split-Path -Parent $database
$databaseName = Split-Path -Leaf $database

function Read-StrictUtf8Json {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,

        [Parameter(Mandatory = $true)]
        [string]$Label
    )

    try {
        $text = [System.IO.File]::ReadAllText($Path, $strictUtf8)
        $value = $text | ConvertFrom-Json
        Write-Output -NoEnumerate $value
    }
    catch {
        throw "$Label is not valid UTF-8 JSON: $($_.Exception.Message)"
    }
}

function Get-RequiredProperty {
    param(
        [Parameter(Mandatory = $true)]
        $Object,

        [Parameter(Mandatory = $true)]
        [string]$Name,

        [Parameter(Mandatory = $true)]
        [string]$Context
    )

    if ($null -eq $Object -or $Object.PSObject.Properties.Name -notcontains $Name) {
        throw "$Context is missing property '$Name'"
    }
    return $Object.$Name
}

function Convert-RequiredInt64 {
    param(
        [Parameter(Mandatory = $true)]
        $Value,

        [Parameter(Mandatory = $true)]
        [string]$Label,

        [long]$Minimum = [long]::MinValue,

        [long]$Maximum = [long]::MaxValue
    )

    if ($null -eq $Value) {
        throw "$Label must be an integer"
    }
    $typeCode = [System.Type]::GetTypeCode($Value.GetType())
    $integerTypes = @(
        [System.TypeCode]::SByte,
        [System.TypeCode]::Byte,
        [System.TypeCode]::Int16,
        [System.TypeCode]::UInt16,
        [System.TypeCode]::Int32,
        [System.TypeCode]::UInt32,
        [System.TypeCode]::Int64
    )
    if ($integerTypes -notcontains $typeCode) {
        throw "$Label must be an integer"
    }
    $integer = [long]$Value
    if ($integer -lt $Minimum -or $integer -gt $Maximum) {
        throw "$Label must be between $Minimum and $Maximum"
    }
    return $integer
}

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

function Format-ShanghaiTime {
    param(
        [Parameter(Mandatory = $true)]
        [long]$Timestamp
    )

    $offset = [TimeSpan]::FromHours(8)
    return [DateTimeOffset]::FromUnixTimeSeconds($Timestamp).ToOffset($offset).ToString("yyyy-MM-dd HH:mm:ss zzz")
}

$source = Read-StrictUtf8Json -Path $summonPoolFile -Label "Summon pool source"
if (
    $source -isnot [System.Array] -or
    $source.Count -ne 2 -or
    $source[0] -ne "summon_pool" -or
    $source[1] -isnot [System.Array]
) {
    throw "Summon pool source must be a summon_pool tuple"
}

$availablePoolIds = [System.Collections.Generic.HashSet[long]]::new()
foreach ($sourcePool in @($source[1])) {
    $sourceIdValue = Get-RequiredProperty -Object $sourcePool -Name "id" -Context "Summon pool"
    $sourceId = Convert-RequiredInt64 -Value $sourceIdValue -Label "Summon pool ID" -Minimum 1
    if (-not $availablePoolIds.Add($sourceId)) {
        throw "Duplicate summon pool ID in source: $sourceId"
    }
}
if ($availablePoolIds.Count -eq 0) {
    throw "Summon pool source contains no pools"
}

$preset = Read-StrictUtf8Json -Path $presetFile -Label "Preset"
$schemaVersionValue = Get-RequiredProperty -Object $preset -Name "schemaVersion" -Context "Preset"
$schemaVersion = Convert-RequiredInt64 -Value $schemaVersionValue -Label "Preset schema version"
if ($schemaVersion -ne 1) {
    throw "Unsupported preset schema version: $schemaVersion"
}

$presetName = [string](Get-RequiredProperty -Object $preset -Name "presetName" -Context "Preset")
if ([string]::IsNullOrWhiteSpace($presetName)) {
    throw "Preset name is required"
}
$generatedAt = [string](Get-RequiredProperty -Object $preset -Name "generatedAt" -Context "Preset")
try {
    [DateTimeOffset]::Parse($generatedAt) | Out-Null
}
catch {
    throw "Preset generatedAt is not a valid timestamp"
}

$sourceMetadata = Get-RequiredProperty -Object $preset -Name "source" -Context "Preset"
$sourcePoolCountValue = Get-RequiredProperty -Object $sourceMetadata -Name "poolCount" -Context "Preset source"
$sourcePoolCount = Convert-RequiredInt64 -Value $sourcePoolCountValue -Label "Preset source poolCount" -Minimum 1
if ($sourcePoolCount -ne $availablePoolIds.Count) {
    throw "Preset source poolCount $sourcePoolCount does not match current source count $($availablePoolIds.Count)"
}

$schedule = Get-RequiredProperty -Object $preset -Name "schedule" -Context "Preset"
$mode = [string](Get-RequiredProperty -Object $schedule -Name "mode" -Context "Preset schedule")
if (@("simultaneous", "batch", "manual") -notcontains $mode) {
    throw "Unsupported preset schedule mode: $mode"
}
$timezone = [string](Get-RequiredProperty -Object $schedule -Name "timezone" -Context "Preset schedule")
if ($timezone -ne "Asia/Shanghai") {
    throw "Preset timezone must be Asia/Shanghai"
}

$presetPoolsValue = Get-RequiredProperty -Object $preset -Name "pools" -Context "Preset"
$presetPools = @($presetPoolsValue)
if ($presetPools.Count -eq 0) {
    throw "Preset must contain at least one pool"
}

$seenPresetPoolIds = [System.Collections.Generic.HashSet[long]]::new()
$normalizedPools = @()
for ($index = 0; $index -lt $presetPools.Count; $index++) {
    $presetPool = $presetPools[$index]
    $poolIdValue = Get-RequiredProperty -Object $presetPool -Name "poolId" -Context "Preset pool at index $index"
    $poolId = Convert-RequiredInt64 -Value $poolIdValue -Label "Preset pool ID" -Minimum 1
    if (-not $seenPresetPoolIds.Add($poolId)) {
        throw "Duplicate preset pool ID: $poolId"
    }
    if (-not $availablePoolIds.Contains($poolId)) {
        throw "Unknown preset pool ID in current summon_pool.json: $poolId"
    }

    $orderValue = Get-RequiredProperty -Object $presetPool -Name "order" -Context "Preset pool $poolId"
    $order = Convert-RequiredInt64 -Value $orderValue -Label "Preset pool order" -Minimum 0
    if ($order -ne $index) {
        throw "Preset pool order must be contiguous; expected $index, got $order"
    }

    $onlineTimeValue = Get-RequiredProperty -Object $presetPool -Name "onlineTime" -Context "Preset pool $poolId"
    $onlineTime = Convert-RequiredInt64 -Value $onlineTimeValue -Label "Pool $poolId onlineTime" -Minimum 0 -Maximum 2147483647
    $offlineTimeValue = Get-RequiredProperty -Object $presetPool -Name "offlineTime" -Context "Preset pool $poolId"
    $offlineTime = Convert-RequiredInt64 -Value $offlineTimeValue -Label "Pool $poolId offlineTime" -Minimum 0 -Maximum 2147483647
    if ($offlineTime -le $onlineTime) {
        throw "Pool $poolId offlineTime must be later than onlineTime"
    }

    $normalizedPools += [pscustomobject]@{
        PoolId = $poolId
        Order = $order
        OnlineTime = $onlineTime
        OfflineTime = $offlineTime
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
    throw "Database must contain banner_schedule and user_summon_pools tables"
}

Write-Output "Validated preset '$presetName' with $($normalizedPools.Count) pools"
Write-Output "pool_id | online_time | offline_time | Shanghai interval"
foreach ($pool in $normalizedPools) {
    $onlineDisplay = Format-ShanghaiTime -Timestamp $pool.OnlineTime
    $offlineDisplay = Format-ShanghaiTime -Timestamp $pool.OfflineTime
    Write-Output "$($pool.PoolId) | $($pool.OnlineTime) | $($pool.OfflineTime) | $onlineDisplay -> $offlineDisplay"
}

if (-not $Apply) {
    Write-Output "Dry run only; no backup or database write was performed"
    return
}

if (-not $SkipBackup) {
    $timestamp = [DateTime]::UtcNow.ToString("yyyyMMddTHHmmssfffZ")
    $backupName = "$databaseName.banner-preset.$timestamp.bak"
    $backupPath = Join-Path $databaseDirectory $backupName
    $escapedBackupName = $backupName.Replace("'", "''")
    Invoke-Sqlite -Statement ".backup '$escapedBackupName'" | Out-Null
    if (-not (Test-Path -LiteralPath $backupPath -PathType Leaf)) {
        throw "SQLite backup was not created: $backupPath"
    }
    if ((Get-Item -LiteralPath $backupPath).Length -le 0) {
        throw "SQLite backup is empty: $backupPath"
    }
    Write-Output "Backup created: $backupPath"
}

$values = ($normalizedPools | ForEach-Object {
    "($($_.PoolId),$($_.OnlineTime),$($_.OfflineTime))"
}) -join ","
$now = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
$transaction = @"
BEGIN IMMEDIATE;
CREATE TEMP TABLE desired_banner_schedule(
    pool_id INTEGER PRIMARY KEY,
    online_time INTEGER NOT NULL,
    offline_time INTEGER NOT NULL
);
INSERT INTO desired_banner_schedule(pool_id, online_time, offline_time) VALUES $values;
DELETE FROM banner_schedule
WHERE pool_id NOT IN (SELECT pool_id FROM desired_banner_schedule);
INSERT INTO banner_schedule(pool_id, online_time, offline_time, created_at, updated_at)
SELECT pool_id, online_time, offline_time, $now, $now
FROM desired_banner_schedule
WHERE 1
ON CONFLICT(pool_id) DO UPDATE SET
    online_time = excluded.online_time,
    offline_time = excluded.offline_time,
    updated_at = excluded.updated_at;
UPDATE user_summon_pools
SET online_time = (
        SELECT desired.online_time
        FROM desired_banner_schedule AS desired
        WHERE desired.pool_id = user_summon_pools.pool_id
    ),
    offline_time = (
        SELECT desired.offline_time
        FROM desired_banner_schedule AS desired
        WHERE desired.pool_id = user_summon_pools.pool_id
    ),
    updated_at = $now
WHERE pool_id IN (SELECT pool_id FROM desired_banner_schedule);
COMMIT;
"@

Invoke-Sqlite -Statement $transaction | Out-Null

$verificationRows = @(Invoke-Sqlite -Statement @"
SELECT pool_id || '|' || online_time || '|' || offline_time
FROM banner_schedule
ORDER BY pool_id;
"@)
$expectedRows = @($normalizedPools | Sort-Object PoolId | ForEach-Object {
    "$($_.PoolId)|$($_.OnlineTime)|$($_.OfflineTime)"
})

if ($verificationRows.Count -ne $expectedRows.Count) {
    throw "Schedule verification count mismatch: expected $($expectedRows.Count), got $($verificationRows.Count)"
}
for ($index = 0; $index -lt $expectedRows.Count; $index++) {
    if ($verificationRows[$index].ToString() -ne $expectedRows[$index]) {
        throw "Schedule verification mismatch at row $index`: expected '$($expectedRows[$index])', got '$($verificationRows[$index])'"
    }
}

Write-Output "Verified exact schedule for $($expectedRows.Count) pools"
