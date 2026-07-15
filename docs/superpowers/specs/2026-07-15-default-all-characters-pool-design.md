# Default All-Characters Pool Design

## Goal

Create a standalone replacement `summon.json` in which the official standard character pool `ID 2` can produce every character marked `isOnline = "1"` by the international 3.6 `character.json`.

The deliverable also includes a reproducible generator and a black-box verification script. Building and testing the files must not modify the source data, runtime data, database, client, or server configuration.

## Scope

- Read the official international 3.6 `summon.json` and `character.json` as strict UTF-8 JSON tuples.
- Preserve every summon record except the five rarity records whose `id` is `2`.
- Rebuild the five `id = 2` records from all online character rows, grouped by character rarity.
- Write a complete replacement tuple named `summon`, suitable for use as `data/excel2json/summon.json` after a separate manual deployment.
- Produce deterministic output and verify exact membership, rarity counts, exclusions, and source-file immutability.

## Explicit Exclusions

- Do not deploy or copy the generated file into `sonetto-data`, `sonetto-rs/runtime`, or any active server data directory.
- Do not restart services or modify the current SQLite database.
- Do not change `summon_pool.json`, `common/Config.toml`, server source, generated Rust config, protocol files, proxy settings, `tcp_rules`, or the official client.
- Do not create a new pool ID. The official client has no metadata for arbitrary new IDs.
- Do not include characters with `isOnline != "1"`.
- Do not commit, push, or create a pull request.

## Deliverables

- `tools/banner-scheduler/presets/default-all-characters/summon.json`: generated full replacement file.
- `tools/banner-scheduler/New-DefaultAllCharactersSummon.ps1`: reproducible PowerShell 5.1 generator.
- `tools/banner-scheduler/tests/Test-DefaultAllCharactersSummon.ps1`: black-box generator and output verifier.
- `tools/banner-scheduler/presets/default-all-characters/README.md`: contents, rates, safe deployment order, and rollback instructions.

## Source Contract

Both source files use the tuple format:

```json
[
  "table_name",
  [
    { "id": 1 }
  ]
]
```

The generator requires:

- tuple name `summon` for the summon source;
- tuple name `character` for the character source;
- a non-empty record array in each source;
- exactly five standard-pool source rows with `id = 2`, one for each rarity `1` through `5`;
- positive, unique character IDs;
- integer character rarities from `1` through `5`;
- at least one online character for every rarity.

Strict UTF-8 decoding and JSON parsing occur before any output path is created or replaced.

## Character Membership

An eligible character is any `character.json` row where:

```text
isOnline == "1"
```

No `heroType` restriction is applied. This intentionally includes standard, limited, collaboration, and special playable characters represented by the current source data.

For international 3.6, the expected membership is 120 unique characters:

| Character `rare` / summon `rare` | Display rarity | Count |
| --- | --- | ---: |
| 5 | 6-star | 61 |
| 4 | 5-star | 29 |
| 3 | 4-star | 17 |
| 2 | 3-star | 11 |
| 1 | 2-star | 2 |

The output explicitly excludes Schneider `3029` and Machine D III `9998` because their source rows have `isOnline = "0"`.

## Generated Pool Records

The generator removes the existing five `id = 2` rows and inserts five replacement rows at the position of the first removed row. Rows remain ordered by rarity `5`, `4`, `3`, `2`, `1`, matching the official file.

Each replacement has this shape:

```json
{
  "id": 2,
  "rare": 5,
  "summonId": "3003#3004#...",
  "luckyBagId": ""
}
```

Within each rarity, IDs are sorted numerically and joined by `#`. Every eligible character appears exactly once across the five rows. All non-pool-2 records retain the same property values and ordering as the source records.

## Probability and Pity Behavior

The generated file changes only candidate membership. It does not change rates or pity logic.

Pool `2` remains the existing `type = 2` standard pool from `summon_pool.json`, with an empty `upWeight`. The current gameserver therefore keeps:

- base 6-star probability of 1.5%;
- 5-star probability of 8.5%;
- 4-star probability of 40%;
- 3-star probability of 45%;
- 2-star probability of 5%;
- existing 6-star pity progression and 70-pull hard pity;
- the existing ten-pull 5-star-or-better guarantee.

Characters within a selected rarity are chosen uniformly because the standard pool has no UP list.

## Generator Interface

`New-DefaultAllCharactersSummon.ps1` accepts mandatory paths:

```powershell
-SummonJson <official summon.json>
-CharacterJson <official character.json>
-OutputPath <standalone output summon.json>
```

The output path must differ from both input paths. The script refuses to replace an existing output unless `-Force` is explicitly provided.

The generator:

1. resolves and validates both inputs;
2. validates source tuple structure and pool-2 baseline rows;
3. derives sorted online-character IDs per rarity;
4. constructs the replacement rows and complete output tuple;
5. serializes deterministic UTF-8 JSON without a BOM;
6. writes to a temporary sibling file;
7. parses and verifies that temporary file;
8. atomically moves it to the requested output path;
9. prints source/output counts and rarity counts.

On any error, the final output remains absent or unchanged and the temporary file is removed.

## Verification

The black-box test uses the real international 3.6 source files and a unique temporary output path. It verifies:

- both source hashes remain unchanged;
- the generated tuple is named `summon` and has the same total row count as the source;
- all non-pool-2 rows are semantically identical and remain in order;
- pool `2` has exactly five rows with rarities `5` through `1`;
- the union contains exactly the 120 online character IDs with no duplicates or unknown IDs;
- each rarity list exactly matches direct filtering of `character.json`;
- IDs `3029` and `9998` are absent;
- rerunning with `-Force` produces the same SHA-256 hash;
- writing to either source path is rejected.

The checked-in generated file is independently compared with a freshly generated temporary file byte for byte.

## Deployment Boundary

The generated file is not applied by this task. Its README documents a later manual deployment:

1. stop or prepare to restart `gameserver`;
2. back up the active `data/excel2json/summon.json`;
3. copy the generated file into that exact active path under the filename `summon.json`;
4. restart `gameserver`, because game config is loaded at process startup;
5. enter the official standard pool `2` and perform controlled test pulls;
6. restore the backup and restart to roll back.

The client pool-detail page can continue showing official static descriptions. Actual draw membership is controlled by the server-side generated `summon.json`.

## Success Criteria

- The standalone file contains 120 unique online characters in pool `2` with counts `61/29/17/11/2` for rarity `5/4/3/2/1`.
- The file contains every original non-pool-2 summon record unchanged in value and order.
- The generator is deterministic, source-safe, and PowerShell 5.1 compatible.
- The test suite proves the source and current runtime/database are not modified.
- No generated file is deployed during implementation or verification.
