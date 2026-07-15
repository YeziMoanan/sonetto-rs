[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$toolRoot = Split-Path -Parent $PSScriptRoot
$scriptPath = Join-Path $toolRoot "Save-OptionalResources.ps1"
$tempRoot = Join-Path ([IO.Path]::GetTempPath()) ("optional-resource-cache-test-" + [Guid]::NewGuid().ToString("N"))
$strictUtf8 = [Text.UTF8Encoding]::new($false, $true)

function Assert-True {
    param(
        [Parameter(Mandatory = $true)]
        [bool]$Condition,

        [Parameter(Mandatory = $true)]
        [string]$Message
    )

    if (-not $Condition) {
        throw $Message
    }
}

function Write-Fixture {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,

        [Parameter(Mandatory = $true)]
        $Value
    )

    $json = $Value | ConvertTo-Json -Depth 8
    [IO.File]::WriteAllText($Path, $json + "`n", $strictUtf8)
}

function Assert-RejectedFixture {
    param(
        [Parameter(Mandatory = $true)]
        $Fixture,

        [Parameter(Mandatory = $true)]
        [string]$ExpectedPattern,

        [string[]]$Groups
    )

    $fixturePath = Join-Path $tempRoot ("fixture-" + [Guid]::NewGuid().ToString("N") + ".json")
    $outputPath = Join-Path $tempRoot ("output-" + [Guid]::NewGuid().ToString("N"))
    Write-Fixture -Path $fixturePath -Value $Fixture

    try {
        if ($null -ne $Groups -and $Groups.Length -gt 0) {
            & $scriptPath -ResourceCheckJson $fixturePath -OutputDirectory $outputPath -Groups $Groups -DryRun
        }
        else {
            & $scriptPath -ResourceCheckJson $fixturePath -OutputDirectory $outputPath -DryRun
        }
        throw "Fixture was accepted but should have been rejected"
    }
    catch {
        Assert-True ($_.Exception.Message -match $ExpectedPattern) "Expected rejection matching '$ExpectedPattern', got '$($_.Exception.Message)'"
    }
}

try {
    Assert-True (Test-Path -LiteralPath $scriptPath -PathType Leaf) "Missing cache tool: $scriptPath"
    [IO.Directory]::CreateDirectory($tempRoot) | Out-Null

    $validFixture = [ordered]@{
        "res-opveract" = [ordered]@{
            res = @(
                [ordered]@{
                    hash = "098f6bcd4621d373cade4e832627b4f6"
                    name = "merge/test.zip"
                    length = 4
                    order = 1
                }
            )
            latest_ver = "109.113"
            download_url = "https://optionalres-res-hw.sl916.com/uploadzip/60001/4/63"
            download_url_bak = "https://optionalres-res-bak-hw.sl916.com/uploadzip/60001/4/63"
        }
        "en-opveract" = [ordered]@{
            res = @(
                [ordered]@{
                    hash = "5d41402abc4b2a76b9719d911017c592"
                    name = "language.zip"
                    length = 5
                    order = 1
                }
            )
            latest_ver = "109.113"
            download_url = "https://optionalres-res-bak-hw.sl916.com/uploadzip/60001/4/63"
            download_url_bak = "https://optionalres-res-hw.sl916.com/uploadzip/60001/4/63"
        }
    }

    $validFixturePath = Join-Path $tempRoot "valid.json"
    $validOutputPath = Join-Path $tempRoot "valid-output"
    Write-Fixture -Path $validFixturePath -Value $validFixture
    & $scriptPath -ResourceCheckJson $validFixturePath -OutputDirectory $validOutputPath -DryRun

    $manifestPath = Join-Path $validOutputPath "cache-manifest.json"
    Assert-True (Test-Path -LiteralPath $manifestPath -PathType Leaf) "Dry run did not create cache-manifest.json"
    $manifest = [IO.File]::ReadAllText($manifestPath, $strictUtf8) | ConvertFrom-Json
    Assert-True (@($manifest.resources).Count -eq 2) "Expected two planned resources"
    Assert-True ($manifest.resources[0].relativePath -eq "files/res-opveract/merge/test.zip") "Nested resource path was not preserved"
    Assert-True ($manifest.resources[1].sourceUrl -eq "https://optionalres-res-bak-hw.sl916.com/uploadzip/60001/4/63/language.zip") "Backup CDN host was not accepted"

    $unsafeHostFixture = [ordered]@{
        "res-opveract" = [ordered]@{
            res = @([ordered]@{ hash = "098f6bcd4621d373cade4e832627b4f6"; name = "test.zip"; length = 4; order = 1 })
            latest_ver = "109.113"
            download_url = "https://evil.optionalres-res-hw.sl916.com/uploadzip"
            download_url_bak = "https://optionalres-res-bak-hw.sl916.com/uploadzip"
        }
    }
    Assert-RejectedFixture -Fixture $unsafeHostFixture -ExpectedPattern "approved CDN host"

    $traversalFixture = [ordered]@{
        "res-opveract" = [ordered]@{
            res = @([ordered]@{ hash = "098f6bcd4621d373cade4e832627b4f6"; name = "../escape.zip"; length = 4; order = 1 })
            latest_ver = "109.113"
            download_url = "https://optionalres-res-hw.sl916.com/uploadzip"
            download_url_bak = "https://optionalres-res-bak-hw.sl916.com/uploadzip"
        }
    }
    Assert-RejectedFixture -Fixture $traversalFixture -ExpectedPattern "safe relative path"

    $unexpectedGroupFixture = [ordered]@{
        "res-HD" = [ordered]@{
            res = @([ordered]@{ hash = "098f6bcd4621d373cade4e832627b4f6"; name = "legacy.zip"; length = 4; order = 1 })
            latest_ver = "101.65"
            download_url = "https://optionalres-res-hw.sl916.com/uploadzip"
            download_url_bak = "https://optionalres-res-bak-hw.sl916.com/uploadzip"
        }
    }
    Assert-RejectedFixture -Fixture $unexpectedGroupFixture -ExpectedPattern "was not requested" -Groups @("res-opveract")

    Write-Output "PASS: exact CDN allowlist, normalized cache plan, and unsafe input rejection"
}
finally {
    if (Test-Path -LiteralPath $tempRoot) {
        Remove-Item -LiteralPath $tempRoot -Recurse -Force
    }
}
