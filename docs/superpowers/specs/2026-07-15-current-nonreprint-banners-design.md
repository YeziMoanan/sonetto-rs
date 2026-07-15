# Current non-reprint banner script design

## Goal

Provide a replay script for the current international 3.6 data set that keeps only the permanent pool, newbie pool, and current-version non-reprint pools.

## Selected pools

- Permanent/newbie: `1`, `2`
- Current non-reprint: `34111`, `34121`, `34131`, `34141`, `34151`, `34161`, `34191`
- Explicitly excluded reprints: `34171`, `34181`

## Architecture

Extend the existing database replay script with an optional explicit pool-ID list. Add a small wrapper that owns the nine-ID policy and delegates dry-run, backup, transaction, and verification behavior to the existing script.

## Safety

- Do not apply the new policy to the current database in this task.
- Test against a temporary SQLite backup only.
- Finish with a dry run and independently confirm that the current database still contains 210 active schedules.
- Do not change activity data, server code, proxy settings, protocol files, or the official client.

