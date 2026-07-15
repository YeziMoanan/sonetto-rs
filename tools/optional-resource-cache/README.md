# Optional resource cache

`Save-OptionalResources.ps1` reads the international client optional-resource manifest through local `sdkserver`, downloads the returned ZIP archives, and verifies both byte length and MD5 before keeping them.

The tool accepts only these exact HTTPS hosts:

- `optionalres-res-hw.sl916.com`
- `optionalres-res-bak-hw.sl916.com`

It does not change any proxy, Windows setting, Android setting, registry key, environment variable, or `tcp_rules`.

## Cache all groups requested by the 3.6 client

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools\optional-resource-cache\Save-OptionalResources.ps1
```

The default output is `runtime/cdn-cache/optionalres`. Interrupted downloads remain as `.part` files and resume on the next run. Completed archives are skipped only after length and MD5 verification.

## Dry run from a saved response

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools\optional-resource-cache\Save-OptionalResources.ps1 `
  -ResourceCheckJson C:\path\to\resource-check.json `
  -OutputDirectory C:\path\to\cache `
  -DryRun
```

## Test

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools\optional-resource-cache\tests\Test-SaveOptionalResources.ps1
```
