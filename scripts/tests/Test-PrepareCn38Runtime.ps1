[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$prepareScript = Join-Path (Split-Path -Parent $PSScriptRoot) "Prepare-Cn38Runtime.ps1"
$tempRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("sonetto-cn38-runtime-test-" + [guid]::NewGuid().ToString("N"))
$fakeRepo = Join-Path $tempRoot "repo"
$dataSource = Join-Path $tempRoot "data-source"
$runtimeRoot = Join-Path $fakeRepo "runtime"

function Assert-True {
    param(
        [Parameter(Mandatory = $true)][bool]$Condition,
        [Parameter(Mandatory = $true)][string]$Message
    )
    if (-not $Condition) { throw $Message }
}

try {
    if (-not (Test-Path -LiteralPath $prepareScript -PathType Leaf)) {
        throw "Prepare script not found: $prepareScript"
    }

    $null = New-Item -ItemType Directory -Path (Join-Path $fakeRepo "common") -Force
    $null = New-Item -ItemType Directory -Path (Join-Path $fakeRepo "target\debug") -Force
    $null = New-Item -ItemType Directory -Path (Join-Path $dataSource "excel2json") -Force
    $null = New-Item -ItemType Directory -Path (Join-Path $dataSource "static") -Force

    [System.IO.File]::WriteAllText(
        (Join-Path $fakeRepo "common\Config.toml"),
        "[server]`nhost = `"127.0.0.1`"`ndns = `"localhost`"`nhttp_port = 21100`ngame_port = 23401`n[paths]`ndata_dir = `"./data`"`nexcel_data = `"./data/excel2json`"`nstatic_data = `"./data/static`"`n[database]`npath = `"./db/sonetto-3.8-cn.db`"`n"
    )
    [System.IO.File]::WriteAllText((Join-Path $fakeRepo "target\debug\sdkserver.exe"), "sdk")
    [System.IO.File]::WriteAllText((Join-Path $fakeRepo "target\debug\gameserver.exe"), "game")
    [System.IO.File]::WriteAllText((Join-Path $dataSource "excel2json\character.json"), "[]")
    [System.IO.File]::WriteAllText((Join-Path $dataSource "static\marker.txt"), "static")
    [System.IO.File]::WriteAllText((Join-Path $dataSource "sonetto.db"), "must-not-copy")

    $first = & $prepareScript -RepositoryRoot $fakeRepo -DataSource $dataSource -RuntimeRoot $runtimeRoot | ConvertFrom-Json
    $second = & $prepareScript -RepositoryRoot $fakeRepo -DataSource $dataSource -RuntimeRoot $runtimeRoot | ConvertFrom-Json

    Assert-True ($first.http_port -eq 21100) "Unexpected HTTP port"
    Assert-True ($first.game_port -eq 23401) "Unexpected game port"
    Assert-True (Test-Path -LiteralPath (Join-Path $runtimeRoot "config.toml") -PathType Leaf) "Config was not copied"
    Assert-True (Test-Path -LiteralPath (Join-Path $runtimeRoot "sdkserver.exe") -PathType Leaf) "SDK binary was not copied"
    Assert-True (Test-Path -LiteralPath (Join-Path $runtimeRoot "gameserver.exe") -PathType Leaf) "Game binary was not copied"
    Assert-True (Test-Path -LiteralPath (Join-Path $runtimeRoot "data\excel2json\character.json") -PathType Leaf) "Excel data was not copied"
    Assert-True (Test-Path -LiteralPath (Join-Path $runtimeRoot "data\static\marker.txt") -PathType Leaf) "Static data was not copied"
    Assert-True (-not (Test-Path -LiteralPath (Join-Path $runtimeRoot "sonetto.db"))) "Source database was copied"
    Assert-True (@(Get-ChildItem -LiteralPath (Join-Path $runtimeRoot "db") -File).Count -eq 0) "Runtime database directory is not empty"
    Assert-True ($second.runtime_root -eq $first.runtime_root) "Repeated preparation changed the runtime root"

    Write-Output "PASS: CN 3.8 runtime preparation is isolated and repeatable"
}
finally {
    if (Test-Path -LiteralPath $tempRoot) {
        Remove-Item -LiteralPath $tempRoot -Recurse -Force
    }
}
