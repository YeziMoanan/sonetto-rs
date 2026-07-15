# Banner Scheduler Design

## Goal

Provide a local, dependency-free page for selecting and scheduling Reverse: 1999 summon pools from the international 3.6 `summon_pool.json`, plus a fixed PowerShell executor that can safely reproduce the exported schedule in a Sonetto SQLite database.

The page never opens or writes a database. The executor performs a dry run unless the operator explicitly supplies `-Apply`.

## Scope

- Load the UTF-8 source file `sonetto-data/excel2json/summon_pool.json` through a browser file picker or drag-and-drop target.
- Search, filter, select, and reorder all pools found in the source file.
- Provide built-in pool presets and browser-local custom presets.
- Generate simultaneous, rotating batch, or manually dated schedules.
- Export a versioned JSON preset that can be imported and edited later.
- Validate and optionally apply an exported preset to `banner_schedule` and existing matching `user_summon_pools` rows.
- Back up the SQLite database before every non-test apply unless `-SkipBackup` is explicitly supplied.

## Explicit Exclusions

- Do not apply a generated preset to the current runtime database as part of building or testing this tool.
- Do not modify server protocol handling, generated protocol files, the official `GameAssembly.dll`, `sonetto-patch`, proxy settings, or `tcp_rules`.
- Do not commit, push, or open a pull request.
- Do not modify `assets/static/activity/activity_infos.json`.
- The “all activity” preset means summon pools with `bannerFlag = 2`; it does not enable game activities handled by `GetActivityInfos`.

## File Layout

- `tools/banner-scheduler/index.html`: semantic page structure and controls.
- `tools/banner-scheduler/styles.css`: responsive visual styling.
- `tools/banner-scheduler/scheduler.js`: pure source parsing, filtering, preset selection, validation, and schedule generation.
- `tools/banner-scheduler/app.js`: DOM state, file operations, queue editing, custom presets, and downloads.
- `tools/banner-scheduler/Apply-BannerPreset.ps1`: dry-run/apply executor for SQLite.
- `tools/banner-scheduler/tests/scheduler.test.js`: Node tests for all pure JavaScript behavior.
- `tools/banner-scheduler/tests/Test-ApplyBannerPreset.ps1`: black-box executor test using only a temporary database copy.
- `tools/banner-scheduler/examples/all-collaboration.json`: small schema example generated from a test fixture rather than the runtime database.
- `tools/banner-scheduler/README.md`: opening, exporting, dry-run, and explicit apply instructions.

## Architecture

The browser layer and database layer communicate only through a versioned JSON preset. `scheduler.js` contains no DOM or filesystem code and exposes the same functions to Node CommonJS tests and the browser. `app.js` owns mutable UI state and renders from that state after each action.

The PowerShell executor treats the preset and `summon_pool.json` as untrusted inputs. It validates both before constructing SQL, uses a temporary desired-schedule table inside one transaction, and verifies the complete resulting schedule after the transaction.

## Source Data

The real source file is a UTF-8 JSON tuple:

```json
[
  "summon_pool",
  [
    {
      "id": 1,
      "nameEn": "THE FIRST DROP OF RAIN",
      "bannerFlag": 1,
      "type": 1,
      "priority": 999999
    }
  ]
]
```

The loader accepts only this tuple shape, requires the second element to be an array, and normalizes each item to the fields used by the page. Pool IDs must be positive, safe integers and unique. Invalid UTF-8/JSON, the wrong tuple name, missing arrays, duplicate IDs, or invalid fields produce a visible error and leave the previous valid page state unchanged.

## Pool Classification

Built-in presets use authoritative `bannerFlag` values:

| Preset | Rule |
| --- | --- |
| All pools | Every source pool |
| All collaborations | `bannerFlag === 5` |
| All reruns | `bannerFlag === 4 || bannerFlag === 6` |
| All activity pools | `bannerFlag === 2` |
| All limited pools | `bannerFlag === 3` |

The filter panel supports case-insensitive English-name text, exact or partial numeric ID, ID prefix, one or more `bannerFlag` values, one or more `type` values, and selected/unselected state. Filters change only the visible rows; they never silently remove selected pools.

## Page Interaction

After loading the source file, the page shows source count, visible count, and selected count. Each result row displays ID, English name, `bannerFlag`, `type`, and priority. The operator can select individual visible rows, select all visible rows, clear visible selections, or apply a built-in preset.

The selected queue is ordered independently of the source list. The operator can drag rows, move a row up or down, remove a row, or sort the queue by ID, priority, or current source order. Queue order is the schedule order used by batch generation.

Custom presets are stored in `localStorage` under a schema-specific key. A custom preset stores its name, selected pool IDs, queue order, filter controls, schedule mode, and schedule inputs. Loading one revalidates every stored ID against the currently loaded source and reports missing IDs rather than silently discarding them. Presets can be renamed and deleted.

The page imports previously exported schedule JSON, reconstructs the queue and schedule settings, and allows a fresh export. Import does not apply anything to SQLite.

## Schedule Modes

All generated times are Unix seconds. The page displays dates using `Asia/Shanghai` (`UTC+08:00`) and records that timezone in the preset. An offline time is exclusive.

### Simultaneous

Every selected pool receives the selected start date at `00:00:00+08:00` as `onlineTime` and `2147483647` as `offlineTime`. The default start date is the current Shanghai calendar date.

### Rotating Batches

The operator supplies positive integer `daysPerBatch` and `poolsPerBatch` values. For zero-based queue index `i`:

```text
batchIndex = floor(i / poolsPerBatch)
onlineTime = startDate + batchIndex * daysPerBatch
offlineTime = onlineTime + daysPerBatch
```

Pools in the same batch share the same interval. When a batch ends, its pools close and the next batch opens. Calendar-day arithmetic is anchored to Shanghai midnight so daylight-saving behavior on the host machine cannot change the result.

### Manual Dates

Each selected pool has explicit online and offline date inputs interpreted as Shanghai midnight. The offline date must be later than the online date. Reordering a manual queue does not rewrite its dates.

## Export Contract

The page exports UTF-8 JSON with this version 1 contract:

```json
{
  "schemaVersion": 1,
  "presetName": "All collaborations - rotation",
  "generatedAt": "2026-07-15T03:00:00.000Z",
  "source": {
    "fileName": "summon_pool.json",
    "poolCount": 210
  },
  "schedule": {
    "mode": "batch",
    "timezone": "Asia/Shanghai",
    "startDate": "2026-07-15",
    "daysPerBatch": 7,
    "poolsPerBatch": 2
  },
  "pools": [
    {
      "poolId": 12345,
      "order": 0,
      "onlineTime": 1784044800,
      "offlineTime": 1784649600
    }
  ]
}
```

`schedule.mode` is `simultaneous`, `batch`, or `manual`. Simultaneous mode records `daysPerBatch` and `poolsPerBatch` as `null`; manual mode records all automatic schedule inputs as `null`. The `pools` array must be non-empty, ordered contiguously from zero, contain unique IDs, and contain integer timestamps where `offlineTime > onlineTime`.

## Executor Contract

`Apply-BannerPreset.ps1` requires explicit `-PresetPath`, `-DatabasePath`, and `-SummonPoolJson` arguments. It resolves all paths, reads both JSON files as UTF-8, and validates:

- schema version, schedule mode, timezone, pool array, contiguous order, unique IDs, and integer timestamps;
- every preset pool ID exists exactly once in the current source file;
- the SQLite database contains `banner_schedule` and `user_summon_pools`;
- each selected schedule interval is valid.

Without `-Apply`, it prints the selected count and a deterministic table of pool IDs and UTC/Shanghai intervals, then exits before backup or SQL execution.

With `-Apply`, it creates a timestamped SQLite `.backup` beside the target database unless `-SkipBackup` is present. It then runs one `BEGIN IMMEDIATE` transaction that:

1. creates a temporary table containing the exact desired IDs and times;
2. removes `banner_schedule` rows not present in the preset;
3. upserts all desired `banner_schedule` rows with their generated times;
4. updates matching existing `user_summon_pools` rows to the same times;
5. commits atomically.

Afterward, it queries every `banner_schedule` row ordered by pool ID and compares IDs, online times, offline times, and row count with the preset. Any mismatch throws a non-zero error. Reapplying the same preset produces the same schedule state.

`-SkipBackup` exists for isolated automated tests and explicit operator use; documentation always shows normal applies with backup enabled.

`gameserver` startup calls `sync_banner_schedule` and upserts the banner IDs listed in `common/Config.toml`, which can overwrite those rows' database times. The executor does not modify that config. Documentation therefore defines the reproducible runtime order as: start or restart services, wait for gameserver readiness, apply the database preset, then reconnect or reload the client. A later gameserver restart requires reapplying the preset.

## Error Handling

The page uses one persistent status area with success, warning, and error states. Errors include the source filename and actionable reason without exposing a browser stack trace. Downloads are disabled until the source, selection, and schedule all validate.

The executor uses strict mode and terminating errors. It performs all validation before a backup or write. SQL values are created only from already validated integers, and paths are passed separately to SQLite rather than interpolated into SQL statements except for the escaped backup filename supported by `.backup`.

## Testing

JavaScript tests use Node's built-in `node:test` and cover:

- real tuple-shape parsing and rejection of malformed, duplicate, or invalid IDs;
- every built-in `bannerFlag` preset;
- combined text, ID, flag, type, and selection filters;
- stable queue ordering operations;
- Shanghai Unix conversion;
- simultaneous, multi-batch, and manual schedule generation;
- invalid empty selections, non-positive batch values, discontinuous order, duplicate IDs, and bad intervals;
- export/import round trips.

The PowerShell black-box test creates a minimal SQLite database in a temporary directory, generates a fixture source and preset, verifies dry run leaves the database hash unchanged, applies the preset twice with `-SkipBackup`, and checks exact IDs and timestamps after both runs. It never points at `runtime/db/sonetto.db`.

Manual browser verification loads the real 210-pool source, exercises every built-in preset and filter, reorders a queue, exports/imports each schedule mode, reloads a custom preset, and confirms there are no console errors. Executor verification uses the exported file only against a temporary database copy.

## Success Criteria

- The real UTF-8 source loads as 210 unique pools.
- All built-in preset counts match direct `bannerFlag` filtering.
- Queue order deterministically controls batch assignment.
- Exported JSON passes both browser import and executor validation.
- Default executor invocation performs no database write.
- Temporary-database apply is atomic, exact, backed by automated tests, and idempotent.
- No implementation or validation step modifies the current runtime database or any excluded system/client setting.
