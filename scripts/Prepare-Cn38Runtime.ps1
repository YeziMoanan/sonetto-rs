[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$DataSource,

    [string]$RepositoryRoot = (Split-Path -Parent $PSScriptRoot),

    [string]$RuntimeRoot,

    [ValidateSet("debug", "release")]
    [string]$Profile = "debug"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Resolve-RequiredDirectory {
    param([Parameter(Mandatory = $true)][string]$Path)
    if (-not (Test-Path -LiteralPath $Path -PathType Container)) {
        throw "Required directory not found: $Path"
    }
    (Resolve-Path -LiteralPath $Path).Path
}

function Resolve-RequiredFile {
    param([Parameter(Mandatory = $true)][string]$Path)
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Required file not found: $Path"
    }
    (Resolve-Path -LiteralPath $Path).Path
}

function Copy-DirectoryContent {
    param(
        [Parameter(Mandatory = $true)][string]$Source,
        [Parameter(Mandatory = $true)][string]$Destination
    )
    $null = New-Item -ItemType Directory -Path $Destination -Force
    $output = & robocopy.exe $Source $Destination /E /COPY:DAT /DCOPY:DAT /R:1 /W:1 /NFL /NDL /NJH /NJS /NP
    $exitCode = $LASTEXITCODE
    if ($exitCode -gt 7) {
        throw "Robocopy failed with exit code $exitCode`: $($output -join [Environment]::NewLine)"
    }
}

$repo = Resolve-RequiredDirectory -Path $RepositoryRoot
$data = Resolve-RequiredDirectory -Path $DataSource
if ([string]::IsNullOrWhiteSpace($RuntimeRoot)) {
    $RuntimeRoot = Join-Path $repo "runtime"
}
$runtime = [System.IO.Path]::GetFullPath($RuntimeRoot)

$config = Resolve-RequiredFile -Path (Join-Path $repo "common\Config.toml")
$binaryDirectory = Resolve-RequiredDirectory -Path (Join-Path $repo "target\$Profile")
$sdkBinary = Resolve-RequiredFile -Path (Join-Path $binaryDirectory "sdkserver.exe")
$gameBinary = Resolve-RequiredFile -Path (Join-Path $binaryDirectory "gameserver.exe")
$excelSource = Resolve-RequiredDirectory -Path (Join-Path $data "excel2json")
$staticSource = Resolve-RequiredDirectory -Path (Join-Path $data "static")

$configText = [System.IO.File]::ReadAllText($config)
if ($configText -notmatch '(?m)^http_port\s*=\s*21100\s*$') {
    throw "Config does not declare HTTP port 21100"
}
if ($configText -notmatch '(?m)^game_port\s*=\s*23401\s*$') {
    throw "Config does not declare game port 23401"
}
if ($configText -notmatch '(?m)^path\s*=\s*"\./db/sonetto-3\.8-cn\.db"\s*$') {
    throw "Config does not declare the isolated CN 3.8 database"
}

$null = New-Item -ItemType Directory -Path $runtime -Force
$null = New-Item -ItemType Directory -Path (Join-Path $runtime "db") -Force
Copy-Item -LiteralPath $config -Destination (Join-Path $runtime "config.toml") -Force
Copy-Item -LiteralPath $sdkBinary -Destination (Join-Path $runtime "sdkserver.exe") -Force
Copy-Item -LiteralPath $gameBinary -Destination (Join-Path $runtime "gameserver.exe") -Force
Copy-DirectoryContent -Source $excelSource -Destination (Join-Path $runtime "data\excel2json")
Copy-DirectoryContent -Source $staticSource -Destination (Join-Path $runtime "data\static")

[ordered]@{
    runtime_root = $runtime
    profile = $Profile
    http_port = 21100
    game_port = 23401
    database_path = (Join-Path $runtime "db\sonetto-3.8-cn.db")
    database_exists = (Test-Path -LiteralPath (Join-Path $runtime "db\sonetto-3.8-cn.db"))
} | ConvertTo-Json -Compress
