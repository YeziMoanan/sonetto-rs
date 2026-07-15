# Banner Scheduler Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a local page that turns the international 3.6 summon-pool source into editable schedule presets and a safe, dry-run-by-default PowerShell executor for Sonetto SQLite databases.

**Architecture:** Pure scheduling behavior lives in one dependency-free JavaScript module shared by Node tests and the browser. A thin browser adapter manages DOM state, local presets, and JSON files. A separate PowerShell executor validates the exported JSON and applies its exact schedule in one SQLite transaction only when `-Apply` is supplied.

**Tech Stack:** HTML5, CSS, dependency-free JavaScript, Node 22 `node:test`, PowerShell 5.1, SQLite CLI

**Repository constraint:** Do not commit, push, or create a PR. Replace commit steps with `git diff --check` and focused test checkpoints. Do not apply any preset to `runtime/db/sonetto.db` while implementing this plan.

---

## File Map

- Create `tools/banner-scheduler/scheduler.js` for pure parsing, filtering, queue, scheduling, and preset-contract logic.
- Create `tools/banner-scheduler/tests/scheduler.test.js` for Node behavior tests.
- Create `tools/banner-scheduler/index.html` for page structure and accessible controls.
- Create `tools/banner-scheduler/styles.css` for responsive presentation.
- Create `tools/banner-scheduler/app.js` for browser event wiring and state rendering.
- Create `tools/banner-scheduler/Apply-BannerPreset.ps1` for validation, dry run, backup, transaction, and verification.
- Create `tools/banner-scheduler/tests/Test-ApplyBannerPreset.ps1` for temporary-database black-box tests.
- Create `tools/banner-scheduler/examples/all-collaboration.json` for schema demonstration.
- Create `tools/banner-scheduler/README.md` for safe operation and restart ordering.

## Task 1: Source Parsing and Built-in Presets

**Files:**
- Create: `tools/banner-scheduler/tests/scheduler.test.js`
- Create: `tools/banner-scheduler/scheduler.js`

- [ ] **Step 1: Write failing parser and preset tests**

Start the test file with Node's built-in runner and a wished-for CommonJS API:

```js
const test = require("node:test");
const assert = require("node:assert/strict");
const scheduler = require("../scheduler.js");

const sourcePools = [
  { id: 1, nameEn: "Permanent", bannerFlag: 0, type: 1, priority: 99 },
  { id: 20, nameEn: "Activity", bannerFlag: 2, type: 3, priority: 20 },
  { id: 30, nameEn: "Limited", bannerFlag: 3, type: 3, priority: 30 },
  { id: 40, nameEn: "Rerun Four", bannerFlag: 4, type: 4, priority: 40 },
  { id: 50, nameEn: "Collaboration", bannerFlag: 5, type: 5, priority: 50 },
  { id: 60, nameEn: "Rerun Six", bannerFlag: 6, type: 6, priority: 60 },
];

function sourceJson(pools = sourcePools) {
  return JSON.stringify(["summon_pool", pools]);
}

test("parses and normalizes the summon_pool tuple", () => {
  assert.deepEqual(scheduler.parseSourceJson(sourceJson()), sourcePools);
});

test("rejects duplicate and invalid pool IDs", () => {
  assert.throws(() => scheduler.parseSourceJson(sourceJson([{ id: 1 }, { id: 1 }])), /duplicate/i);
  assert.throws(() => scheduler.parseSourceJson(sourceJson([{ id: 0 }])), /positive/i);
});

test("selects every built-in bannerFlag preset", () => {
  assert.deepEqual(scheduler.selectPreset(sourcePools, "all"), [1, 20, 30, 40, 50, 60]);
  assert.deepEqual(scheduler.selectPreset(sourcePools, "collaboration"), [50]);
  assert.deepEqual(scheduler.selectPreset(sourcePools, "rerun"), [40, 60]);
  assert.deepEqual(scheduler.selectPreset(sourcePools, "activity"), [20]);
  assert.deepEqual(scheduler.selectPreset(sourcePools, "limited"), [30]);
});
```

- [ ] **Step 2: Run RED**

Run `node --test tools/banner-scheduler/tests/scheduler.test.js`.

Expected: FAIL with `Cannot find module '../scheduler.js'`.

- [ ] **Step 3: Implement minimal parser and preset selection**

Create a UMD-style module that assigns the same API to `module.exports` and `window.BannerScheduler`. Implement:

```js
const PRESET_RULES = Object.freeze({
  all: () => true,
  collaboration: (pool) => pool.bannerFlag === 5,
  rerun: (pool) => pool.bannerFlag === 4 || pool.bannerFlag === 6,
  activity: (pool) => pool.bannerFlag === 2,
  limited: (pool) => pool.bannerFlag === 3,
});

function parseSourceJson(text) {
  const value = JSON.parse(text);
  if (!Array.isArray(value) || value[0] !== "summon_pool" || !Array.isArray(value[1])) {
    throw new Error("Source must be a summon_pool tuple");
  }
  const seen = new Set();
  return value[1].map((pool) => {
    if (!Number.isSafeInteger(pool.id) || pool.id <= 0) throw new Error("Pool ID must be a positive safe integer");
    if (seen.has(pool.id)) throw new Error(`Duplicate pool ID: ${pool.id}`);
    seen.add(pool.id);
    return {
      id: pool.id,
      nameEn: typeof pool.nameEn === "string" ? pool.nameEn : "",
      bannerFlag: Number.isSafeInteger(pool.bannerFlag) ? pool.bannerFlag : 0,
      type: Number.isSafeInteger(pool.type) ? pool.type : 0,
      priority: Number.isSafeInteger(pool.priority) ? pool.priority : 0,
    };
  });
}

function selectPreset(pools, key) {
  const rule = PRESET_RULES[key];
  if (!rule) throw new Error(`Unknown preset: ${key}`);
  return pools.filter(rule).map((pool) => pool.id);
}
```

- [ ] **Step 4: Run GREEN and checkpoint**

Run the focused Node test and `git diff --check -- tools/banner-scheduler`. Expected: all tests pass and no whitespace errors.

## Task 2: Filtering and Queue Operations

**Files:**
- Modify: `tools/banner-scheduler/tests/scheduler.test.js`
- Modify: `tools/banner-scheduler/scheduler.js`

- [ ] **Step 1: Add failing behavior tests**

Add tests for combined filters and stable queue changes:

```js
test("combines name, ID prefix, flags, types, and selection filters", () => {
  const visible = scheduler.filterPools(sourcePools, {
    query: "activity",
    idPrefix: "2",
    bannerFlags: [2],
    types: [3],
    selection: "selected",
  }, new Set([20, 50]));
  assert.deepEqual(visible.map((pool) => pool.id), [20]);
});

test("moves and sorts queue IDs without changing membership", () => {
  assert.deepEqual(scheduler.moveQueueItem([30, 20, 10], 20, -1), [20, 30, 10]);
  const pools = [{ id: 10, priority: 3 }, { id: 20, priority: 1 }, { id: 30, priority: 2 }];
  assert.deepEqual(scheduler.sortQueue([30, 10, 20], pools, "priority"), [20, 30, 10]);
});
```

- [ ] **Step 2: Run RED**

Run the focused Node test. Expected: FAIL because `filterPools`, `moveQueueItem`, and `sortQueue` are undefined.

- [ ] **Step 3: Implement minimal filter and queue functions**

Implement exact intersection semantics. `query` matches lowercase English name or a partial decimal ID; `idPrefix` matches the decimal prefix; empty flag/type arrays mean no restriction. `selection` accepts `all`, `selected`, or `unselected`. Queue sorting accepts `source`, `id`, or `priority`, uses source order as a deterministic tie-breaker, and rejects unknown IDs.

- [ ] **Step 4: Run GREEN and checkpoint**

Run the focused test and `git diff --check -- tools/banner-scheduler`.

## Task 3: Schedule Generation and Export Validation

**Files:**
- Modify: `tools/banner-scheduler/tests/scheduler.test.js`
- Modify: `tools/banner-scheduler/scheduler.js`

- [ ] **Step 1: Add failing Shanghai-time and schedule tests**

```js
test("converts Shanghai calendar dates independently of host timezone", () => {
  assert.equal(scheduler.shanghaiDateToUnix("2026-07-15"), 1784044800);
  assert.throws(() => scheduler.shanghaiDateToUnix("2026-02-30"), /date/i);
});

test("generates simultaneous and rotating batch schedules", () => {
  assert.deepEqual(scheduler.buildSchedule([50, 40], {
    mode: "simultaneous", startDate: "2026-07-15",
  }), [
    { poolId: 50, order: 0, onlineTime: 1784044800, offlineTime: 2147483647 },
    { poolId: 40, order: 1, onlineTime: 1784044800, offlineTime: 2147483647 },
  ]);

  const batch = scheduler.buildSchedule([50, 40, 30], {
    mode: "batch", startDate: "2026-07-15", daysPerBatch: 7, poolsPerBatch: 2,
  });
  assert.deepEqual(batch.map(({ onlineTime, offlineTime }) => [onlineTime, offlineTime]), [
    [1784044800, 1784649600],
    [1784044800, 1784649600],
    [1784649600, 1785254400],
  ]);
});

test("keeps manual dates attached to pool IDs after reordering", () => {
  const pools = scheduler.buildSchedule([40, 50], {
    mode: "manual",
    manualDates: {
      50: { onlineDate: "2026-07-15", offlineDate: "2026-07-16" },
      40: { onlineDate: "2026-08-01", offlineDate: "2026-08-03" },
    },
  });
  assert.equal(pools[0].poolId, 40);
  assert.equal(pools[0].onlineTime, scheduler.shanghaiDateToUnix("2026-08-01"));
});
```

- [ ] **Step 2: Run RED**

Run the focused test. Expected: FAIL because schedule functions are undefined.

- [ ] **Step 3: Implement schedule functions**

Validate calendar components before calculating `Date.UTC(year, month - 1, day, -8, 0, 0) / 1000`. Require a non-empty unique ordered ID list. Require positive safe integers for both batch values. For manual schedules, require one date pair per ID and `offlineTime > onlineTime`.

- [ ] **Step 4: Add failing export/import round-trip tests**

Test a deterministic `createPreset({ presetName, sourceFileName, sourcePoolCount, schedule, pools, generatedAt })` and `parsePresetJson(text, availablePools)` API. Assert the full version 1 object, successful round trip, and rejection of duplicate IDs, discontinuous order, unknown IDs, bad timezone, unsupported schema version, non-integer timestamps, and empty pools.

- [ ] **Step 5: Run RED, implement validation, then run GREEN**

`createPreset` must emit only the documented fields. `parsePresetJson` must return a normalized copy and never mutate parsed input. Run all Node tests and diff checks.

## Task 4: Build the Static Page Shell

**Files:**
- Create: `tools/banner-scheduler/tests/page-contract.test.js`
- Create: `tools/banner-scheduler/index.html`
- Create: `tools/banner-scheduler/styles.css`

- [ ] **Step 1: Write a failing page contract test**

Use `fs.readFileSync` to load the not-yet-created page, then assert the required contract after creation: one `main`, a file input `#source-file`, status `#status`, filters `#filter-query`, `#filter-id-prefix`, `#filter-flags`, `#filter-types`, `#filter-selection`, preset buttons with `data-preset`, pool list `#pool-list`, selected queue `#selected-queue`, schedule controls, JSON import `#preset-file`, and export button `#export-preset`. Also assert local `styles.css`, `scheduler.js`, and deferred `app.js` script references with no remote URLs.

- [ ] **Step 2: Run RED**

Run `node --test tools/banner-scheduler/tests/page-contract.test.js`. Expected: FAIL with `ENOENT` for `index.html`.

- [ ] **Step 3: Create semantic HTML and responsive CSS**

Use a header/status strip, a compact source/filter sidebar, a scrollable source table, a selected-queue panel, and a schedule/export panel. Preserve visible focus, keyboard-operable buttons, labels, live status announcements, high contrast, and layouts for both wide desktop and narrow windows. Use only local assets and system fonts.

- [ ] **Step 4: Run GREEN and visual syntax checks**

Run both Node test files and `git diff --check -- tools/banner-scheduler`.

## Task 5: Wire Browser State and Custom Presets

**Files:**
- Modify: `tools/banner-scheduler/tests/scheduler.test.js`
- Create: `tools/banner-scheduler/app.js`

- [ ] **Step 1: Add failing UI-state serialization tests to the pure module**

Add `normalizeWorkspace` and `restoreWorkspace` tests covering selection order, filter values, schedule inputs, missing source IDs, and manual dates. The saved shape must contain no raw source rows and use key `banner-scheduler.workspace.v1`.

- [ ] **Step 2: Run RED, implement pure workspace helpers, run GREEN**

Keep storage serialization in `scheduler.js`; inject JSON text into `restoreWorkspace` so Node tests require no browser mock.

- [ ] **Step 3: Implement the thin DOM adapter**

`app.js` must:

- decode selected files with `file.text()` and preserve the previous state on parse failure;
- populate distinct flag/type controls from the current source;
- render filtered source rows and the ordered selected queue;
- support visible select/clear, built-in presets, remove, up/down, HTML drag reorder, and deterministic sorting;
- toggle simultaneous/batch/manual inputs and render the generated interval preview;
- save, load, rename, and delete custom workspaces in `localStorage`;
- import exported JSON through `parsePresetJson` and reconstruct editable state;
- create UTF-8 JSON downloads with `Blob` and revoke object URLs;
- disable export until all validation passes;
- report all errors in `#status` without stack traces.

- [ ] **Step 4: Run static checks**

Run `node --check tools/banner-scheduler/app.js`, all Node tests, and diff checks.

## Task 6: PowerShell Executor via Black-box TDD

**Files:**
- Create: `tools/banner-scheduler/tests/Test-ApplyBannerPreset.ps1`
- Create: `tools/banner-scheduler/Apply-BannerPreset.ps1`

- [ ] **Step 1: Write the failing black-box test**

The test creates a unique temporary directory, writes a UTF-8 `summon_pool.json` tuple with IDs `20`, `40`, and `50`, writes a schema version 1 preset scheduling `50` and `40`, and creates minimal tables matching migrations 021 and 042. It records the database SHA-256 hash, invokes the missing executor without `-Apply`, requires the same hash, applies with `-Apply -SkipBackup`, and queries:

```sql
SELECT pool_id || '|' || online_time || '|' || offline_time
FROM banner_schedule
ORDER BY pool_id;
```

Expected rows are exactly the two preset rows. Invoke apply a second time and require identical results. Add negative cases for unknown IDs, duplicate IDs, invalid order, invalid interval, and missing tables. Cleanup runs in `finally`.

- [ ] **Step 2: Run RED**

Run:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools\banner-scheduler\tests\Test-ApplyBannerPreset.ps1
```

Expected: FAIL because `Apply-BannerPreset.ps1` does not exist.

- [ ] **Step 3: Implement strict input validation and dry run**

Define mandatory `PresetPath`, `DatabasePath`, and `SummonPoolJson`, plus `Apply` and `SkipBackup` switches. Use `Set-StrictMode -Version Latest`, terminating errors, strict UTF-8 decoding, and `ConvertFrom-Json`. Require version `1`, timezone `Asia/Shanghai`, valid modes, a non-empty array, contiguous order, unique positive integer IDs, timestamps between `0` and `2147483647`, valid intervals, known source IDs, and both database tables.

Dry run prints a stable `pool_id | online_time | offline_time` table and returns before backup or mutation.

- [ ] **Step 4: Implement backup, transaction, and exact verification**

Unless skipped, create `.banner-preset.<UTC timestamp>.bak` with SQLite `.backup`. In a single transaction, fill a temporary table `(pool_id PRIMARY KEY, online_time, offline_time)`, delete unselected schedule rows, upsert desired schedule rows, and update matching `user_summon_pools`. Query all final schedule rows and compare every field against the normalized preset.

- [ ] **Step 5: Run GREEN and add backup coverage**

Run the black-box test. Add one temporary-database invocation without `-SkipBackup` and assert exactly one non-empty backup. Run again and require all cases to pass.

## Task 7: Example and Safe Operations Guide

**Files:**
- Create: `tools/banner-scheduler/examples/all-collaboration.json`
- Create: `tools/banner-scheduler/README.md`

- [ ] **Step 1: Add the versioned example**

Use the documented schema with one collaboration fixture pool, contiguous order, Shanghai timezone, and valid integer times. Verify it with `parsePresetJson` in a new Node test.

- [ ] **Step 2: Write operation instructions**

Document direct `index.html` opening, real source selection, custom preset storage boundaries, export/import, executor dry run, and explicit apply:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools\banner-scheduler\Apply-BannerPreset.ps1 `
  -PresetPath C:\path\to\preset.json `
  -DatabasePath runtime\db\sonetto.db `
  -SummonPoolJson D:\python-tools\重返未来1999\sonetto-data\excel2json\summon_pool.json
```

Show the same command with `-Apply` only in a separate section. State that `gameserver` startup upserts IDs from `common/Config.toml` and can overwrite those rows' times; therefore the reproducible order is: start/restart services, wait for port `23301`, then run the preset executor, then reconnect/reload the client. Do not suggest editing `Config.toml` or applying during implementation.

- [ ] **Step 3: Run documentation and example checks**

Run Node tests, placeholder scan, and `git diff --check -- tools/banner-scheduler docs/superpowers`.

## Task 8: Real-source and Browser Verification

**Files:**
- Read only: `D:\python-tools\重返未来1999\sonetto-data\excel2json\summon_pool.json`
- Runtime temporary files only: system temp directory

- [ ] **Step 1: Verify the real source without writing the runtime database**

Use Node to parse the real UTF-8 file through `parseSourceJson`. Require exactly 210 unique pools and compare each built-in preset count with a direct `bannerFlag` filter.

- [ ] **Step 2: Run the executor against a temporary database only**

Copy the runtime database into a unique temporary directory using SQLite `.backup`, dry-run a page-exported preset, apply it to the copy, reapply it, and independently query exact IDs/times. Delete the temporary directory afterward. Never pass the original runtime database as `-DatabasePath`.

- [ ] **Step 3: Perform browser interaction verification**

Open the local page, load the real source, test all built-in presets, combined filters, queue movement/dragging, simultaneous and batch previews, manual validation, custom preset reload, and export/import round trip. Check responsive layout and browser console for errors.

- [ ] **Step 4: Run the complete verification suite**

Run:

```powershell
node --test tools/banner-scheduler/tests/*.test.js
powershell -NoProfile -ExecutionPolicy Bypass -File tools\banner-scheduler\tests\Test-ApplyBannerPreset.ps1
node --check tools/banner-scheduler/scheduler.js
node --check tools/banner-scheduler/app.js
git diff --check -- tools/banner-scheduler docs/superpowers
```

Expected: every test passes, syntax checks return zero, diff check is clean, and the current runtime database timestamp/hash remains unchanged by this plan.
