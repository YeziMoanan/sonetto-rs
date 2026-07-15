# Runtime Log Command Remediation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复最新运行日志中仍会断开连接的 `DestinyRankUpCmd`、`GetActivityInfosWithParamCmd` 和 `GetFightCardDeckInfoCmd`。

**Architecture:** 沿用现有命令调度和强类型 protobuf 回复模式。命轨升阶更新独立 3.8 数据库并推送英雄状态；活动查询复用完整活动静态数据后按请求 ID 过滤；尚无持久化来源的战斗卡组返回协议允许的空列表。

**Tech Stack:** Rust 2024、Tokio、prost、SQLx/SQLite、现有 gameserver 集成测试工具。

---

### Task 1: Reproduce all three disconnects

**Files:**
- Modify: `gameserver/src/network/handler.rs`

- [ ] Add an integration test that dispatches `DestinyRankUpCmd` for a migrated in-memory hero and expects `destiny_rank=1`, `destiny_level=1`.
- [ ] Run `cargo test -p gameserver destiny_rank_up_unlocks_first_rank_for_zero_rank_hero -- --exact` and verify it fails with `Unhandled Cmd: DestinyRankUpCmd`.
- [ ] Add an integration test that dispatches `GetActivityInfosWithParamCmd` for IDs `13316` and `12301` and expects exactly those activities.
- [ ] Run the targeted test and verify it fails with `Unhandled Cmd: GetActivityInfosWithParamCmd`.
- [ ] Add an integration test that dispatches `GetFightCardDeckInfoCmd` and expects an empty successful `GetFightCardDeckInfoReply`.
- [ ] Run the targeted test and verify it fails with `Unhandled Cmd: GetFightCardDeckInfoCmd`.

### Task 2: Implement destiny rank-up

**Files:**
- Create: `gameserver/src/handlers/destiny_stone/destiny_rank_up.rs`
- Modify: `gameserver/src/handlers/destiny_stone/mod.rs`
- Modify: `database/src/models/game/heros.rs`
- Modify: `gameserver/src/network/handler.rs`

- [ ] Add `HeroModel::destiny_rank_up`, scoped by hero and user, which increments rank and normalizes level to at least `1`.
- [ ] Decode `DestinyRankUpRequest`, update the logged-in user's hero, send `HeroUpdatePush`, then send `DestinyRankUpReply`.
- [ ] Register `CmdId::DestinyRankUpCmd` and rerun its targeted test until green.

### Task 3: Implement filtered activities and card-deck reply

**Files:**
- Modify: `gameserver/src/handlers/events.rs`
- Create: `gameserver/src/handlers/fight/get_fight_card_deck_info.rs`
- Modify: `gameserver/src/handlers/fight/mod.rs`
- Modify: `gameserver/src/network/handler.rs`

- [ ] Decode `GetActivityInfosWithParamRequest`, load the existing complete activity reply, retain only requested IDs, and send `GetActivityInfosWithParamReply`.
- [ ] Decode `GetFightCardDeckInfoRequest` and send a successful reply with `deck_infos: []`.
- [ ] Register both commands and rerun both targeted tests until green.

### Task 4: Verify and commit

**Files:**
- Verify all modified files above.

- [ ] Run `cargo fmt --check`.
- [ ] Run `cargo check --workspace`.
- [ ] Run `cargo test --workspace`.
- [ ] Confirm the original 3.6 worktree was not modified by this remediation.
- [ ] Commit the 3.8 changes with `fix: handle commands found in runtime logs`.
