[CmdletBinding()]
param(
    [string]$ResourceCheckJson,

    [string]$ResourceCheckUri = "http://127.0.0.1:21000/resource/60001/check",

    [string]$OutputDirectory,

    [string[]]$Groups = @(
        "res-oppartygame",
        "media-oppartygame",
        "en-oppartygame",
        "jp-oppartygame",
        "zh-oppartygame",
        "kr-oppartygame",
        "res-opveract",
        "media-opveract",
        "en-opveract",
        "jp-opveract",
        "zh-opveract",
        "kr-opveract",
        "res-opexplore",
        "media-opexplore",
        "en-opexplore",
        "jp-opexplore",
        "zh-opexplore",
        "kr-opexplore"
    ),

    [string]$CurrentVersion = "109.0",

    [switch]$DryRun
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$strictUtf8 = [Text.UTF8Encoding]::new($false, $true)
$approvedHosts = @(
    "optionalres-res-hw.sl916.com",
    "optionalres-res-bak-hw.sl916.com"
)
$toolRoot = $PSScriptRoot
$repositoryRoot = (Resolve-Path -LiteralPath (Join-Path $toolRoot "..\..")).Path

if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
    $OutputDirectory = Join-Path $repositoryRoot "runtime\cdn-cache\optionalres"
}

$outputRoot = [IO.Path]::GetFullPath($OutputDirectory)
$manifestPath = Join-Path $outputRoot "cache-manifest.json"
$resourceCheckPath = Join-Path $outputRoot "resource-check.json"
$requestedGroups = @($Groups)
$requestedGroupSet = @{}

if ($requestedGroups.Count -eq 0) {
    throw "At least one resource group is required"
}
foreach ($requestedGroup in $requestedGroups) {
    if ($requestedGroup -notmatch '^[A-Za-z0-9][A-Za-z0-9._-]*$') {
        throw "Requested resource group '$requestedGroup' is invalid"
    }
    if ($requestedGroupSet.ContainsKey($requestedGroup)) {
        throw "Requested resource group '$requestedGroup' is duplicated"
    }
    $requestedGroupSet[$requestedGroup] = $true
}

function Read-StrictJsonText {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Text,

        [Parameter(Mandatory = $true)]
        [string]$Label
    )

    try {
        return $Text | ConvertFrom-Json
    }
    catch {
        throw "$Label is not valid JSON: $($_.Exception.Message)"
    }
}

function Read-StrictUtf8File {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,

        [Parameter(Mandatory = $true)]
        [string]$Label
    )

    try {
        return [IO.File]::ReadAllText($Path, $strictUtf8)
    }
    catch {
        throw "$Label is not valid UTF-8: $($_.Exception.Message)"
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

function Assert-ApprovedCdnBaseUrl {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Value,

        [Parameter(Mandatory = $true)]
        [string]$Context
    )

    $uri = $null
    if (-not [Uri]::TryCreate($Value, [UriKind]::Absolute, [ref]$uri)) {
        throw "$Context must be an absolute URL"
    }
    if ($uri.Scheme -cne "https") {
        throw "$Context must use HTTPS"
    }
    if ($approvedHosts -cnotcontains $uri.DnsSafeHost.ToLowerInvariant()) {
        throw "$Context must use an approved CDN host"
    }
    if (-not $uri.IsDefaultPort -or -not [string]::IsNullOrEmpty($uri.UserInfo)) {
        throw "$Context must not contain credentials or a custom port"
    }
    if (-not [string]::IsNullOrEmpty($uri.Query) -or -not [string]::IsNullOrEmpty($uri.Fragment)) {
        throw "$Context must not contain a query or fragment"
    }

    return $Value.TrimEnd('/')
}

function Assert-SafeRelativeResourcePath {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Value,

        [Parameter(Mandatory = $true)]
        [string]$Context
    )

    if (
        [string]::IsNullOrWhiteSpace($Value) -or
        [IO.Path]::IsPathRooted($Value) -or
        $Value.Contains("\")
    ) {
        throw "$Context must be a safe relative path"
    }

    $invalidCharacters = [IO.Path]::GetInvalidFileNameChars()
    $segments = @($Value.Split('/'))
    if ($segments.Count -eq 0) {
        throw "$Context must be a safe relative path"
    }

    foreach ($segment in $segments) {
        if (
            [string]::IsNullOrWhiteSpace($segment) -or
            $segment -eq "." -or
            $segment -eq ".." -or
            $segment.IndexOfAny($invalidCharacters) -ge 0
        ) {
            throw "$Context must be a safe relative path"
        }
    }

    if (-not $segments[-1].EndsWith(".zip", [StringComparison]::OrdinalIgnoreCase)) {
        throw "$Context must name a ZIP archive"
    }

    return $segments
}

function Join-CdnUrl {
    param(
        [Parameter(Mandatory = $true)]
        [string]$BaseUrl,

        [Parameter(Mandatory = $true)]
        [string[]]$Segments
    )

    $escapedPath = ($Segments | ForEach-Object { [Uri]::EscapeDataString($_) }) -join "/"
    return "$BaseUrl/$escapedPath"
}

function Write-Manifest {
    param(
        [Parameter(Mandatory = $true)]
        [System.Collections.IEnumerable]$Resources,

        [Parameter(Mandatory = $true)]
        [string]$Source
    )

    $manifest = [ordered]@{
        generatedAtUtc = [DateTime]::UtcNow.ToString("o")
        dryRun = [bool]$DryRun
        resourceCheckSource = $Source
        resources = @($Resources)
    }
    $json = $manifest | ConvertTo-Json -Depth 8
    [IO.File]::WriteAllText($manifestPath, $json + "`n", $strictUtf8)
}

function Test-ArchiveIntegrity {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,

        [Parameter(Mandatory = $true)]
        [long]$ExpectedLength,

        [Parameter(Mandatory = $true)]
        [string]$ExpectedMd5
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        return $false
    }

    $item = Get-Item -LiteralPath $Path
    if ($item.Length -ne $ExpectedLength) {
        return $false
    }

    $actualMd5 = (Get-FileHash -LiteralPath $Path -Algorithm MD5).Hash.ToLowerInvariant()
    return $actualMd5 -ceq $ExpectedMd5
}

[IO.Directory]::CreateDirectory($outputRoot) | Out-Null

if ([string]::IsNullOrWhiteSpace($ResourceCheckJson)) {
    if ([string]::IsNullOrWhiteSpace($CurrentVersion)) {
        throw "CurrentVersion is required"
    }
    if ($CurrentVersion -notmatch '^[0-9]+(?:\.[0-9]+)*$') {
        throw "CurrentVersion must contain only numeric version segments"
    }

    $versions = (@($requestedGroups | ForEach-Object { $CurrentVersion })) -join ","
    $separator = if ($ResourceCheckUri.Contains("?")) { "&" } else { "?" }
    $requestUri = $ResourceCheckUri + $separator +
        "os_type=2&lang=" + ($requestedGroups -join ",") +
        "&version=" + $versions +
        "&env_type=4&channel_id=200"
    $response = Invoke-WebRequest -Uri $requestUri -UseBasicParsing -TimeoutSec 60
    $resourceCheckText = [string]$response.Content
    $resourceCheckSource = $requestUri
}
else {
    $resolvedResourceCheckJson = (Resolve-Path -LiteralPath $ResourceCheckJson).Path
    $resourceCheckText = Read-StrictUtf8File -Path $resolvedResourceCheckJson -Label "Resource check fixture"
    $resourceCheckSource = $resolvedResourceCheckJson
}

$resourceCheck = Read-StrictJsonText -Text $resourceCheckText -Label "Resource check response"
[IO.File]::WriteAllText($resourceCheckPath, $resourceCheckText.TrimEnd() + "`n", $strictUtf8)

$resources = New-Object System.Collections.Generic.List[object]
foreach ($groupProperty in $resourceCheck.PSObject.Properties) {
    $groupName = [string]$groupProperty.Name
    if ($groupName -notmatch '^[A-Za-z0-9][A-Za-z0-9._-]*$') {
        throw "Resource group '$groupName' is not a safe directory name"
    }
    if (-not $requestedGroupSet.ContainsKey($groupName)) {
        throw "Resource group '$groupName' was not requested"
    }

    $group = $groupProperty.Value
    $latestVersion = [string](Get-RequiredProperty -Object $group -Name "latest_ver" -Context "Resource group '$groupName'")
    $primaryBaseUrl = Assert-ApprovedCdnBaseUrl -Value ([string](Get-RequiredProperty -Object $group -Name "download_url" -Context "Resource group '$groupName'")) -Context "Resource group '$groupName' download_url"
    $backupBaseUrl = Assert-ApprovedCdnBaseUrl -Value ([string](Get-RequiredProperty -Object $group -Name "download_url_bak" -Context "Resource group '$groupName'")) -Context "Resource group '$groupName' download_url_bak"
    $groupResources = @(Get-RequiredProperty -Object $group -Name "res" -Context "Resource group '$groupName'")

    foreach ($resource in $groupResources) {
        $name = [string](Get-RequiredProperty -Object $resource -Name "name" -Context "Resource in group '$groupName'")
        $segments = Assert-SafeRelativeResourcePath -Value $name -Context "Resource '$name'"
        $expectedMd5 = ([string](Get-RequiredProperty -Object $resource -Name "hash" -Context "Resource '$name'")).ToLowerInvariant()
        if ($expectedMd5 -notmatch '^[0-9a-f]{32}$') {
            throw "Resource '$name' hash must be a 32-character MD5"
        }

        try {
            $expectedLength = [Convert]::ToInt64((Get-RequiredProperty -Object $resource -Name "length" -Context "Resource '$name'"))
        }
        catch {
            throw "Resource '$name' length must be an integer"
        }
        if ($expectedLength -le 0) {
            throw "Resource '$name' length must be positive"
        }

        $relativeSegments = @("files", $groupName) + $segments
        $relativePath = $relativeSegments -join "/"
        $destinationPath = $outputRoot
        foreach ($segment in $relativeSegments) {
            $destinationPath = Join-Path $destinationPath $segment
        }

        $resources.Add([pscustomobject][ordered]@{
            group = $groupName
            latestVersion = $latestVersion
            name = $name
            sourceUrl = Join-CdnUrl -BaseUrl $primaryBaseUrl -Segments $segments
            backupSourceUrl = Join-CdnUrl -BaseUrl $backupBaseUrl -Segments $segments
            relativePath = $relativePath
            length = $expectedLength
            md5 = $expectedMd5
            status = if ($DryRun) { "planned" } else { "pending" }
            destinationPath = $destinationPath
        })
    }
}

Write-Manifest -Resources $resources -Source $resourceCheckSource

if ($DryRun) {
    Write-Output "Planned $($resources.Count) optional resource archive(s) in $outputRoot"
    return
}

$curl = (Get-Command curl.exe -ErrorAction Stop).Source
for ($index = 0; $index -lt $resources.Count; $index++) {
    $resource = $resources[$index]
    $destinationPath = [string]$resource.destinationPath
    $partPath = "$destinationPath.part"
    [IO.Directory]::CreateDirectory((Split-Path -Parent $destinationPath)) | Out-Null

    if (Test-ArchiveIntegrity -Path $destinationPath -ExpectedLength $resource.length -ExpectedMd5 $resource.md5) {
        $resource.status = "cached"
        Write-Output "[$($index + 1)/$($resources.Count)] cached $($resource.group)/$($resource.name)"
        Write-Manifest -Resources $resources -Source $resourceCheckSource
        continue
    }
    if (Test-Path -LiteralPath $destinationPath -PathType Leaf) {
        throw "Existing archive failed integrity verification: $destinationPath"
    }

    Write-Output "[$($index + 1)/$($resources.Count)] downloading $($resource.group)/$($resource.name)"
    & $curl --fail --location --silent --show-error --retry 5 --retry-delay 2 --continue-at - --output $partPath $resource.sourceUrl
    if ($LASTEXITCODE -ne 0) {
        throw "curl failed with exit code $LASTEXITCODE for $($resource.sourceUrl)"
    }
    if (-not (Test-ArchiveIntegrity -Path $partPath -ExpectedLength $resource.length -ExpectedMd5 $resource.md5)) {
        throw "Downloaded archive failed length or MD5 verification: $partPath"
    }

    Move-Item -LiteralPath $partPath -Destination $destinationPath
    $resource.status = "verified"
    Write-Manifest -Resources $resources -Source $resourceCheckSource
}

Write-Output "Verified and cached $($resources.Count) optional resource archive(s) in $outputRoot"
