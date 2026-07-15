[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$executor = Join-Path (Split-Path -Parent $PSScriptRoot) "Apply-BannerPreset.ps1"
$sqlite = (Get-Command sqlite3.exe -ErrorAction Stop).Source
$tempDirectory = Join-Path ([System.IO.Path]::GetTempPath()) ("banner-scheduler-test-" + [Guid]::NewGuid().ToString("N"))
$utf8 = [System.Text.UTF8Encoding]::new($false)

function Write-Utf8File {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,

        [Parameter(Mandatory = $true)]
        [string]$Text
    )

    [System.IO.File]::WriteAllText($Path, $Text, $utf8)
}

function Invoke-Sqlite {
    param(
        [Parameter(Mandatory = $true)]
        [string]$DatabasePath,

        [Parameter(Mandatory = $true)]
        [string]$Statement
    )

    $output = & $sqlite $DatabasePath $Statement 2>&1
    if ($LASTEXITCODE -ne 0) {
        throw "sqlite3 failed: $($output -join [Environment]::NewLine)"
    }
    return @($output)
}

function Assert-Equal {
    param(
        [Parameter(Mandatory = $true)]
        $Expected,

        [Parameter(Mandatory = $true)]
        $Actual,

        [Parameter(Mandatory = $true)]
        [string]$Message
    )

    if ($Expected -ne $Actual) {
        throw "$Message. Expected '$Expected', got '$Actual'"
    }
}

function Assert-PresetFails {
    param(
        [Parameter(Mandatory = $true)]
        $Preset,

        [Parameter(Mandatory = $true)]
        [string]$Pattern,

        [Parameter(Mandatory = $true)]
        [string]$Name,

        [Parameter(Mandatory = $true)]
        [string]$DatabasePath,

        [Parameter(Mandatory = $true)]
        [string]$SourcePath
    )

    $invalidPath = Join-Path $tempDirectory "$Name.json"
    Write-Utf8File -Path $invalidPath -Text ($Preset | ConvertTo-Json -Depth 10)
    try {
        & $executor `
            -PresetPath $invalidPath `
            -DatabasePath $DatabasePath `
            -SummonPoolJson $SourcePath `
            -SkipBackup | Out-Null
        throw "Expected $Name to fail"
    }
    catch {
        if ($_.Exception.Message -notmatch $Pattern) {
            throw "$Name failed with unexpected message: $($_.Exception.Message)"
        }
    }
}

New-Item -ItemType Directory -Path $tempDirectory | Out-Null

try {
    if (-not (Test-Path -LiteralPath $executor -PathType Leaf)) {
        throw "Executor not found: $executor"
    }

    $sourcePath = Join-Path $tempDirectory "summon_pool.json"
    $presetPath = Join-Path $tempDirectory "preset.json"
    $databasePath = Join-Path $tempDirectory "sonetto.db"

    Write-Utf8File -Path $sourcePath -Text @'
[
  "summon_pool",
  [
    { "id": 20, "nameEn": "Activity", "bannerFlag": 2, "type": 3, "priority": 20 },
    { "id": 40, "nameEn": "Rerun", "bannerFlag": 4, "type": 4, "priority": 40 },
    { "id": 50, "nameEn": "Collaboration", "bannerFlag": 5, "type": 5, "priority": 50 }
  ]
]
'@

    $preset = [pscustomobject]@{
        schemaVersion = 1
        presetName = "Fixture rotation"
        generatedAt = "2026-07-15T03:00:00.000Z"
        source = [pscustomobject]@{
            fileName = "summon_pool.json"
            poolCount = 3
        }
        schedule = [pscustomobject]@{
            mode = "batch"
            timezone = "Asia/Shanghai"
            startDate = "2026-07-15"
            daysPerBatch = 7
            poolsPerBatch = 1
        }
        pools = @(
            [pscustomobject]@{ poolId = 50; order = 0; onlineTime = 1784044800; offlineTime = 1784649600 },
            [pscustomobject]@{ poolId = 40; order = 1; onlineTime = 1784649600; offlineTime = 1785254400 }
        )
    }
    Write-Utf8File -Path $presetPath -Text ($preset | ConvertTo-Json -Depth 10)

    Invoke-Sqlite -DatabasePath $databasePath -Statement @"
CREATE TABLE banner_schedule (
    pool_id INTEGER PRIMARY KEY,
    online_time INTEGER NOT NULL,
    offline_time INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE user_summon_pools (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL,
    pool_id INTEGER NOT NULL,
    online_time INTEGER NOT NULL DEFAULT 0,
    offline_time INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE(user_id, pool_id)
);
INSERT INTO banner_schedule VALUES (20, 1, 2, 1, 1), (50, 1, 2, 1, 1);
INSERT INTO user_summon_pools(user_id, pool_id, online_time, offline_time, created_at, updated_at)
VALUES (1, 20, 1, 2, 1, 1), (1, 50, 1, 2, 1, 1);
"@ | Out-Null

    $beforeDryRunHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $databasePath).Hash
    $dryRunOutput = & $executor `
        -PresetPath $presetPath `
        -DatabasePath $databasePath `
        -SummonPoolJson $sourcePath `
        -SkipBackup
    $afterDryRunHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $databasePath).Hash
    Assert-Equal -Expected $beforeDryRunHash -Actual $afterDryRunHash -Message "Dry run changed the database"
    if (($dryRunOutput -join "`n") -notmatch "Dry run") {
        throw "Dry run output did not identify itself"
    }

    & $executor `
        -PresetPath $presetPath `
        -DatabasePath $databasePath `
        -SummonPoolJson $sourcePath `
        -Apply `
        -SkipBackup | Out-Null

    $expectedSchedule = "40|1784649600|1785254400`n50|1784044800|1784649600"
    $firstSchedule = (Invoke-Sqlite -DatabasePath $databasePath -Statement @"
SELECT pool_id || '|' || online_time || '|' || offline_time
FROM banner_schedule
ORDER BY pool_id;
"@) -join "`n"
    Assert-Equal -Expected $expectedSchedule -Actual $firstSchedule -Message "Applied schedule was not exact"

    $syncedUserPool = (Invoke-Sqlite -DatabasePath $databasePath -Statement @"
SELECT online_time || '|' || offline_time
FROM user_summon_pools
WHERE user_id = 1 AND pool_id = 50;
"@) -join "`n"
    Assert-Equal -Expected "1784044800|1784649600" -Actual $syncedUserPool -Message "Existing user pool was not synchronized"

    & $executor `
        -PresetPath $presetPath `
        -DatabasePath $databasePath `
        -SummonPoolJson $sourcePath `
        -Apply `
        -SkipBackup | Out-Null
    $secondSchedule = (Invoke-Sqlite -DatabasePath $databasePath -Statement @"
SELECT pool_id || '|' || online_time || '|' || offline_time
FROM banner_schedule
ORDER BY pool_id;
"@) -join "`n"
    Assert-Equal -Expected $firstSchedule -Actual $secondSchedule -Message "Second apply was not idempotent"

    $backupCountBefore = @(Get-ChildItem -LiteralPath $tempDirectory -Filter "sonetto.db.banner-preset.*.bak").Count
    & $executor `
        -PresetPath $presetPath `
        -DatabasePath $databasePath `
        -SummonPoolJson $sourcePath `
        -Apply | Out-Null
    $backups = @(Get-ChildItem -LiteralPath $tempDirectory -Filter "sonetto.db.banner-preset.*.bak")
    Assert-Equal -Expected ($backupCountBefore + 1) -Actual $backups.Count -Message "Apply did not create one backup"
    if ($backups[-1].Length -le 0) {
        throw "Backup file was empty"
    }

    $unknown = $preset | ConvertTo-Json -Depth 10 | ConvertFrom-Json
    $unknown.pools[1].poolId = 999
    Assert-PresetFails -Preset $unknown -Pattern "unknown|missing" -Name "unknown-pool" -DatabasePath $databasePath -SourcePath $sourcePath

    $duplicate = $preset | ConvertTo-Json -Depth 10 | ConvertFrom-Json
    $duplicate.pools[1].poolId = 50
    Assert-PresetFails -Preset $duplicate -Pattern "duplicate" -Name "duplicate-pool" -DatabasePath $databasePath -SourcePath $sourcePath

    $badOrder = $preset | ConvertTo-Json -Depth 10 | ConvertFrom-Json
    $badOrder.pools[1].order = 3
    Assert-PresetFails -Preset $badOrder -Pattern "order" -Name "bad-order" -DatabasePath $databasePath -SourcePath $sourcePath

    $badInterval = $preset | ConvertTo-Json -Depth 10 | ConvertFrom-Json
    $badInterval.pools[0].offlineTime = $badInterval.pools[0].onlineTime
    Assert-PresetFails -Preset $badInterval -Pattern "later|offline" -Name "bad-interval" -DatabasePath $databasePath -SourcePath $sourcePath

    $missingTableDatabase = Join-Path $tempDirectory "missing-table.db"
    Invoke-Sqlite -DatabasePath $missingTableDatabase -Statement "CREATE TABLE placeholder(id INTEGER);" | Out-Null
    try {
        & $executor `
            -PresetPath $presetPath `
            -DatabasePath $missingTableDatabase `
            -SummonPoolJson $sourcePath `
            -SkipBackup | Out-Null
        throw "Expected missing table database to fail"
    }
    catch {
        if ($_.Exception.Message -notmatch "banner_schedule|user_summon_pools|table") {
            throw "Missing table test failed with unexpected message: $($_.Exception.Message)"
        }
    }

    Write-Output "PASS: dry run, exact apply, user sync, idempotence, backup, and validation"
}
finally {
    if (Test-Path -LiteralPath $tempDirectory) {
        Remove-Item -LiteralPath $tempDirectory -Recurse -Force
    }
}
