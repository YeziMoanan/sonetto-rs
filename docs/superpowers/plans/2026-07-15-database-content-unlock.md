# Database-only Summon Unlock Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Open all summon pools in the current SQLite database and provide a safe, idempotent replay script.

**Architecture:** A PowerShell replay script discovers unique pool IDs from `summon_pool.json`, uses SQLite's online backup command, and applies one transaction to `banner_schedule` and existing `user_summon_pools`. A black-box test runs the script twice against a temporary database copy and verifies exact counts. Activity availability remains untouched because it is not database-backed.

**Tech Stack:** PowerShell 5+, SQLite CLI, Sonetto SQLite schema

---

### Task 1: Add the black-box replay test

**Files:**
- Create: `scripts/Test-EnableAllBanners.ps1`

- [ ] **Step 1: Write the failing test**

Create a PowerShell test that resolves the source database and `summon_pool.json`, creates a SQLite backup in a temporary directory, invokes `scripts/Enable-AllBanners.ps1 -Apply -SkipBackup`, and queries:

```sql
SELECT COUNT(*),
       SUM(CASE WHEN online_time <= strftime('%s','now')
                 AND offline_time > strftime('%s','now') THEN 1 ELSE 0 END)
FROM banner_schedule;
```

The test must assert that both values equal the unique IDs matched by `"id"\s*:\s*(\d+)` in `summon_pool.json`, invoke the replay script a second time, and assert the same counts again.

- [ ] **Step 2: Run the test to verify RED**

Run:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\Test-EnableAllBanners.ps1 `
  -SourceDatabase runtime\db\sonetto.db `
  -SummonPoolJson D:\python-tools\重返未来1999\sonetto-data\excel2json\summon_pool.json
```

Expected: FAIL because `scripts/Enable-AllBanners.ps1` does not exist.

### Task 2: Implement the replay script

**Files:**
- Create: `scripts/Enable-AllBanners.ps1`

- [ ] **Step 1: Implement discovery and dry-run behavior**

Resolve both input paths, require `sqlite3.exe`, discover sorted unique integer IDs with the exact regex from Task 1, reject an empty result, and exit without a write unless `-Apply` is present.

- [ ] **Step 2: Implement consistent backup and transaction**

Unless `-SkipBackup` is present, create a name such as `sonetto.db.banner-open.20260715T010203Z.bak` using SQLite's `.backup` command. Build the transaction from the discovered IDs and current Unix time:

```powershell
$values = ($poolIds | ForEach-Object { "($_)" }) -join ","
$now = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
$sql = @"
BEGIN IMMEDIATE;
CREATE TEMP TABLE desired_banner_schedule(pool_id INTEGER PRIMARY KEY);
INSERT INTO desired_banner_schedule(pool_id) VALUES $values;
DELETE FROM banner_schedule
WHERE pool_id NOT IN (SELECT pool_id FROM desired_banner_schedule);
INSERT INTO banner_schedule(pool_id, online_time, offline_time, created_at, updated_at)
SELECT pool_id, 0, 2147483647, $now, $now
FROM desired_banner_schedule
WHERE 1
ON CONFLICT(pool_id) DO UPDATE SET
    online_time = excluded.online_time,
    offline_time = excluded.offline_time,
    updated_at = excluded.updated_at;
UPDATE user_summon_pools
SET online_time = 0,
    offline_time = 2147483647,
    updated_at = $now
WHERE pool_id IN (SELECT pool_id FROM desired_banner_schedule);
COMMIT;
"@
```

- [ ] **Step 3: Implement post-write verification**

Query total and currently active schedules after the transaction. Throw if either differs from the discovered unique pool count. Print the backup path and verified count on success.

- [ ] **Step 4: Run the black-box test to verify GREEN**

Run the Task 1 command. Expected: PASS with 210 scheduled and active pools after both runs.

### Task 3: Apply to the current database

**Files:**
- Modify at runtime: `runtime/db/sonetto.db`
- Create at runtime: `runtime/db/sonetto.db.banner-open.YYYYMMDDTHHMMSSZ.bak`

- [ ] **Step 1: Run a dry run**

Run:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\Enable-AllBanners.ps1 `
  -DatabasePath runtime\db\sonetto.db `
  -SummonPoolJson D:\python-tools\重返未来1999\sonetto-data\excel2json\summon_pool.json
```

Expected: 210 unique pool IDs discovered and no database write.

- [ ] **Step 2: Apply the database patch**

Run the same command with `-Apply`. Expected: a timestamped SQLite backup and verification of 210 scheduled and active pools.

- [ ] **Step 3: Verify independently**

Run:

```powershell
sqlite3 runtime\db\sonetto.db "SELECT COUNT(*), SUM(CASE WHEN online_time <= strftime('%s','now') AND offline_time > strftime('%s','now') THEN 1 ELSE 0 END) FROM banner_schedule;"
```

Expected: `210|210`.

### Task 4: Runtime validation

**Files:**
- Read: `gameserver-active.log`

- [ ] **Step 1: Restart only the gameserver after preserving the current client state**

Use the existing detached append-log launch pattern. Confirm `127.0.0.1:23301` is listening.

- [ ] **Step 2: Reconnect the client and inspect summon info**

Open the summon screen, confirm non-permanent pools are visible, and perform one test pull from a non-permanent pool only if the client remains connected.

- [ ] **Step 3: Inspect the log**

Confirm `GetSummonInfoCmd` and the selected `SummonCmd` complete without `Dispatch error`, unknown raw IDs, or disconnects. Record any newly exposed command as a separate compatibility task.
