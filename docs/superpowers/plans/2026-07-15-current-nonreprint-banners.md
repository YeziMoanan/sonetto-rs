# Current Non-Reprint Banner Script Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Generate and verify a replay script that selects exactly nine current non-reprint summon pools without changing the current database.

**Architecture:** Add explicit ID selection to the existing replay engine, then add a policy wrapper with the nine approved IDs. Verify exact IDs and idempotency on a temporary database and run only dry-run mode against the current database.

**Tech Stack:** PowerShell 5+, SQLite CLI

---

### Task 1: Add a failing black-box test

**Files:**
- Create: `scripts/Test-EnableCurrentNonReprintBanners.ps1`

- [ ] Create a temporary SQLite backup, invoke the missing wrapper twice with `-Apply -SkipBackup`, and assert that `banner_schedule` contains exactly `1,2,34111,34121,34131,34141,34151,34161,34191` after each run.
- [ ] Run the test and confirm it fails only because `Enable-CurrentNonReprintBanners.ps1` is missing.

### Task 2: Add explicit selection and wrapper

**Files:**
- Modify: `scripts/Enable-AllBanners.ps1`
- Create: `scripts/Enable-CurrentNonReprintBanners.ps1`

- [ ] Add optional `-IncludePoolId` input to the existing script, validate every requested ID exists in `summon_pool.json`, and use the selected IDs for transaction and verification.
- [ ] Add a wrapper with the exact nine approved IDs and forward `-Apply` and `-SkipBackup` to the existing script.

### Task 3: Verify both policies

**Files:**
- Test: `scripts/Test-EnableAllBanners.ps1`
- Test: `scripts/Test-EnableCurrentNonReprintBanners.ps1`

- [ ] Run the existing all-banner test; expect `210/210` twice.
- [ ] Run the new non-reprint test; expect the exact nine IDs twice.
- [ ] Run the new wrapper without `-Apply`; expect a dry-run report for nine IDs.
- [ ] Query the current database independently; expect it to remain `210/210`.

