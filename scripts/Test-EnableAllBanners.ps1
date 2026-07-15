[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$SourceDatabase,

    [Parameter(Mandatory = $true)]
    [string]$SummonPoolJson
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$replayScript = Join-Path $PSScriptRoot "Enable-AllBanners.ps1"
if (-not (Test-Path -LiteralPath $replayScript -PathType Leaf)) {
    throw "Replay script not found: $replayScript"
}

$databasePath = (Resolve-Path -LiteralPath $SourceDatabase).Path
$summonPoolPath = (Resolve-Path -LiteralPath $SummonPoolJson).Path
$sqlite = (Get-Command sqlite3.exe -ErrorAction Stop).Source

$json = [System.IO.File]::ReadAllText($summonPoolPath)
$expectedPoolIds = @(
    [regex]::Matches($json, '"id"\s*:\s*(\d+)') |
        ForEach-Object { [int]$_.Groups[1].Value } |
        Sort-Object -Unique
)

if ($expectedPoolIds.Count -eq 0) {
    throw "No summon pool IDs found in $summonPoolPath"
}

$tempDirectory = Join-Path ([System.IO.Path]::GetTempPath()) ("sonetto-banner-test-" + [guid]::NewGuid().ToString("N"))
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

function Assert-ScheduleCounts {
    $output = Invoke-Sqlite -Database $tempDatabase -Statement @"
SELECT COUNT(*),
       COALESCE(SUM(CASE
           WHEN online_time <= strftime('%s','now')
            AND offline_time > strftime('%s','now')
           THEN 1 ELSE 0 END), 0)
FROM banner_schedule;
"@

    $parts = ($output | Select-Object -Last 1).ToString().Split('|')
    if ($parts.Count -ne 2) {
        throw "Unexpected verification output: $($output -join [Environment]::NewLine)"
    }

    $scheduled = [int]$parts[0]
    $active = [int]$parts[1]
    if ($scheduled -ne $expectedPoolIds.Count -or $active -ne $expectedPoolIds.Count) {
        throw "Expected $($expectedPoolIds.Count) scheduled and active pools, got $scheduled scheduled and $active active"
    }
}

try {
    $escapedTempDatabase = $tempDatabase.Replace("'", "''")
    Invoke-Sqlite -Database $databasePath -Statement ".backup '$escapedTempDatabase'" | Out-Null

    & $replayScript -DatabasePath $tempDatabase -SummonPoolJson $summonPoolPath -Apply -SkipBackup
    Assert-ScheduleCounts

    & $replayScript -DatabasePath $tempDatabase -SummonPoolJson $summonPoolPath -Apply -SkipBackup
    Assert-ScheduleCounts

    Write-Output "PASS: replay is idempotent with $($expectedPoolIds.Count) scheduled and active pools"
}
finally {
    if (Test-Path -LiteralPath $tempDirectory) {
        Remove-Item -LiteralPath $tempDirectory -Recurse -Force
    }
}
