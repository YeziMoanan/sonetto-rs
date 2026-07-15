# 限时/活动副本与商城开放修复 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将限时副本、活动副本、活动商城、常驻商城和充值商城从“静态显示/部分可调用”修复为可按内容独立门控、服务端强校验、原子结算、可回归测试的本地私服功能。

**Architecture:** 先在 `common` 中增加默认关闭的功能门控，再以 SQLite `content_schedule` 作为活动和限时内容的服务端真值源。`gameserver::services` 集中处理时效、副本访问、物料解析、原子发放、商品校验和沙盒订单；协议 handler 只负责解码、调用服务和发送回复/推送。特殊活动玩法在完成专用协议前保持关闭，不使用通用副本伪装“已支持”。

**Tech Stack:** Rust 2024, Tokio, SQLx + SQLite, Prost, Axum, Serde/TOML

**Repository constraints:** 不修改生成的 `protocol/include/_.rs`；不修改任何系统或环境代理；自动化测试不读写 `runtime/db/sonetto.db`；本计划不包含 commit、push 或 PR 操作。

**Charge scope:** 充值商城只落地“本地沙盒商城”，不接真实支付渠道、不处理真实货币、不宣称支付成功。真实支付需单独的安全、合规和对账项目。

---

## Baseline and Release Policy

当前审计基线（数据日期 2026-07-15）：

- `episode.json`: 3,907 条副本，其中 2,269 条属于活动关联章节。
- 活动关联副本中：1 条战斗引用缺失，204 条奖励引用在 `bonus.json` 中缺失。
- `store_goods.json`: 1,263 条；按源时间窗口计算的 866 条有效商品中，198 条包含当前未发放的物料类型 `11` 或 `13`。
- `store_charge_goods.json`: 494 条；其中 19 条当前有效充值商品的奖励只在 `product` 字段，现有代码只读 `item`。
- 现有运行账号只有 41 条 `user_charge_info`，与 494 条充值配置不一致。
- `cargo test --workspace --no-fail-fast` 通过，但当前没有目标子系统的业务测试。

开放顺序固定为：

1. 功能门控、数据审计和原子物料事务。
2. 常驻商城的已支持商品子集。
3. 明确章节 ID 且通过审计的限时副本。
4. 单个活动的活动副本和对应活动商店同时开放。
5. 本地沙盒充值商城。

任一阶段未通过本计划的自动化和真客户端验收时，对应功能开关必须保持关闭。

## File Map

### Configuration and Gates

- Modify `common/src/config.rs`: 增加 `FeatureSettings` 和 `ChargeMode`，为旧配置提供安全默认值。
- Modify `common/Config.toml`: 新增默认关闭的五类功能门控。
- Create `database/migrations/043_content_schedule.sql`: 活动、限时章节、商店和充值商品的服务端时间覆盖。
- Create `database/src/db/game/content_schedule.rs`: 查询和更新内容时间窗口。
- Modify `database/src/db/game/mod.rs`: 导出 `content_schedule`。
- Create `gameserver/src/services/availability.rs`: 统一时效、活动关联和功能门控判定。

### Transactional Materials

- Create `database/src/models/game/materials.rs`: `MaterialKind` 和 `MaterialAmount`。
- Modify `database/src/models/game/mod.rs`: 导出 `materials`。
- Create `database/src/db/game/materials.rs`: 基于 `sqlx::Transaction` 的扣除、发放和余额快照。
- Modify `database/src/db/game/equipment.rs`: 新增事务版心相发放。
- Modify `database/src/models/game/heros.rs`: 新增事务版英雄/重复获取入口。
- Create `gameserver/src/services/materials.rs`: 严格解析 `type#id#amount` 并组装客户端推送。

### Dungeon Flow

- Create `database/migrations/044_dungeon_runs.sql`: 副本运行状态和幂等结算记录。
- Create `database/src/db/game/dungeon_runs.rs`: 开始、胜利、失败、中止和重复结算保护。
- Modify `database/src/db/game/mod.rs`: 导出 `dungeon_runs`。
- Create `gameserver/src/services/dungeon.rs`: 副本访问校验、成本扣除、奖励生成和原子结算。
- Modify `gameserver/src/handlers/dungeon/start_dungeon.rs`: 调用访问服务，去除仅按 `episode_id` 开战的路径。
- Modify `gameserver/src/handlers/dungeon/begin_round.rs`: 取消强制首回合胜利，仅在真实胜利时结算。
- Modify `gameserver/src/handlers/dungeon/auto_round.rs`: 与手动回合共用结算服务。
- Modify `gameserver/src/handlers/dungeon/dungeon_end_dungeon.rs`: 中止时记录状态并按 `failCost` 退还差额。
- Modify `gameserver/src/util/push.rs`: 推送函数只消费已提交的结算结果，不再伪装发放奖励。

### Activity Metadata

- Modify `gameserver/src/handlers/events.rs`: 动态生成 `GetActivityInfosReply`，新增按 ID 查询的 handler。
- Modify `gameserver/src/network/handler.rs`: 注册 `GetActivityInfosWithParamCmd` 和已确认的通用活动命令。
- Create `gameserver/src/services/activity.rs`: 将静态 `ActivityInfo` 与 `content_schedule` 合并。

### Store and Charge

- Create `gameserver/src/services/store.rs`: 商品分类、时效、限购、成本和奖励校验。
- Modify `gameserver/src/handlers/store/get_store_infos.rs`: 只返回可开放商品和真实下线时间。
- Modify `gameserver/src/handlers/store/buy_goods.rs`: 使用单个 SQLite 事务扣费、发奖和更新购买次数。
- Create `database/migrations/045_purchase_orders.sql`: 本地沙盒订单与幂等发货状态。
- Create `database/src/db/game/purchase_orders.rs`: 创建、完成和查询沙盒订单。
- Modify `gameserver/src/handlers/store/new_order.rs`: 仅在 `charge_mode = "sandbox"` 时原子完成本地订单。
- Modify `gameserver/src/handlers/charge.rs`: 基于全量配置合并用户购买状态。
- Modify `sdkserver/src/handlers/trade/good_list.rs`: 返回当前可用的沙盒商品。
- Modify `sdkserver/src/handlers/trade/payment_list.rs`: 明确标记本地沙盒方式。
- Modify `sdkserver/src/handlers/trade/order.rs`: 校验商品和订单，不产生未关联的随机订单。
- Modify `sdkserver/src/handlers/game/sdk_pay.rs` and `sdkserver/src/handlers/game/sdk_pay_complete.rs`: 仅显示沙盒结果，不触发二次发货。

### Tests, Audit, and Operations

- Create `gameserver/tests/support/mod.rs`: 内存 SQLite、迁移、最小用户和配置夹具。
- Create `gameserver/tests/dungeon_readiness.rs`: 副本访问与结算集成测试。
- Create `gameserver/tests/store_readiness.rs`: 常驻/活动商品原子购买测试。
- Create `gameserver/tests/charge_sandbox.rs`: 充值沙盒幂等和商品发放测试。
- Create `gameserver/src/services/readiness.rs` and `gameserver/src/bin/readiness_audit.rs`: 对启用内容进行引用和支持程度审计。
- Create `docs/operations/dungeon-store-open-checklist.md`: 备份、启动、灰度、验收和回滚手册。
- Modify `README.md`: 更新功能状态和“沙盒充值”边界。

---

### Task 1: Establish Isolated Integration Test Infrastructure

**Files:**
- Create: `gameserver/tests/support/mod.rs`
- Create: `gameserver/tests/dungeon_readiness.rs`
- Create: `gameserver/tests/store_readiness.rs`
- Create: `gameserver/tests/charge_sandbox.rs`

- [ ] **Step 1: Add the failing fixture-smoke test**

Create only `dungeon_readiness.rs` first. Reference the not-yet-created support module:

```rust
mod support;

#[tokio::test]
async fn in_memory_database_runs_existing_migrations() {
    let pool = support::migrated_pool().await;
    let names: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(names.contains(&"users".to_string()));
    assert!(names.contains(&"user_dungeons".to_string()));
    assert!(names.contains(&"user_store_goods".to_string()));
}
```

- [ ] **Step 2: Run RED**

Run:

```powershell
cargo test -p gameserver --test dungeon_readiness in_memory_database_runs_existing_migrations -- --nocapture
```

Expected: FAIL because `gameserver/tests/support/mod.rs` does not exist.

- [ ] **Step 3: Create the support module and reusable fixtures**

Create `support/mod.rs` with the intended API:

```rust
use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};

pub async fn migrated_pool() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("open in-memory sqlite");
    sqlx::migrate!("../database/migrations")
        .run(&pool)
        .await
        .expect("run migrations");
    pool
}
```

Extend the same file with `create_user`, `set_currency`, `currency_quantity`, `item_quantity`, and `insert_user_dungeon`. Use explicit SQL and deterministic IDs; do not call `load_all_starter_data`, because it seeds all 3,907 episodes and makes access tests ambiguous.

- [ ] **Step 4: Run GREEN and prove the fixtures do not touch runtime data**

Run the focused test and require PASS. Capture the SHA-256 and last-write time of `runtime/db/sonetto.db`, run the integration-test binary, and assert both values remain unchanged.

### Task 2: Add Safe Feature Gates and Charge Mode

**Files:**
- Modify: `common/src/config.rs`
- Modify: `common/Config.toml`

- [ ] **Step 1: Write failing configuration tests**

Add tests requiring old configuration files to deserialize with every target feature disabled:

```rust
#[test]
fn missing_features_default_to_closed() {
    let cfg: ServerConfig = toml::from_str(r#"
        banners = []
        [server]
        host = "127.0.0.1"
        dns = "localhost"
        http_port = 21000
        game_port = 23301
        [paths]
        data_dir = "./data"
        excel_data = "./data/excel2json"
        static_data = "./data/static"
        [database]
        path = "./db/sonetto.db"
    "#).unwrap();
    assert!(!cfg.features.limited_dungeons);
    assert!(!cfg.features.activity_dungeons);
    assert!(!cfg.features.activity_store);
    assert!(!cfg.features.permanent_store);
    assert_eq!(cfg.features.charge_mode, ChargeMode::Disabled);
}
```

- [ ] **Step 2: Run RED**

Run `cargo test -p common missing_features_default_to_closed -- --nocapture`.

Expected: FAIL because `features` and `ChargeMode` are undefined.

- [ ] **Step 3: Implement the exact configuration contract**

Add:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FeatureSettings {
    #[serde(default)]
    pub limited_dungeons: bool,
    #[serde(default)]
    pub activity_dungeons: bool,
    #[serde(default)]
    pub activity_store: bool,
    #[serde(default)]
    pub permanent_store: bool,
    #[serde(default)]
    pub charge_mode: ChargeMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ChargeMode {
    #[default]
    Disabled,
    Sandbox,
}
```

Add `#[serde(default)] pub features: FeatureSettings` to `ServerConfig` and add the following template block:

```toml
[features]
limited_dungeons = false
activity_dungeons = false
activity_store = false
permanent_store = false
charge_mode = "disabled"
```

- [ ] **Step 4: Run GREEN and compatibility tests**

Run:

```powershell
cargo test -p common
cargo test --workspace --no-fail-fast
```

Expected: configuration tests and all existing workspace tests pass.

### Task 3: Implement Server-side Content Scheduling

**Files:**
- Create: `database/migrations/043_content_schedule.sql`
- Create: `database/src/db/game/content_schedule.rs`
- Modify: `database/src/db/game/mod.rs`
- Create: `gameserver/src/services/mod.rs`
- Create: `gameserver/src/services/availability.rs`
- Modify: `gameserver/src/lib.rs`

- [ ] **Step 1: Write failing schedule-table and availability tests**

Add a test that queries `content_schedule` from `support::migrated_pool()` and the pure availability tests described below. The migration-table assertion must fail before migration `043` exists.

Define the intended pure API:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AvailabilityWindow {
    pub open_time_ms: i64,
    pub close_time_ms: i64,
    pub enabled: bool,
}

pub fn is_open(now_ms: i64, window: AvailabilityWindow) -> bool;
pub fn parse_excel_time(value: &str) -> Result<Option<i64>, AvailabilityError>;
```

Tests must cover blank timestamps, exact open boundary, exact close boundary, invalid dates, disabled overrides, and an override replacing source timestamps.

- [ ] **Step 2: Run RED**

Run `cargo test -p gameserver services::availability::tests -- --nocapture` and the schedule-table integration test.

Expected: FAIL because migration `043` and `services::availability` do not exist.

- [ ] **Step 3: Add the migration and database query module**

Use this schema:

```sql
CREATE TABLE content_schedule (
    content_kind TEXT NOT NULL CHECK (content_kind IN (
        'activity', 'limited_dungeon', 'store', 'charge_goods'
    )),
    content_id INTEGER NOT NULL,
    open_time INTEGER NOT NULL,
    close_time INTEGER NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT 1,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (content_kind, content_id),
    CHECK (content_id > 0),
    CHECK (close_time > open_time)
);

CREATE INDEX idx_content_schedule_active
ON content_schedule(content_kind, enabled, open_time, close_time);
```

- [ ] **Step 4: Implement schedule query and merge semantics**

`database::db::game::content_schedule::get_window` returns an optional database override. `availability::resolve_window` uses the override when present; otherwise it uses source `onlineTime/offlineTime`; blank source start means `i64::MIN`, blank source close means `i64::MAX`. Close time is exclusive.

- [ ] **Step 5: Run GREEN and migration tests**

Run the focused availability tests and the schedule-table integration test. Expected: both pass.

### Task 4: Create Strict Material Parsing and Atomic Inventory Operations

**Files:**
- Create: `database/src/models/game/materials.rs`
- Modify: `database/src/models/game/mod.rs`
- Create: `database/src/db/game/materials.rs`
- Modify: `database/src/db/game/mod.rs`
- Modify: `database/src/db/game/equipment.rs`
- Modify: `database/src/models/game/heros.rs`
- Create: `gameserver/src/services/materials.rs`
- Modify: `gameserver/src/services/mod.rs`

- [ ] **Step 1: Write failing parser tests**

Use this model:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaterialKind {
    Item,
    Currency,
    Hero,
    Skin,
    Equip,
    PowerItem,
    Building,
    ShopCurrency,
    SpecialBlock,
    InsightItem,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaterialAmount {
    pub kind: MaterialKind,
    pub id: i32,
    pub amount: i32,
}
```

`parse_materials` must map codes `1,2,4,5,9,10,11,13,14,24`, reject all other codes, reject non-positive IDs/amounts, reject malformed segments, and use checked multiplication.

- [ ] **Step 2: Run RED**

Run `cargo test -p gameserver services::materials::tests -- --nocapture`.

- [ ] **Step 3: Implement transaction APIs**

Add:

```rust
pub async fn consume_materials(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    user_id: i64,
    materials: &[MaterialAmount],
) -> anyhow::Result<()>;

pub async fn grant_materials(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    user_id: i64,
    materials: &[MaterialAmount],
) -> anyhow::Result<MaterialGrantResult>;
```

Map `ShopCurrency` to `currencies`; insert skins into `hero_all_skins`; insert buildings into `user_buildings` without specifying `uid`; require special-block amount `1` and insert into `user_special_blocks`; preserve existing duplicate-hero conversion rules; return changed IDs for post-commit pushes.

- [ ] **Step 4: Add rollback tests**

Test insufficient currency, an unsupported material in a mixed reward, duplicate skin, duplicate special block, equipment grant failure, and hero duplicate conversion. Every failure must leave costs, rewards, purchase counts, and ownership unchanged.

- [ ] **Step 5: Run GREEN**

Run:

```powershell
cargo test -p gameserver services::materials::tests -- --nocapture
cargo test -p database materials -- --nocapture
```

Expected: all material parsing and transaction tests pass.

### Task 5: Validate Dungeon Access Before Starting a Battle

**Files:**
- Create: `database/migrations/044_dungeon_runs.sql`
- Create: `database/src/db/game/dungeon_runs.rs`
- Modify: `database/src/db/game/mod.rs`
- Create: `gameserver/src/services/dungeon.rs`
- Modify: `gameserver/src/services/mod.rs`
- Modify: `gameserver/src/handlers/dungeon/start_dungeon.rs`

- [ ] **Step 1: Write failing access and dungeon-run table tests**

Add a table assertion for `dungeon_runs` and create cases for:

- mismatched `chapter_id` and `episode.chapter_id`;
- missing battle configuration;
- `multiplication` equal to `0`, negative, or greater than `4`;
- activity-linked chapter while `activity_dungeons` is false;
- activity-linked chapter without an active `content_schedule` row;
- limited chapter with an expired `limited_dungeon` schedule row;
- unfinished `preEpisodeId` when the field is non-zero;
- insufficient episode cost;
- valid normal, limited, and activity dungeon requests.

- [ ] **Step 2: Run RED**

Run `cargo test -p gameserver --test dungeon_readiness access_ -- --nocapture`.

Expected: FAIL because migration `044` and `services::dungeon` do not exist.

- [ ] **Step 3: Add the dungeon-run migration and database module**

```sql
CREATE TABLE dungeon_runs (
    run_id INTEGER PRIMARY KEY,
    user_id INTEGER NOT NULL,
    chapter_id INTEGER NOT NULL,
    episode_id INTEGER NOT NULL,
    multiplication INTEGER NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('started', 'won', 'lost', 'aborted')),
    cost_json TEXT NOT NULL,
    started_at INTEGER NOT NULL,
    settled_at INTEGER,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
    CHECK (multiplication BETWEEN 1 AND 4)
);

CREATE INDEX idx_dungeon_runs_user_status
ON dungeon_runs(user_id, status, started_at);
```

- [ ] **Step 4: Implement classification and validation**

Use these exact rules:

- `chapter.act_id != 0 || chapter.ea_activity_id != 0` means activity dungeon and uses schedule key `activity/<activity_id>`.
- A non-activity chapter becomes limited only when an enabled `limited_dungeon/<chapter_id>` schedule row exists.
- Other chapters use the existing normal-dungeon path.
- A battle episode requires its referenced `battle` row; story-only episodes require `battle_id == 0`.
- The request chapter must equal `episode.chapter_id`.
- Cost is parsed from `episode.cost` and multiplied with checked arithmetic.

- [ ] **Step 5: Consume cost and create the run atomically**

Build the battle first. After battle construction succeeds, begin one SQLite transaction, consume cost, insert `dungeon_runs(status='started')`, then commit. Only after commit place `ActiveBattle` on the connection.

- [ ] **Step 6: Run GREEN**

Run the dungeon access test group. Expected: all invalid cases perform zero writes and every valid case creates exactly one started run.

### Task 6: Implement Idempotent Dungeon Settlement and Real Finish Conditions

**Files:**
- Modify: `gameserver/src/state/battle/simulator.rs`
- Modify: `gameserver/src/handlers/dungeon/begin_round.rs`
- Modify: `gameserver/src/handlers/dungeon/auto_round.rs`
- Modify: `gameserver/src/handlers/dungeon/dungeon_end_dungeon.rs`
- Modify: `gameserver/src/state/battle/rewards.rs`
- Modify: `gameserver/src/services/dungeon.rs`
- Modify: `gameserver/src/util/push.rs`
- Modify: `database/src/db/game/dungeons.rs`
- Modify: `database/src/db/game/dungeon_runs.rs`

- [ ] **Step 1: Write failing simulator tests**

Require `process_round` to return unfinished while defenders remain alive, victory when every defender is dead, and loss when every attacker is dead or `max_round` is exceeded. Remove tests that rely on handlers overwriting `round.is_finish`.

- [ ] **Step 2: Run RED**

Run `cargo test -p gameserver state::battle::simulator -- --nocapture`.

- [ ] **Step 3: Remove forced victory and route both modes through one settlement function**

Delete handler assignments equivalent to `round.is_finish = Some(true)`. Add:

```rust
pub async fn settle_dungeon_run(
    pool: &sqlx::SqlitePool,
    user_id: i64,
    run_id: i64,
    outcome: DungeonOutcome,
) -> Result<Option<DungeonSettlement>, AppError>;
```

Return `Ok(None)` when the run is already settled, preventing duplicate progress and duplicate rewards.

- [ ] **Step 4: Make reward granting and progress updates one transaction**

Within the transaction:

1. Load the started run.
2. Read the pre-settlement `challenge_count` to determine first clear.
3. Parse `bonus`, `firstBonus`, and `freeBonus`.
4. Grant all rewards through `grant_materials`.
5. Update dungeon progress and daily counters.
6. Mark the run `won` or `lost` with `settled_at`.
7. Commit.

Send inventory, material, dungeon, and end-dungeon pushes only after commit. Populate `first_bonus` and `normal_bonus` separately instead of combining them into one field.

- [ ] **Step 5: Implement abort refund semantics**

On abort, parse `episode.fail_cost`. Refund `start_cost - fail_cost` per matching material, reject negative differences, mark the run `aborted`, and clear `active_battle`. A repeated abort must be a no-op.

- [ ] **Step 6: Add settlement integration tests**

Cover first clear, repeat clear, multiplication, missing bonus configuration, duplicate settlement, abort, loss, transaction rollback during reward grant, and reconnect after a committed win.

- [ ] **Step 7: Run GREEN**

Run:

```powershell
cargo test -p gameserver --test dungeon_readiness -- --nocapture
cargo test -p gameserver state::battle -- --nocapture
```

Expected: rewards exist in SQLite, pushes match committed values, and duplicate requests do not change state.

### Task 7: Build Activity Metadata from Schedules

**Files:**
- Create: `gameserver/src/services/activity.rs`
- Modify: `gameserver/src/services/mod.rs`
- Modify: `gameserver/src/handlers/events.rs`
- Modify: `gameserver/src/network/handler.rs`

- [ ] **Step 1: Write failing activity tests**

Load `assets/static/activity/activity_infos.json` and assert:

- a disabled schedule produces `online=false` and `is_unlock=false`;
- an active override replaces stale start/end values;
- a closed record is not reported as unlocked;
- `GetActivityInfosWithParamRequest` returns only requested IDs;
- unknown IDs are ignored rather than replaced with another activity's static payload.

- [ ] **Step 2: Run RED**

Run `cargo test -p gameserver services::activity::tests -- --nocapture`.

- [ ] **Step 3: Implement dynamic reply generation**

Deserialize the static file once, apply schedule overrides, recompute `online`, and preserve user-specific flags only where they are backed by persisted state. Do not use `send_reply!` for activity lists after this change.

- [ ] **Step 4: Register the parameterized command**

Add `CmdId::GetActivityInfosWithParamCmd` to dispatch and route both list handlers through the same service. Keep specialized activity commands disabled unless their activity has its own tested handler.

- [ ] **Step 5: Run GREEN and protocol regression tests**

Run the activity tests and existing `network::handler` tests.

### Task 8: Filter Store Listings by Category, Time, and Activity

**Files:**
- Create: `gameserver/src/services/store.rs`
- Modify: `gameserver/src/services/mod.rs`
- Modify: `gameserver/src/handlers/store/get_store_infos.rs`

- [ ] **Step 1: Write failing listing tests**

Test these fixtures:

- permanent goods with no activity and no dates;
- expired goods whose `isOnline` remains true;
- future goods;
- activity goods with active and inactive activity schedules;
- activity goods while `activity_store` is false;
- permanent goods while `permanent_store` is false;
- a requested store with no visible goods.

- [ ] **Step 2: Run RED**

Run `cargo test -p gameserver --test store_readiness list_ -- --nocapture`.

- [ ] **Step 3: Implement exact visibility rules**

For `store_goods`:

- `activity_id != 0` requires `activity_store` and an active `activity/<activity_id>` window.
- `activity_id == 0` requires `permanent_store`.
- `is_online` must be true.
- source `online_time/offline_time` and optional `store/<store_id>` override must both permit access.
- goods with a reward string that fails strict parsing are hidden and logged as readiness errors.

Set `GoodsInfo.offline_time` to the effective close timestamp instead of `0`.

- [ ] **Step 4: Verify the historical-store regression**

Use the real `store 902` data in a read-only test and assert that only goods belonging to an active scheduled activity are returned; the previous all-300 response must not recur.

- [ ] **Step 5: Run GREEN**

Run all store listing tests. Expected: expired stores `112` and `130` return zero goods unless an explicit active override is present.

### Task 9: Make Normal Store Purchases Validated and Atomic

**Files:**
- Modify: `gameserver/src/handlers/store/buy_goods.rs`
- Modify: `gameserver/src/services/store.rs`
- Modify: `database/src/db/game/sign_in.rs`

- [ ] **Step 1: Write failing purchase tests**

Cover zero/negative quantity, wrong `store_id`, offline goods, inactive activity goods, invalid `select_cost`, insufficient funds, max-buy overflow, successful item purchase, successful shop-currency reward, skin, building, special block, equipment, duplicate hero, and a forced database failure after cost validation.

- [ ] **Step 2: Run RED**

Run `cargo test -p gameserver --test store_readiness buy_ -- --nocapture`.

- [ ] **Step 3: Implement pre-transaction validation**

Require `1 <= num <= 99`, exact store ownership, visible goods, supported selected cost, and checked multiplication. Resolve the current reset period before evaluating `max_buy_count`.

- [ ] **Step 4: Implement one purchase transaction**

Within one transaction:

1. Re-read `buy_count`.
2. Re-check the limit.
3. Consume costs.
4. Grant products.
5. Upsert `user_store_goods` with checked addition.
6. Commit.

For rejected purchases send the existing reply body with result code `1`, perform no push, and keep the TCP session alive. Success uses result code `0`.

- [ ] **Step 5: Send post-commit state pushes**

Use `MaterialGrantResult` to send only changed item, currency, equipment, hero, skin, building, and block state. Never send a success-looking reward popup for a rolled-back transaction.

- [ ] **Step 6: Keep daily/weekly/monthly resets consistent**

Refactor reset-period classification into a shared function used by purchase validation and `database/src/db/game/sign_in.rs`. Add boundary tests at 05:00 server reset, week rollover, and month rollover.

- [ ] **Step 7: Run GREEN**

Run all store tests and the sign-in reset tests.

### Task 10: Implement Sandbox Charge Orders and Correct Reward Payloads

**Files:**
- Create: `database/migrations/045_purchase_orders.sql`
- Create: `database/src/db/game/purchase_orders.rs`
- Modify: `database/src/db/game/mod.rs`
- Modify: `gameserver/src/handlers/store/new_order.rs`
- Modify: `gameserver/src/handlers/charge.rs`
- Create: `gameserver/src/services/charge.rs`
- Modify: `gameserver/src/services/mod.rs`

- [ ] **Step 1: Write failing charge and purchase-order table tests**

Add a table assertion for `purchase_orders` and cover disabled mode, offline goods, invalid region/selection, expired goods, purchase limit, base crystal pack with empty `item` and populated `product`, month card, repeated fulfillment by `game_order_id`, a 494-product charge-info merge, and transaction rollback.

- [ ] **Step 2: Run RED**

Run `cargo test -p gameserver --test charge_sandbox -- --nocapture`.

Expected: FAIL because migration `045` and `services::charge` do not exist.

- [ ] **Step 3: Add the order migration and database module**

```sql
CREATE TABLE purchase_orders (
    game_order_id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL,
    goods_id INTEGER NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('created', 'fulfilled', 'cancelled')),
    amount_minor INTEGER NOT NULL,
    currency TEXT NOT NULL,
    selection_json TEXT NOT NULL,
    reward_json TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    fulfilled_at INTEGER,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
    CHECK (amount_minor >= 0)
);

CREATE INDEX idx_purchase_orders_user_created
ON purchase_orders(user_id, created_at);
```

- [ ] **Step 4: Normalize charge rewards**

Use this rule:

```rust
let base_reward = if !goods.item.trim().is_empty() {
    goods.item.as_str()
} else {
    goods.product.as_str()
};
```

Append only validated optional selections. Reject duplicate regions, missing required regions, out-of-range positions, and optional entries that do not belong to the goods ID.

- [ ] **Step 5: Fulfill sandbox orders atomically**

Only `ChargeMode::Sandbox` may create orders. In one transaction create the order, grant normalized rewards or extend the month card, upsert `user_charge_info`, update `user_stats`, then mark the order fulfilled. A repeated fulfillment of the same `game_order_id` returns the stored result without another grant.

- [ ] **Step 6: Return complete charge state**

`GetChargeInfoReply.infos` must be built from active `store_charge_goods` left-joined with persisted counts, so missing rows appear as `buy_count=0` rather than disappearing. Set `sandbox_enable=true` only in sandbox mode.

- [ ] **Step 7: Run GREEN**

Run the charge tests and independently query that one sandbox order produces one order row, one count increment, and one reward grant.

### Task 11: Make SDK Commerce Endpoints Honest and Consistent

**Files:**
- Modify: `sdkserver/src/handlers/trade/good_list.rs`
- Modify: `sdkserver/src/handlers/trade/payment_list.rs`
- Modify: `sdkserver/src/handlers/trade/order.rs`
- Modify: `sdkserver/src/handlers/game/sdk_pay.rs`
- Modify: `sdkserver/src/handlers/game/sdk_pay_complete.rs`
- Add tests beside each handler or in `sdkserver/tests/charge_sandbox_http.rs`

- [ ] **Step 1: Write failing HTTP handler tests**

Require disabled mode to return no payment methods, sandbox mode to return active goods, unknown goods to fail, and callback pages to avoid changing inventory or order status.

- [ ] **Step 2: Run RED**

Run `cargo test -p sdkserver charge_sandbox -- --nocapture`.

- [ ] **Step 3: Build the goods list from server data**

Return `goods_id`, localized name, amount, currency, and active window for currently available charge goods. Remove the unconditional empty list.

- [ ] **Step 4: Label the payment path as sandbox**

Use payment method name `Sonetto Local Sandbox`, a local/empty icon, and a local URL. Do not use external Worldpay assets or wording that implies real authorization.

- [ ] **Step 5: Link order lookup to the game order**

`/trade/order` must validate the referenced fulfilled sandbox `game_order_id` and goods ID. It must not create an unrelated random order or grant rewards.

- [ ] **Step 6: Keep callback pages side-effect free**

The callback may only notify the client UI of the already-recorded sandbox result. It cannot update inventory, charge counts, month cards, or order state.

- [ ] **Step 7: Run GREEN**

Run the SDK tests and workspace tests.

### Task 12: Add a Readiness Auditor and Explicit Content Allowlisting

**Files:**
- Create: `gameserver/src/services/readiness.rs`
- Create: `gameserver/src/bin/readiness_audit.rs`
- Modify: `gameserver/Cargo.toml` only if an explicit binary entry is required

- [ ] **Step 1: Write failing audit tests**

The audit result must contain machine-readable errors and warnings:

```rust
pub struct ReadinessReport {
    pub errors: Vec<ReadinessIssue>,
    pub warnings: Vec<ReadinessIssue>,
    pub enabled_counts: EnabledContentCounts,
}
```

Test missing battle, missing bonus, unsupported reward type, inactive activity, enabled feature with zero eligible content, charge payload mismatch, and a clean single-activity fixture.

- [ ] **Step 2: Run RED**

Run `cargo test -p gameserver services::readiness::tests -- --nocapture`.

- [ ] **Step 3: Audit only enabled content**

The auditor must:

- validate every enabled episode's chapter, battle, monster, and bonus references;
- reject specialized activity chapters unless their required command handlers are registered;
- validate every visible store cost/product through the strict material parser;
- validate every visible charge reward and optional selection;
- confirm every enabled limited/activity content item has an active schedule;
- report counts for each of the five requested categories.

- [ ] **Step 4: Implement the command-line contract**

Run:

```powershell
cargo run -p gameserver --bin readiness_audit -- `
  --config target\debug\config.toml `
  --database runtime\db\sonetto.db
```

The binary loads configuration and Excel data exactly like the server, opens SQLite read-only, prints JSON, and exits `0` only when `errors` is empty. It never writes schedules or user state.

- [ ] **Step 5: Run GREEN against fixtures and current runtime**

Fixture report must be clean. Current runtime is expected to remain non-ready until schedules are populated and feature flags are deliberately enabled.

### Task 13: Full Regression, Real-client Acceptance, and Rollout Documentation

**Files:**
- Create: `docs/operations/dungeon-store-open-checklist.md`
- Modify: `README.md`

- [ ] **Step 1: Run the complete automated gate**

```powershell
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace --no-fail-fast
cargo run -p gameserver --bin readiness_audit -- --config target\debug\config.toml --database runtime\db\sonetto.db
```

Expected before opening: formatting, build, and tests pass; readiness audit reports zero errors for the content IDs selected for rollout.

- [ ] **Step 2: Verify new and upgraded databases**

Run migrations against a new temporary database and a `.backup` copy of the existing runtime database. Confirm migrations `043-045` apply once, preserve users/inventory, and can be reopened by both servers.

- [ ] **Step 3: Document and execute permanent-store canary**

Enable only `permanent_store`. With a test account, verify one item purchase, one shop-currency purchase, one skin/building purchase, insufficient funds, max limit, restart, and re-query. Confirm no activity or charge goods appear.

- [ ] **Step 4: Document and execute limited-dungeon canary**

Insert one explicit `limited_dungeon/<chapter_id>` schedule, enable `limited_dungeons`, and verify enter, cost deduction, multi-round battle, win, reward persistence, abort, duplicate settlement, restart, and schedule expiry.

- [ ] **Step 5: Open one activity as a closed loop**

Select one activity whose generic dungeon and store data pass the auditor. Enable `activity_dungeons` and `activity_store` together, create one activity schedule, and verify activity list, dungeon reward currency, store purchase, expiry, and restart. Do not enable activities requiring unhandled `ActXXX` commands.

- [ ] **Step 6: Open sandbox charge last**

Set `charge_mode = "sandbox"`, verify base crystal pack, optional pack, month card, purchase limit, duplicate order protection, full charge-info list, and SDK UI wording. Confirm no real payment endpoint or external asset is involved.

- [ ] **Step 7: Define rollback**

Rollback is configuration-first: set all five gates to false/disabled and restart both servers. If database rollback is required, stop both servers, preserve the failed database, restore the pre-rollout SQLite `.backup`, start SDK then game server, and re-run the readiness auditor before reconnecting clients.

- [ ] **Step 8: Update public status accurately**

README must distinguish:

- generic dungeon flow supported;
- individually audited activities supported;
- specialized activity minigames still closed;
- permanent and activity stores use server-side availability;
- charge shop is local sandbox only, not real payment.

---

## Final Definition of Open-ready

A category may be marked open only when all conditions below hold:

- Its feature gate is the only newly enabled gate in the canary step.
- `readiness_audit` returns no error for every enabled content ID.
- Invalid IDs, wrong store/chapter IDs, negative quantities, expired content, insufficient resources, duplicate settlement, and repeated order fulfillment produce zero unintended writes.
- Cost deduction, reward grant, progress/count update, and order/run status are committed in one SQLite transaction.
- Client pushes are derived from the committed transaction result.
- Server restart and client reconnect preserve the same inventory, progress, purchase counts, schedules, month cards, and order states.
- Automated tests never mutate `runtime/db/sonetto.db`.
- Actual client verification covers one success and one failure path for the category.
- Logs contain no `Dispatch error`, missing JSON, missing battle/bonus reference, unsupported material type, or SQLite transaction error for the tested path.

## Plan Self-review Record

- **Coverage:** All five requested categories have an independent gate, implementation phase, automated tests, client acceptance, and rollback rule.
- **Scope:** Real-money payment and specialized activity minigames are explicitly excluded; both require separate designs and plans.
- **Consistency:** Material codes, schedule kinds, migration numbers, feature names, test commands, and order/run states are used consistently throughout the plan.
- **Safety:** Runtime database writes are excluded from implementation tests; rollout requires backup and canary activation.
