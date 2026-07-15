[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$SummonJson,

    [Parameter(Mandatory = $true)]
    [string]$CharacterJson,

    [Parameter(Mandatory = $true)]
    [string]$OutputPath,

    [switch]$Force
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$strictUtf8 = [Text.UTF8Encoding]::new($false, $true)
$summonSource = (Resolve-Path -LiteralPath $SummonJson).Path
$characterSource = (Resolve-Path -LiteralPath $CharacterJson).Path
$outputFile = [IO.Path]::GetFullPath($OutputPath)
$pathComparer = [StringComparer]::OrdinalIgnoreCase

if (
    $pathComparer.Equals($outputFile, $summonSource) -or
    $pathComparer.Equals($outputFile, $characterSource)
) {
    throw "Output path must not be the same as either input source"
}

function Read-StrictJson {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,

        [Parameter(Mandatory = $true)]
        [string]$Label
    )

    try {
        $text = [IO.File]::ReadAllText($Path, $strictUtf8)
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

function Convert-RequiredInt {
    param(
        [Parameter(Mandatory = $true)]
        $Value,

        [Parameter(Mandatory = $true)]
        [string]$Label,

        [int]$Minimum,

        [int]$Maximum = [int]::MaxValue
    )

    if ($null -eq $Value) {
        throw "$Label must be an integer"
    }
    $typeCode = [Type]::GetTypeCode($Value.GetType())
    $integerTypes = @(
        [TypeCode]::SByte,
        [TypeCode]::Byte,
        [TypeCode]::Int16,
        [TypeCode]::UInt16,
        [TypeCode]::Int32,
        [TypeCode]::UInt32,
        [TypeCode]::Int64
    )
    if ($integerTypes -notcontains $typeCode) {
        throw "$Label must be an integer"
    }
    $integer = [long]$Value
    if ($integer -lt $Minimum -or $integer -gt $Maximum) {
        throw "$Label must be between $Minimum and $Maximum"
    }
    return [int]$integer
}

function Assert-Tuple {
    param(
        [Parameter(Mandatory = $true)]
        $Value,

        [Parameter(Mandatory = $true)]
        [string]$ExpectedName,

        [Parameter(Mandatory = $true)]
        [string]$Label
    )

    if (
        $Value -isnot [Array] -or
        $Value.Count -ne 2 -or
        $Value[0] -ne $ExpectedName -or
        $Value[1] -isnot [Array] -or
        $Value[1].Count -eq 0
    ) {
        throw "$Label must be a non-empty '$ExpectedName' tuple"
    }
}

function Get-PoolId {
    param(
        [Parameter(Mandatory = $true)]
        $Record,

        [Parameter(Mandatory = $true)]
        [string]$Context
    )

    $value = Get-RequiredProperty -Object $Record -Name "id" -Context $Context
    return Convert-RequiredInt -Value $value -Label "$Context id" -Minimum 1
}

function Get-Rarity {
    param(
        [Parameter(Mandatory = $true)]
        $Record,

        [Parameter(Mandatory = $true)]
        [string]$Context
    )

    $value = Get-RequiredProperty -Object $Record -Name "rare" -Context $Context
    return Convert-RequiredInt -Value $value -Label "$Context rare" -Minimum 1 -Maximum 5
}

$summonTuple = Read-StrictJson -Path $summonSource -Label "Summon source"
$characterTuple = Read-StrictJson -Path $characterSource -Label "Character source"
Assert-Tuple -Value $summonTuple -ExpectedName "summon" -Label "Summon source"
Assert-Tuple -Value $characterTuple -ExpectedName "character" -Label "Character source"

$summonRecords = @($summonTuple[1])
$characterRecords = @($characterTuple[1])
$baselinePoolRows = @()

for ($index = 0; $index -lt $summonRecords.Count; $index++) {
    $record = $summonRecords[$index]
    $poolId = Get-PoolId -Record $record -Context "Summon row $index"
    if ($poolId -eq 2) {
        $baselinePoolRows += $record
    }
}

if ($baselinePoolRows.Count -ne 5) {
    throw "Standard pool 2 must contain exactly five source rows"
}
$baselineRarities = @($baselinePoolRows | ForEach-Object {
    Get-Rarity -Record $_ -Context "Standard pool 2 row"
})
if (($baselineRarities | Sort-Object -Unique).Count -ne 5) {
    throw "Standard pool 2 must contain one source row for each rarity 1 through 5"
}

$characterIds = [Collections.Generic.HashSet[int]]::new()
$onlineIdsByRarity = @{}
foreach ($rarity in 1..5) {
    $onlineIdsByRarity[$rarity] = [Collections.Generic.List[int]]::new()
}

for ($index = 0; $index -lt $characterRecords.Count; $index++) {
    $record = $characterRecords[$index]
    $characterId = Get-PoolId -Record $record -Context "Character row $index"
    if (-not $characterIds.Add($characterId)) {
        throw "Duplicate character ID: $characterId"
    }
    $rarity = Get-Rarity -Record $record -Context "Character $characterId"
    $onlineValue = [string](Get-RequiredProperty -Object $record -Name "isOnline" -Context "Character $characterId")
    if ($onlineValue -eq "1") {
        $onlineIdsByRarity[$rarity].Add($characterId)
    }
}

$replacementRows = @()
$eligibleIds = [Collections.Generic.HashSet[int]]::new()
foreach ($rarity in @(5, 4, 3, 2, 1)) {
    $ids = @($onlineIdsByRarity[$rarity] | Sort-Object)
    if ($ids.Count -eq 0) {
        throw "No online characters found for rarity $rarity"
    }
    foreach ($characterId in $ids) {
        if (-not $eligibleIds.Add($characterId)) {
            throw "Duplicate online character ID across rarities: $characterId"
        }
    }
    $replacementRows += [pscustomobject][ordered]@{
        id = 2
        rare = $rarity
        summonId = ($ids -join "#")
        luckyBagId = ""
    }
}

$outputRecords = [Collections.Generic.List[object]]::new()
$insertedReplacement = $false
foreach ($record in $summonRecords) {
    $poolId = Get-PoolId -Record $record -Context "Summon row"
    if ($poolId -eq 2) {
        if (-not $insertedReplacement) {
            foreach ($replacement in $replacementRows) {
                $outputRecords.Add($replacement)
            }
            $insertedReplacement = $true
        }
        continue
    }
    $outputRecords.Add($record)
}

if (-not $insertedReplacement -or $outputRecords.Count -ne $summonRecords.Count) {
    throw "Failed to replace standard pool 2 without changing total row count"
}
if (Test-Path -LiteralPath $outputFile -PathType Leaf) {
    if (-not $Force) {
        throw "Output file already exists; rerun with -Force: $outputFile"
    }
}

$outputDirectory = Split-Path -Parent $outputFile
if (-not (Test-Path -LiteralPath $outputDirectory -PathType Container)) {
    [IO.Directory]::CreateDirectory($outputDirectory) | Out-Null
}

$temporaryFile = "$outputFile.tmp.$([Guid]::NewGuid().ToString("N"))"
$replacementBackup = "$outputFile.replace-backup.$([Guid]::NewGuid().ToString("N"))"
try {
    $recordsJson = $outputRecords | ConvertTo-Json -Depth 10
    $outputJson = "[`r`n  `"summon`",`r`n$recordsJson`r`n]`r`n"
    [IO.File]::WriteAllText($temporaryFile, $outputJson, [Text.UTF8Encoding]::new($false))

    $verificationTuple = Read-StrictJson -Path $temporaryFile -Label "Generated output"
    Assert-Tuple -Value $verificationTuple -ExpectedName "summon" -Label "Generated output"
    if (@($verificationTuple[1]).Count -ne $summonRecords.Count) {
        throw "Generated output row count changed"
    }
    $verificationPoolRows = @($verificationTuple[1] | Where-Object {
        (Get-PoolId -Record $_ -Context "Generated summon row") -eq 2
    })
    if ($verificationPoolRows.Count -ne 5) {
        throw "Generated output does not contain five pool 2 rows"
    }
    for ($index = 0; $index -lt 5; $index++) {
        $expectedRarity = 5 - $index
        $verificationRow = $verificationPoolRows[$index]
        $actualRarity = Get-Rarity -Record $verificationRow -Context "Generated pool 2 row"
        if ($actualRarity -ne $expectedRarity) {
            throw "Generated pool 2 rarity order is invalid"
        }
        $actualIds = @(
            $verificationRow.summonId.Split("#", [StringSplitOptions]::RemoveEmptyEntries) |
                ForEach-Object { [int]$_ }
        )
        $expectedIds = @($onlineIdsByRarity[$expectedRarity] | Sort-Object)
        if (($actualIds -join ",") -ne ($expectedIds -join ",")) {
            throw "Generated pool 2 membership differs for rarity $expectedRarity"
        }
    }
    if ($eligibleIds.Count -ne ($onlineIdsByRarity.Values | ForEach-Object { $_.Count } | Measure-Object -Sum).Sum) {
        throw "Generated online character IDs are not unique"
    }
    if ($eligibleIds.Contains(3029) -or $eligibleIds.Contains(9998)) {
        throw "Generated output contains offline character IDs"
    }

    if (Test-Path -LiteralPath $outputFile -PathType Leaf) {
        [IO.File]::Replace($temporaryFile, $outputFile, $replacementBackup)
        Remove-Item -LiteralPath $replacementBackup -Force
    }
    else {
        [IO.File]::Move($temporaryFile, $outputFile)
    }
}
finally {
    if (Test-Path -LiteralPath $temporaryFile -PathType Leaf) {
        Remove-Item -LiteralPath $temporaryFile -Force
    }
    if (Test-Path -LiteralPath $replacementBackup -PathType Leaf) {
        Remove-Item -LiteralPath $replacementBackup -Force
    }
}

Write-Output "Generated standard pool 2 with $($eligibleIds.Count) online characters"
foreach ($rarity in @(5, 4, 3, 2, 1)) {
    Write-Output "rarity $rarity`: $($onlineIdsByRarity[$rarity].Count)"
}
Write-Output "Output: $outputFile"
