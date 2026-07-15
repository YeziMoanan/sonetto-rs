[CmdletBinding()]
param(
    [string]$SummonJson,

    [string]$CharacterJson
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$toolRoot = Split-Path -Parent $PSScriptRoot
$generator = Join-Path $toolRoot "New-DefaultAllCharactersSummon.ps1"
$expectedArtifact = Join-Path $toolRoot "presets\default-all-characters\summon.json"
$tempDirectory = Join-Path ([IO.Path]::GetTempPath()) ("all-character-pool-test-" + [Guid]::NewGuid().ToString("N"))
$strictUtf8 = [Text.UTF8Encoding]::new($false, $true)
$workspaceRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..\..\..\..")).Path

if ([string]::IsNullOrWhiteSpace($SummonJson)) {
    $SummonJson = Join-Path $workspaceRoot "sonetto-data\excel2json\summon.json"
}
if ([string]::IsNullOrWhiteSpace($CharacterJson)) {
    $CharacterJson = Join-Path $workspaceRoot "sonetto-data\excel2json\character.json"
}

function Read-StrictJson {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    $text = [IO.File]::ReadAllText($Path, $strictUtf8)
    $value = $text | ConvertFrom-Json
    Write-Output -NoEnumerate $value
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

function Assert-SequenceEqual {
    param(
        [Parameter(Mandatory = $true)]
        [object[]]$Expected,

        [Parameter(Mandatory = $true)]
        [object[]]$Actual,

        [Parameter(Mandatory = $true)]
        [string]$Message
    )

    Assert-Equal -Expected $Expected.Count -Actual $Actual.Count -Message "$Message count"
    for ($index = 0; $index -lt $Expected.Count; $index++) {
        if ($Expected[$index].ToString() -ne $Actual[$index].ToString()) {
            throw "$Message at index $index. Expected '$($Expected[$index])', got '$($Actual[$index])'"
        }
    }
}

function Assert-RejectsSourceOutput {
    param(
        [Parameter(Mandatory = $true)]
        [string]$OutputPath,

        [Parameter(Mandatory = $true)]
        [string]$Name,

        [Parameter(Mandatory = $true)]
        [string]$ResolvedSummon,

        [Parameter(Mandatory = $true)]
        [string]$ResolvedCharacters
    )

    $rejected = $false
    try {
        & $generator `
            -SummonJson $ResolvedSummon `
            -CharacterJson $ResolvedCharacters `
            -OutputPath $OutputPath `
            -Force | Out-Null
    }
    catch {
        $rejected = $true
        if ($_.Exception.Message -notmatch "input|source|same") {
            throw "$Name output collision failed with unexpected message: $($_.Exception.Message)"
        }
    }
    if (-not $rejected) {
        throw "Generator accepted a forbidden output collision for $Name"
    }
}

$resolvedSummon = (Resolve-Path -LiteralPath $SummonJson).Path
$resolvedCharacters = (Resolve-Path -LiteralPath $CharacterJson).Path
$summonHashBefore = (Get-FileHash -Algorithm SHA256 -LiteralPath $resolvedSummon).Hash
$characterHashBefore = (Get-FileHash -Algorithm SHA256 -LiteralPath $resolvedCharacters).Hash

New-Item -ItemType Directory -Path $tempDirectory | Out-Null

try {
    if (-not (Test-Path -LiteralPath $generator -PathType Leaf)) {
        throw "Generator not found: $generator"
    }

    $temporaryOutput = Join-Path $tempDirectory "summon.json"
    & $generator `
        -SummonJson $resolvedSummon `
        -CharacterJson $resolvedCharacters `
        -OutputPath $temporaryOutput | Out-Null

    $source = Read-StrictJson -Path $resolvedSummon
    $characters = Read-StrictJson -Path $resolvedCharacters
    $output = Read-StrictJson -Path $temporaryOutput

    Assert-Equal -Expected "summon" -Actual $output[0] -Message "Output tuple name"
    Assert-Equal -Expected @($source[1]).Count -Actual @($output[1]).Count -Message "Output summon row count"

    $sourceOtherRows = @($source[1] | Where-Object { [int]$_.id -ne 2 })
    $outputOtherRows = @($output[1] | Where-Object { [int]$_.id -ne 2 })
    Assert-Equal -Expected $sourceOtherRows.Count -Actual $outputOtherRows.Count -Message "Non-pool-2 row count"
    for ($index = 0; $index -lt $sourceOtherRows.Count; $index++) {
        $expectedJson = $sourceOtherRows[$index] | ConvertTo-Json -Compress -Depth 10
        $actualJson = $outputOtherRows[$index] | ConvertTo-Json -Compress -Depth 10
        Assert-Equal -Expected $expectedJson -Actual $actualJson -Message "Non-pool-2 row $index"
    }

    $poolRows = @($output[1] | Where-Object { [int]$_.id -eq 2 })
    Assert-Equal -Expected 5 -Actual $poolRows.Count -Message "Pool 2 rarity row count"
    Assert-SequenceEqual `
        -Expected @(5, 4, 3, 2, 1) `
        -Actual @($poolRows | ForEach-Object { [int]$_.rare }) `
        -Message "Pool 2 rarity order"

    $expectedCounts = @{ 5 = 61; 4 = 29; 3 = 17; 2 = 11; 1 = 2 }
    $allActualIds = [Collections.Generic.HashSet[int]]::new()

    foreach ($rarity in @(5, 4, 3, 2, 1)) {
        $expectedIds = @(
            $characters[1] |
                Where-Object { $_.isOnline -eq "1" -and [int]$_.rare -eq $rarity } |
                ForEach-Object { [int]$_.id } |
                Sort-Object
        )
        $poolRow = @($poolRows | Where-Object { [int]$_.rare -eq $rarity })[0]
        $actualIds = @(
            $poolRow.summonId.Split("#", [StringSplitOptions]::RemoveEmptyEntries) |
                ForEach-Object { [int]$_ }
        )

        Assert-Equal -Expected $expectedCounts[$rarity] -Actual $actualIds.Count -Message "Rarity $rarity count"
        Assert-SequenceEqual -Expected $expectedIds -Actual $actualIds -Message "Rarity $rarity membership"

        foreach ($characterId in $actualIds) {
            if (-not $allActualIds.Add($characterId)) {
                throw "Duplicate character ID across rarity rows: $characterId"
            }
        }
    }

    Assert-Equal -Expected 120 -Actual $allActualIds.Count -Message "Unique online character count"
    if ($allActualIds.Contains(3029) -or $allActualIds.Contains(9998)) {
        throw "Offline characters 3029 or 9998 were included"
    }

    $firstOutputHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $temporaryOutput).Hash
    & $generator `
        -SummonJson $resolvedSummon `
        -CharacterJson $resolvedCharacters `
        -OutputPath $temporaryOutput `
        -Force | Out-Null
    $secondOutputHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $temporaryOutput).Hash
    Assert-Equal -Expected $firstOutputHash -Actual $secondOutputHash -Message "Forced regeneration hash"

    Assert-RejectsSourceOutput `
        -OutputPath $resolvedSummon `
        -Name "Summon source" `
        -ResolvedSummon $resolvedSummon `
        -ResolvedCharacters $resolvedCharacters
    Assert-RejectsSourceOutput `
        -OutputPath $resolvedCharacters `
        -Name "Character source" `
        -ResolvedSummon $resolvedSummon `
        -ResolvedCharacters $resolvedCharacters

    Assert-Equal `
        -Expected $summonHashBefore `
        -Actual (Get-FileHash -Algorithm SHA256 -LiteralPath $resolvedSummon).Hash `
        -Message "Summon source hash"
    Assert-Equal `
        -Expected $characterHashBefore `
        -Actual (Get-FileHash -Algorithm SHA256 -LiteralPath $resolvedCharacters).Hash `
        -Message "Character source hash"

    if (-not (Test-Path -LiteralPath $expectedArtifact -PathType Leaf)) {
        throw "Generated artifact not found: $expectedArtifact"
    }
    Assert-Equal `
        -Expected $firstOutputHash `
        -Actual (Get-FileHash -Algorithm SHA256 -LiteralPath $expectedArtifact).Hash `
        -Message "Checked-in artifact hash"

    Write-Output "PASS: 120 online characters, exact rarity membership, deterministic output, and unchanged sources"
}
finally {
    if (Test-Path -LiteralPath $tempDirectory) {
        Remove-Item -LiteralPath $tempDirectory -Recurse -Force
    }
}
