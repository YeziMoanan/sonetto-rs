# Database-only content unlock design

## Goal

Open every summon pool represented by the current `summon_pool.json` in the current Sonetto database, then provide an idempotent PowerShell script that can reproduce the same database state later.

## Scope

- Read pool IDs from `sonetto-data/excel2json/summon_pool.json`.
- Back up the target SQLite database before changing it.
- Upsert all pool IDs into `banner_schedule` with a continuously active time range.
- Update existing matching rows in `user_summon_pools` to the same active time range.
- Run the changes in one SQLite transaction.
- Verify that the active schedule exactly matches the unique pool IDs discovered from the source data.

## Explicit exclusions

- Do not change server source code, generated protocol files, proxy settings, `tcp_rules`, or the official client DLL.
- Do not commit, push, or create a pull request.
- Do not modify `assets/static/activity/activity_infos.json` in this database-only phase.
- Do not claim that activities are fully open: `GetActivityInfos` reads the static JSON file directly, and no activity schedule table exists in the database.

## Script behavior

The replay script accepts explicit database and summon-pool paths. Without `-Apply` it performs a dry run and reports the discovered pool count. With `-Apply`, it creates a timestamped backup, applies the transaction, and fails if the post-write schedule count or active count differs from the discovered unique pool count.

The operation is idempotent: rerunning it updates the same primary-key rows rather than creating duplicates.

## Verification

1. Run the script without `-Apply`; expect 210 unique pool IDs and no database change.
2. Apply the script to a temporary database copy; expect 210 scheduled and active pools.
3. Apply the script to the current database; expect the same counts and a timestamped backup.
4. Restart the gameserver and request summon info from the client; confirm all returned pool IDs remain connected and at least one non-permanent pool can be opened.

