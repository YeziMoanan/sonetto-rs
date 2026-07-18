use std::{
    path::{Path, PathBuf},
    str::FromStr,
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use config::destiny::DestinyConfigIndex;
use database::db::game::{
    currencies::deduct_currency_if_sufficient, destiny::execute_destiny_command,
    items::deduct_item_if_sufficient,
};
use database::db::starter_data::load_hero_list;
use database::models::game::destiny::{
    DestinyCommand, DestinyState, MaterialCost, MaterialKind, MutationKind, OwnedDestinyHero,
    ProgressionError, parse_material_costs, plan_transition,
};
use database::models::game::heros::UserHeroModel;
use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};

static TEMP_DB_SEQUENCE: AtomicU64 = AtomicU64::new(0);
const USER_ID: i64 = 2001;
const HERO_UID: i64 = 1001;

struct TestDatabase {
    pool: SqlitePool,
    path: TestPath,
}

struct TestPath(PathBuf);

struct ProductionTestDatabase {
    pool: SqlitePool,
    path: TestPath,
}

impl TestPath {
    fn cleanup(&self) -> std::io::Result<()> {
        for path in [
            self.0.clone(),
            self.0.with_extension("db-shm"),
            self.0.with_extension("db-wal"),
        ] {
            let mut last_error = None;
            for attempt in 0..100 {
                match std::fs::remove_file(&path) {
                    Ok(()) => break,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                    Err(error) => {
                        last_error = Some(error);
                        if attempt < 99 {
                            std::thread::sleep(std::time::Duration::from_millis(20));
                        }
                    }
                }
            }
            if let Some(error) = last_error {
                if path.exists() {
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    fn cleanup_once(&self) {
        for path in [
            self.0.clone(),
            self.0.with_extension("db-shm"),
            self.0.with_extension("db-wal"),
        ] {
            let _ = std::fs::remove_file(path);
        }
    }
}

impl Drop for TestPath {
    fn drop(&mut self) {
        self.cleanup_once();
    }
}

impl TestDatabase {
    async fn new() -> Self {
        let sequence = TEMP_DB_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "sonetto-destiny-{}-{nanos}-{sequence}.db",
            std::process::id()
        ));
        let options = SqliteConnectOptions::from_str(path.to_str().unwrap())
            .unwrap()
            .create_if_missing(true)
            .foreign_keys(true)
            .busy_timeout(std::time::Duration::from_millis(25));
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await
            .unwrap();

        database::run_migrations(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO users (id, username, created_at, updated_at) VALUES (?, ?, 0, 0)",
        )
        .bind(USER_ID)
        .bind(format!("destiny-test-{sequence}"))
        .execute(&pool)
        .await
        .unwrap();

        Self {
            pool,
            path: TestPath(path),
        }
    }

    async fn close(self) {
        self.pool.close().await;
        let cleanup_path = TestPath(self.path.0.clone());
        tokio::task::spawn_blocking(move || cleanup_path.cleanup())
            .await
            .expect("Destiny test database cleanup task panicked")
            .expect("failed to remove Destiny test database files after closing the pool");
    }

    async fn connect_pool(
        &self,
        max_connections: u32,
        busy_timeout: std::time::Duration,
    ) -> SqlitePool {
        let options = SqliteConnectOptions::from_str(self.path.0.to_str().unwrap())
            .unwrap()
            .foreign_keys(true)
            .busy_timeout(busy_timeout);
        SqlitePoolOptions::new()
            .max_connections(max_connections)
            .connect_with(options)
            .await
            .unwrap()
    }

    async fn connect_default_pool(&self, max_connections: u32) -> SqlitePool {
        let options = SqliteConnectOptions::from_str(self.path.0.to_str().unwrap())
            .unwrap()
            .foreign_keys(true);
        SqlitePoolOptions::new()
            .max_connections(max_connections)
            .connect_with(options)
            .await
            .unwrap()
    }

    async fn insert_hero(&self, hero_id: i32, state: DestinyState) {
        sqlx::query(
            "INSERT INTO heroes
             (uid, user_id, hero_id, create_time, level, exp, rank, breakthrough,
              skin, faith, active_skill_level, ex_skill_level, destiny_rank,
              destiny_level, destiny_stone, base_hp, base_attack, base_defense,
              base_mdefense, base_technic)
             VALUES (?, ?, ?, 0, 1, 0, 1, 0, 0, 0, 1, 1, ?, ?, ?, 1, 1, 1, 1, 1)",
        )
        .bind(HERO_UID)
        .bind(USER_ID)
        .bind(hero_id)
        .bind(state.rank)
        .bind(state.level)
        .bind(state.stone)
        .execute(&self.pool)
        .await
        .unwrap();
    }

    async fn insert_item(&self, item_id: i32, quantity: i32) {
        sqlx::query(
            "INSERT INTO items
             (user_id, item_id, quantity, last_use_time, last_update_time, total_gain_count)
             VALUES (?, ?, ?, NULL, NULL, ?)",
        )
        .bind(USER_ID)
        .bind(item_id)
        .bind(quantity)
        .bind(quantity)
        .execute(&self.pool)
        .await
        .unwrap();
    }

    async fn insert_currency(&self, currency_id: i32, quantity: i32) {
        sqlx::query(
            "INSERT INTO currencies
             (user_id, currency_id, quantity, last_recover_time, expired_time)
             VALUES (?, ?, ?, NULL, NULL)",
        )
        .bind(USER_ID)
        .bind(currency_id)
        .bind(quantity)
        .execute(&self.pool)
        .await
        .unwrap();
    }

    async fn unlock(&self, stone_id: i32) {
        sqlx::query("INSERT INTO hero_destiny_stone_unlocks (hero_uid, stone_id) VALUES (?, ?)")
            .bind(HERO_UID)
            .bind(stone_id)
            .execute(&self.pool)
            .await
            .unwrap();
    }

    async fn state(&self) -> DestinyState {
        let row = sqlx::query(
            "SELECT destiny_rank, destiny_level, destiny_stone FROM heroes WHERE uid = ?",
        )
        .bind(HERO_UID)
        .fetch_one(&self.pool)
        .await
        .unwrap();
        DestinyState {
            rank: row.get("destiny_rank"),
            level: row.get("destiny_level"),
            stone: row.get("destiny_stone"),
        }
    }

    async fn item_quantity(&self, item_id: i32) -> i32 {
        sqlx::query_scalar("SELECT quantity FROM items WHERE user_id = ? AND item_id = ?")
            .bind(USER_ID)
            .bind(item_id)
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }

    async fn currency_quantity(&self, currency_id: i32) -> i32 {
        sqlx::query_scalar("SELECT quantity FROM currencies WHERE user_id = ? AND currency_id = ?")
            .bind(USER_ID)
            .bind(currency_id)
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }

    async fn item_snapshot(&self, item_id: i32) -> (i32, Option<i64>, Option<i64>) {
        sqlx::query_as(
            "SELECT quantity, last_use_time, last_update_time
             FROM items WHERE user_id = ? AND item_id = ?",
        )
        .bind(USER_ID)
        .bind(item_id)
        .fetch_one(&self.pool)
        .await
        .unwrap()
    }

    async fn currency_snapshot(&self, currency_id: i32) -> (i32, Option<i64>) {
        sqlx::query_as(
            "SELECT quantity, last_recover_time
             FROM currencies WHERE user_id = ? AND currency_id = ?",
        )
        .bind(USER_ID)
        .bind(currency_id)
        .fetch_one(&self.pool)
        .await
        .unwrap()
    }

    async fn unlocked_stones(&self) -> Vec<i32> {
        sqlx::query_scalar(
            "SELECT stone_id FROM hero_destiny_stone_unlocks WHERE hero_uid = ? ORDER BY stone_id",
        )
        .bind(HERO_UID)
        .fetch_all(&self.pool)
        .await
        .unwrap()
    }
}

impl ProductionTestDatabase {
    async fn new() -> Self {
        let sequence = TEMP_DB_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "sonetto-destiny-{}-{nanos}-{sequence}.db",
            std::process::id()
        ));
        let options = SqliteConnectOptions::from_str(path.to_str().unwrap())
            .unwrap()
            .create_if_missing(true)
            .foreign_keys(true)
            .busy_timeout(std::time::Duration::from_millis(25));
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await
            .unwrap();

        database::run_migrations(&pool).await.unwrap();
        sqlx::query("INSERT INTO users (id, username, created_at, updated_at) VALUES (?, ?, ?, ?)")
            .bind(USER_ID)
            .bind(format!("destiny-test-{sequence}"))
            .bind(1_i64)
            .bind(1_i64)
            .execute(&pool)
            .await
            .unwrap();

        Self {
            pool,
            path: TestPath(path),
        }
    }

    async fn starter_equipment_map(&self, equip_id: i32) -> std::collections::HashMap<i32, i64> {
        const EQUIP_UID: i64 = 90_001;
        sqlx::query(
            "INSERT INTO equipment
             (uid, user_id, equip_id, level, exp, break_lv, count, is_lock, refine_lv,
              created_at, updated_at)
             VALUES (?, ?, ?, 1, 0, 0, 1, 0, 0, 1, 1)",
        )
        .bind(EQUIP_UID)
        .bind(USER_ID)
        .bind(equip_id)
        .execute(&self.pool)
        .await
        .unwrap();

        std::collections::HashMap::from([(equip_id, EQUIP_UID)])
    }

    async fn close(self) {
        self.pool.close().await;
        let cleanup_path = TestPath(self.path.0.clone());
        tokio::task::spawn_blocking(move || cleanup_path.cleanup())
            .await
            .expect("Destiny production test database cleanup task panicked")
            .expect("failed to remove Destiny production test database files after closing pool");
    }
}

fn catalog() -> &'static DestinyConfigIndex {
    static CATALOG: OnceLock<DestinyConfigIndex> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let db = load_selected_game_db();
        DestinyConfigIndex::try_from_game_db(&db).unwrap()
    })
}

fn mixed_cost_catalog() -> &'static DestinyConfigIndex {
    static CATALOG: OnceLock<DestinyConfigIndex> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let path = selected_catalog_path();
        let mut db = load_selected_game_db();
        let slots_id = db
            .character_destiny
            .iter()
            .find(|record| record.hero_id == 3073)
            .unwrap()
            .slots_id;
        let source = path.join("character_destiny_slots.json");
        let mut value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(source).unwrap()).unwrap();
        let records = value
            .as_array_mut()
            .and_then(|root| root.get_mut(1))
            .and_then(serde_json::Value::as_array_mut)
            .unwrap();
        let record = records
            .iter_mut()
            .find(|record| {
                record["slotsId"].as_i64() == Some(i64::from(slots_id))
                    && record["stage"].as_i64() == Some(1)
                    && record["node"].as_i64() == Some(1)
            })
            .unwrap();
        record["consume"] = serde_json::Value::String("1#620101#80|2#5#11".to_owned());

        let sequence = TEMP_DB_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let fixture_path = std::env::temp_dir().join(format!(
            "sonetto-destiny-slots-{}-{sequence}.json",
            std::process::id()
        ));
        std::fs::write(&fixture_path, serde_json::to_vec(&value).unwrap()).unwrap();
        db.character_destiny_slots =
            config::character_destiny_slots::CharacterDestinySlotsTable::load(
                fixture_path.to_str().unwrap(),
            )
            .unwrap();
        std::fs::remove_file(fixture_path).unwrap();
        DestinyConfigIndex::try_from_game_db(&db).unwrap()
    })
}

fn selected_catalog_path() -> PathBuf {
    std::env::var_os("JSON_DATA_DIR").map_or_else(
        || {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../sonetto-data/versions/international-3.6-destiny-runtime/excel2json")
        },
        PathBuf::from,
    )
}

#[test]
fn selected_catalog_path_prefers_json_data_dir_when_present() {
    let selected = selected_catalog_path();
    if let Some(configured) = std::env::var_os("JSON_DATA_DIR") {
        assert_eq!(selected, PathBuf::from(configured));
    }
    assert!(
        selected.is_dir(),
        "selected Destiny JSON directory does not exist: {}. Set JSON_DATA_DIR to the international 3.6 runtime excel2json directory",
        selected.display()
    );
}

fn load_selected_game_db() -> config::GameDB {
    let path = selected_catalog_path();
    let path_text = path.to_str().unwrap_or_else(|| {
        panic!(
            "selected Destiny JSON path is not valid UTF-8: {}. Set JSON_DATA_DIR to a valid runtime directory",
            path.display()
        )
    });
    config::GameDB::load(path_text).unwrap_or_else(|error| {
        panic!(
            "failed to load Destiny JSON data from selected path {}: {error}. Set JSON_DATA_DIR to the international 3.6 runtime excel2json directory",
            path.display()
        )
    })
}

fn production_game_data() -> &'static config::GameDB {
    static INITIALIZED: OnceLock<()> = OnceLock::new();
    INITIALIZED.get_or_init(|| {
        if config::configs::try_get().is_none() {
            let path = selected_catalog_path();
            let path_text = path.to_str().unwrap_or_else(|| {
                panic!(
                    "selected Destiny JSON path is not valid UTF-8: {}. Set JSON_DATA_DIR to a valid runtime directory",
                    path.display()
                )
            });
            config::configs::init(path_text).unwrap_or_else(|error| {
                panic!(
                    "failed to initialize production game data from {}: {error}",
                    path.display()
                )
            });
        }
    });
    config::configs::get()
}

fn configured_facet_ids(hero_id: i32) -> Vec<i32> {
    let destiny = production_game_data()
        .character_destiny
        .iter()
        .find(|record| record.hero_id == hero_id)
        .unwrap();
    destiny
        .facets_id
        .split('#')
        .map(|value| {
            value.parse::<i32>().unwrap_or_else(|error| {
                panic!(
                    "official facets_id {:?} for hero {hero_id} is invalid: {error}",
                    destiny.facets_id
                )
            })
        })
        .collect()
}

fn starter_test_hero() -> (i32, i32) {
    let game_data = production_game_data();
    let destiny = game_data.character_destiny.iter().next().unwrap();
    let equip_id = game_data
        .character
        .iter()
        .find_map(|record| record.equip_rec.split('#').next()?.parse::<i32>().ok())
        .unwrap();
    (destiny.hero_id, equip_id)
}

fn expected_starter_terminal(hero_id: i32) -> DestinyState {
    let game_data = production_game_data();
    let destiny = game_data
        .character_destiny
        .iter()
        .find(|record| record.hero_id == hero_id)
        .unwrap();
    let terminal = game_data
        .character_destiny_slots
        .iter()
        .filter(|slot| slot.slots_id == destiny.slots_id)
        .max_by_key(|slot| (slot.stage, slot.node))
        .unwrap();
    DestinyState {
        rank: terminal.stage,
        level: terminal.node,
        stone: configured_facet_ids(hero_id)[0],
    }
}

fn owned_hero(hero_id: i32, state: DestinyState, unlocked_stones: Vec<i32>) -> OwnedDestinyHero {
    OwnedDestinyHero {
        hero_uid: 1001,
        user_id: 2001,
        hero_id,
        state,
        unlocked_stones,
    }
}

fn item(id: i32, amount: i32) -> MaterialCost {
    MaterialCost {
        kind: MaterialKind::Item,
        id,
        amount,
    }
}

fn currency(id: i32, amount: i32) -> MaterialCost {
    MaterialCost {
        kind: MaterialKind::Currency,
        id,
        amount,
    }
}

#[tokio::test]
async fn ordinary_created_destiny_hero_starts_zero_with_no_unlocked_stones() {
    let db = ProductionTestDatabase::new().await;
    let hero_id = production_game_data()
        .character_destiny
        .iter()
        .next()
        .unwrap()
        .hero_id;
    let hero_uid = UserHeroModel::new(USER_ID, db.pool.clone())
        .create_hero(hero_id)
        .await
        .unwrap();

    let state: (i32, i32, i32) = sqlx::query_as(
        "SELECT destiny_rank, destiny_level, destiny_stone FROM heroes WHERE uid = ?",
    )
    .bind(hero_uid)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let unlocked_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM hero_destiny_stone_unlocks WHERE hero_uid = ?")
            .bind(hero_uid)
            .fetch_one(&db.pool)
            .await
            .unwrap();

    assert_eq!(state, (0, 0, 0));
    assert_eq!(unlocked_count, 0);
    db.close().await;
}

#[tokio::test]
async fn starter_destiny_hero_uses_slot_derived_terminal_state() {
    let db = ProductionTestDatabase::new().await;
    let (hero_id, equip_id) = starter_test_hero();
    let equip_map = db.starter_equipment_map(equip_id).await;
    let mut tx = db.pool.begin().await.unwrap();
    load_hero_list(&mut tx, USER_ID, &equip_map).await.unwrap();
    tx.commit().await.unwrap();

    let state: (i32, i32, i32) = sqlx::query_as(
        "SELECT destiny_rank, destiny_level, destiny_stone
         FROM heroes WHERE user_id = ? AND hero_id = ?",
    )
    .bind(USER_ID)
    .bind(hero_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let expected = expected_starter_terminal(hero_id);

    assert_eq!(state, (expected.rank, expected.level, expected.stone));
    db.close().await;
}

#[tokio::test]
async fn starter_unlocks_only_facets_owned_by_that_hero() {
    let db = ProductionTestDatabase::new().await;
    let game_data = production_game_data();
    let hero_id = game_data
        .character_destiny
        .iter()
        .max_by_key(|record| record.facets_id.split('#').count())
        .unwrap()
        .hero_id;
    let equip_id = game_data
        .character
        .iter()
        .find_map(|record| record.equip_rec.split('#').next()?.parse::<i32>().ok())
        .unwrap();
    let equip_map = db.starter_equipment_map(equip_id).await;
    let mut tx = db.pool.begin().await.unwrap();
    load_hero_list(&mut tx, USER_ID, &equip_map).await.unwrap();
    tx.commit().await.unwrap();

    let hero_uid: i64 =
        sqlx::query_scalar("SELECT uid FROM heroes WHERE user_id = ? AND hero_id = ?")
            .bind(USER_ID)
            .bind(hero_id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    let unlocked: Vec<i32> = sqlx::query_scalar(
        "SELECT stone_id FROM hero_destiny_stone_unlocks WHERE hero_uid = ? ORDER BY id",
    )
    .bind(hero_uid)
    .fetch_all(&db.pool)
    .await
    .unwrap();

    assert_eq!(unlocked, configured_facet_ids(hero_id));
    db.close().await;
}

#[test]
fn parse_costs_aggregates_duplicate_item_and_currency_entries() {
    let costs = parse_material_costs(&["1#620101#2|2#5#7", "1#620101#3|2#5#11", ""]).unwrap();

    assert_eq!(costs, vec![item(620101, 5), currency(5, 18)]);
}

#[test]
fn parse_costs_rejects_malformed_zero_negative_overflow_and_unknown_type() {
    for raw in [
        "1#620101",
        "1#620101#1#extra",
        "1#620101#0",
        "1#620101#-1",
        "1#620101#2147483648",
        "3#620101#1",
    ] {
        assert!(
            parse_material_costs(&[raw]).is_err(),
            "unexpectedly accepted {raw:?}"
        );
    }

    assert!(
        parse_material_costs(&["1#620101#2147483647", "1#620101#1"]).is_err(),
        "aggregate amounts must also fit in i32"
    );
}

#[test]
fn rank_up_plan_accepts_only_next_stage_node_one() {
    let zero = owned_hero(
        3073,
        DestinyState {
            rank: 0,
            level: 0,
            stone: 0,
        },
        vec![],
    );
    let first =
        plan_transition(catalog(), &zero, DestinyCommand::RankUp { hero_id: 3073 }).unwrap();
    assert_eq!(first.expected, zero.state);
    assert_eq!(
        first.kind,
        MutationKind::Progress {
            target_rank: 1,
            target_level: 1,
        }
    );
    assert_eq!(
        first.costs,
        vec![
            item(110704, 1),
            item(111004, 1),
            item(620101, 80),
            item(620102, 20),
        ]
    );

    let stage_one_terminal = owned_hero(
        3073,
        DestinyState {
            rank: 1,
            level: 5,
            stone: 0,
        },
        vec![],
    );
    let second = plan_transition(
        catalog(),
        &stage_one_terminal,
        DestinyCommand::RankUp { hero_id: 3073 },
    )
    .unwrap();
    assert_eq!(second.expected, stage_one_terminal.state);
    assert_eq!(
        second.kind,
        MutationKind::Progress {
            target_rank: 2,
            target_level: 1,
        }
    );
    assert!(second.costs.contains(&item(620102, 30)));
    assert!(!second.costs.contains(&item(620101, 56)));

    let not_terminal = owned_hero(
        3073,
        DestinyState {
            rank: 1,
            level: 4,
            stone: 0,
        },
        vec![],
    );
    assert!(
        plan_transition(
            catalog(),
            &not_terminal,
            DestinyCommand::RankUp { hero_id: 3073 }
        )
        .is_err()
    );
}

#[test]
fn level_up_plan_sums_every_node_and_equal_target_is_idempotent() {
    let current = owned_hero(
        3073,
        DestinyState {
            rank: 1,
            level: 1,
            stone: 0,
        },
        vec![],
    );
    let batch = plan_transition(
        catalog(),
        &current,
        DestinyCommand::LevelUp {
            hero_id: 3073,
            target_level: 4,
        },
    )
    .unwrap();
    assert_eq!(batch.expected, current.state);
    assert_eq!(
        batch.kind,
        MutationKind::Progress {
            target_rank: 1,
            target_level: 4,
        }
    );
    assert_eq!(batch.costs, vec![item(620101, 142)]);

    let same = plan_transition(
        catalog(),
        &current,
        DestinyCommand::LevelUp {
            hero_id: 3073,
            target_level: 1,
        },
    )
    .unwrap();
    assert_eq!(same.expected, current.state);
    assert_eq!(same.kind, MutationKind::NoChange);
    assert!(same.costs.is_empty());
}

#[test]
fn level_up_plan_rejects_lower_and_cross_stage_targets() {
    let current = owned_hero(
        3073,
        DestinyState {
            rank: 1,
            level: 3,
            stone: 0,
        },
        vec![],
    );
    assert!(
        plan_transition(
            catalog(),
            &current,
            DestinyCommand::LevelUp {
                hero_id: 3073,
                target_level: 2,
            }
        )
        .is_err()
    );

    let stage_terminal = owned_hero(
        3073,
        DestinyState {
            rank: 1,
            level: 5,
            stone: 0,
        },
        vec![],
    );
    assert!(
        plan_transition(
            catalog(),
            &stage_terminal,
            DestinyCommand::LevelUp {
                hero_id: 3073,
                target_level: 6,
            }
        )
        .is_err()
    );
}

#[test]
fn stone_unlock_plan_requires_rank_owned_facet_consume_and_locked_state() {
    let locked = owned_hero(
        3003,
        DestinyState {
            rank: 1,
            level: 1,
            stone: 0,
        },
        vec![],
    );
    let plan = plan_transition(
        catalog(),
        &locked,
        DestinyCommand::UnlockStone {
            hero_id: 3003,
            stone_id: 300302,
        },
    )
    .unwrap();
    assert_eq!(plan.expected, locked.state);
    assert_eq!(plan.kind, MutationKind::UnlockStone { stone_id: 300302 });
    assert_eq!(plan.costs, vec![item(620104, 1)]);

    let no_rank = owned_hero(
        3003,
        DestinyState {
            rank: 0,
            level: 0,
            stone: 0,
        },
        vec![],
    );
    assert!(
        plan_transition(
            catalog(),
            &no_rank,
            DestinyCommand::UnlockStone {
                hero_id: 3003,
                stone_id: 300302,
            }
        )
        .is_err()
    );
    assert!(
        plan_transition(
            catalog(),
            &locked,
            DestinyCommand::UnlockStone {
                hero_id: 3003,
                stone_id: 307301,
            }
        )
        .is_err()
    );

    let unlocked = owned_hero(
        3003,
        DestinyState {
            rank: 1,
            level: 1,
            stone: 0,
        },
        vec![300302],
    );
    let repeated = plan_transition(
        catalog(),
        &unlocked,
        DestinyCommand::UnlockStone {
            hero_id: 3003,
            stone_id: 300302,
        },
    )
    .unwrap();
    assert_eq!(repeated.expected, unlocked.state);
    assert_eq!(repeated.kind, MutationKind::NoChange);
    assert!(repeated.costs.is_empty());
}

#[test]
fn stone_use_plan_accepts_owned_unlocked_or_zero_and_current_is_idempotent() {
    let unequipped = owned_hero(
        3003,
        DestinyState {
            rank: 1,
            level: 1,
            stone: 0,
        },
        vec![300301],
    );
    let equip = plan_transition(
        catalog(),
        &unequipped,
        DestinyCommand::UseStone {
            hero_id: 3003,
            stone_id: 300301,
        },
    )
    .unwrap();
    assert_eq!(equip.expected, unequipped.state);
    assert_eq!(equip.kind, MutationKind::UseStone { stone_id: 300301 });
    assert!(equip.costs.is_empty());

    let equipped = owned_hero(
        3003,
        DestinyState {
            rank: 1,
            level: 1,
            stone: 300301,
        },
        vec![300301],
    );
    let repeated = plan_transition(
        catalog(),
        &equipped,
        DestinyCommand::UseStone {
            hero_id: 3003,
            stone_id: 300301,
        },
    )
    .unwrap();
    assert_eq!(repeated.expected, equipped.state);
    assert_eq!(repeated.kind, MutationKind::NoChange);

    let unequip = plan_transition(
        catalog(),
        &equipped,
        DestinyCommand::UseStone {
            hero_id: 3003,
            stone_id: 0,
        },
    )
    .unwrap();
    assert_eq!(unequip.expected, equipped.state);
    assert_eq!(unequip.kind, MutationKind::UseStone { stone_id: 0 });

    let keep_unequipped = plan_transition(
        catalog(),
        &unequipped,
        DestinyCommand::UseStone {
            hero_id: 3003,
            stone_id: 0,
        },
    )
    .unwrap();
    assert_eq!(keep_unequipped.expected, unequipped.state);
    assert_eq!(keep_unequipped.kind, MutationKind::NoChange);

    assert!(
        plan_transition(
            catalog(),
            &unequipped,
            DestinyCommand::UseStone {
                hero_id: 3003,
                stone_id: 300302,
            }
        )
        .is_err()
    );
    assert!(
        plan_transition(
            catalog(),
            &unequipped,
            DestinyCommand::UseStone {
                hero_id: 3003,
                stone_id: 307301,
            }
        )
        .is_err()
    );
}

#[test]
fn stone_use_plan_rejects_invalid_persisted_current_stone() {
    let foreign_current = owned_hero(
        3003,
        DestinyState {
            rank: 1,
            level: 1,
            stone: 307301,
        },
        vec![307301],
    );
    assert!(matches!(
        plan_transition(
            catalog(),
            &foreign_current,
            DestinyCommand::UseStone {
                hero_id: 3003,
                stone_id: 307301,
            },
        ),
        Err(ProgressionError::Invalid(_))
    ));

    let locked_current = owned_hero(
        3003,
        DestinyState {
            rank: 1,
            level: 1,
            stone: 300302,
        },
        vec![],
    );
    assert!(matches!(
        plan_transition(
            catalog(),
            &locked_current,
            DestinyCommand::UseStone {
                hero_id: 3003,
                stone_id: 300302,
            },
        ),
        Err(ProgressionError::Invalid(_))
    ));
}

#[tokio::test]
async fn rank_up_deducts_exact_items_and_currencies_and_commits_state() {
    let db = TestDatabase::new().await;
    db.insert_hero(
        3073,
        DestinyState {
            rank: 0,
            level: 0,
            stone: 0,
        },
    )
    .await;
    db.insert_item(620101, 100).await;
    db.insert_currency(5, 20).await;

    let mut helper_tx = db.pool.begin().await.unwrap();
    assert!(
        deduct_item_if_sufficient(&mut helper_tx, USER_ID, item(620101, 100), 123,)
            .await
            .unwrap()
    );
    assert!(
        !deduct_item_if_sufficient(&mut helper_tx, USER_ID, item(620101, 1), 124,)
            .await
            .unwrap()
    );
    assert!(
        deduct_currency_if_sufficient(&mut helper_tx, USER_ID, currency(5, 20), 125,)
            .await
            .unwrap()
    );
    assert!(
        !deduct_currency_if_sufficient(&mut helper_tx, USER_ID, currency(5, 1), 126,)
            .await
            .unwrap()
    );
    helper_tx.rollback().await.unwrap();

    let change = execute_destiny_command(
        &db.pool,
        USER_ID,
        mixed_cost_catalog(),
        DestinyCommand::RankUp { hero_id: 3073 },
    )
    .await
    .unwrap();

    assert!(change.changed);
    assert_eq!(change.hero_id, 3073);
    assert_eq!(
        change.state,
        DestinyState {
            rank: 1,
            level: 1,
            stone: 0
        }
    );
    assert_eq!(change.items.len(), 1);
    assert_eq!(change.items[0].item_id, 620101);
    assert_eq!(change.items[0].quantity, 20);
    assert_eq!(change.currencies.len(), 1);
    assert_eq!(change.currencies[0].currency_id, 5);
    assert_eq!(change.currencies[0].quantity, 9);
    assert_eq!(db.state().await, change.state);
    assert_eq!(db.item_quantity(620101).await, 20);
    assert_eq!(db.currency_quantity(5).await, 9);
    db.close().await;
}

#[tokio::test]
async fn batch_level_up_deducts_aggregated_cost_once() {
    let db = TestDatabase::new().await;
    db.insert_hero(
        3073,
        DestinyState {
            rank: 1,
            level: 1,
            stone: 307301,
        },
    )
    .await;
    db.unlock(307301).await;
    db.insert_item(620101, 200).await;

    let change = execute_destiny_command(
        &db.pool,
        USER_ID,
        catalog(),
        DestinyCommand::LevelUp {
            hero_id: 3073,
            target_level: 4,
        },
    )
    .await
    .unwrap();

    assert_eq!(
        change.state,
        DestinyState {
            rank: 1,
            level: 4,
            stone: 307301
        }
    );
    assert_eq!(change.unlocked_stones, vec![307301]);
    assert_eq!(change.items.len(), 1);
    assert_eq!(change.items[0].quantity, 58);
    assert_eq!(db.item_quantity(620101).await, 58);
    db.close().await;
}

#[tokio::test]
async fn insufficient_mixed_cost_rolls_back_every_write() {
    let db = TestDatabase::new().await;
    db.insert_hero(
        3073,
        DestinyState {
            rank: 0,
            level: 0,
            stone: 0,
        },
    )
    .await;
    db.insert_item(620101, 80).await;
    db.insert_currency(5, 10).await;

    let result = execute_destiny_command(
        &db.pool,
        USER_ID,
        mixed_cost_catalog(),
        DestinyCommand::RankUp { hero_id: 3073 },
    )
    .await;

    assert!(matches!(
        result,
        Err(ProgressionError::Insufficient(MaterialCost {
            kind: MaterialKind::Currency,
            id: 5,
            amount: 11,
        }))
    ));
    assert_eq!(db.item_quantity(620101).await, 80);
    assert_eq!(db.currency_quantity(5).await, 10);
    assert_eq!(
        db.state().await,
        DestinyState {
            rank: 0,
            level: 0,
            stone: 0
        }
    );
    db.close().await;
}

#[tokio::test]
async fn hero_update_error_rolls_back_resource_deductions() {
    let db = TestDatabase::new().await;
    db.insert_hero(
        3073,
        DestinyState {
            rank: 0,
            level: 0,
            stone: 0,
        },
    )
    .await;
    db.insert_item(620101, 100).await;
    db.insert_currency(5, 20).await;
    sqlx::query(
        "CREATE TRIGGER reject_destiny_update
         BEFORE UPDATE OF destiny_rank, destiny_level, destiny_stone ON heroes
         BEGIN SELECT RAISE(ABORT, 'forced hero update failure'); END",
    )
    .execute(&db.pool)
    .await
    .unwrap();

    let result = execute_destiny_command(
        &db.pool,
        USER_ID,
        mixed_cost_catalog(),
        DestinyCommand::RankUp { hero_id: 3073 },
    )
    .await;

    assert!(matches!(result, Err(ProgressionError::Database(_))));
    assert_eq!(db.item_quantity(620101).await, 100);
    assert_eq!(db.currency_quantity(5).await, 20);
    assert_eq!(
        db.state().await,
        DestinyState {
            rank: 0,
            level: 0,
            stone: 0
        }
    );
    db.close().await;
}

#[tokio::test]
async fn hero_snapshot_schema_error_rolls_back_every_write() {
    let db = TestDatabase::new().await;
    db.insert_hero(
        3073,
        DestinyState {
            rank: 0,
            level: 0,
            stone: 0,
        },
    )
    .await;
    db.insert_item(620101, 100).await;
    db.insert_currency(5, 20).await;
    sqlx::query("ALTER TABLE heroes DROP COLUMN create_time")
        .execute(&db.pool)
        .await
        .unwrap();

    let result = execute_destiny_command(
        &db.pool,
        USER_ID,
        mixed_cost_catalog(),
        DestinyCommand::RankUp { hero_id: 3073 },
    )
    .await;

    assert!(matches!(result, Err(ProgressionError::Database(_))));
    assert_eq!(db.item_quantity(620101).await, 100);
    assert_eq!(db.currency_quantity(5).await, 20);
    assert_eq!(
        db.state().await,
        DestinyState {
            rank: 0,
            level: 0,
            stone: 0
        }
    );
    db.close().await;
}

#[tokio::test]
async fn repeated_level_target_does_not_charge_twice() {
    let db = TestDatabase::new().await;
    db.insert_hero(
        3073,
        DestinyState {
            rank: 1,
            level: 1,
            stone: 0,
        },
    )
    .await;
    db.insert_item(620101, 200).await;
    let command = DestinyCommand::LevelUp {
        hero_id: 3073,
        target_level: 4,
    };

    let first = execute_destiny_command(&db.pool, USER_ID, catalog(), command)
        .await
        .unwrap();
    let repeated = execute_destiny_command(&db.pool, USER_ID, catalog(), command)
        .await
        .unwrap();

    assert!(first.changed);
    assert!(!repeated.changed);
    assert!(repeated.items.is_empty());
    assert!(repeated.currencies.is_empty());
    assert_eq!(repeated.state, first.state);
    assert_eq!(db.item_quantity(620101).await, 58);
    db.close().await;
}

#[tokio::test]
async fn stone_unlock_cost_and_row_commit_atomically() {
    let db = TestDatabase::new().await;
    db.insert_hero(
        3003,
        DestinyState {
            rank: 1,
            level: 1,
            stone: 0,
        },
    )
    .await;
    db.insert_item(620104, 2).await;

    let change = execute_destiny_command(
        &db.pool,
        USER_ID,
        catalog(),
        DestinyCommand::UnlockStone {
            hero_id: 3003,
            stone_id: 300302,
        },
    )
    .await
    .unwrap();

    assert!(change.changed);
    assert_eq!(
        change.state,
        DestinyState {
            rank: 1,
            level: 1,
            stone: 0
        }
    );
    assert_eq!(change.unlocked_stones, vec![300302]);
    assert_eq!(change.items[0].quantity, 1);
    assert_eq!(db.unlocked_stones().await, vec![300302]);
    assert_eq!(db.item_quantity(620104).await, 1);
    db.close().await;
}

#[tokio::test]
async fn stone_unlock_insert_error_rolls_back_cost_and_hero_cas() {
    let db = TestDatabase::new().await;
    let original = DestinyState {
        rank: 1,
        level: 1,
        stone: 0,
    };
    db.insert_hero(3003, original).await;
    db.insert_item(620104, 2).await;
    sqlx::query(
        "CREATE TRIGGER reject_destiny_unlock
         BEFORE INSERT ON hero_destiny_stone_unlocks
         BEGIN SELECT RAISE(ABORT, 'forced unlock insert failure'); END",
    )
    .execute(&db.pool)
    .await
    .unwrap();

    let result = execute_destiny_command(
        &db.pool,
        USER_ID,
        catalog(),
        DestinyCommand::UnlockStone {
            hero_id: 3003,
            stone_id: 300302,
        },
    )
    .await;

    assert!(matches!(result, Err(ProgressionError::Database(_))));
    assert_eq!(db.item_quantity(620104).await, 2);
    assert_eq!(db.state().await, original);
    assert!(db.unlocked_stones().await.is_empty());
    db.close().await;
}

#[tokio::test]
async fn repeated_stone_unlock_does_not_charge_twice() {
    let db = TestDatabase::new().await;
    db.insert_hero(
        3003,
        DestinyState {
            rank: 1,
            level: 1,
            stone: 0,
        },
    )
    .await;
    db.insert_item(620104, 2).await;
    let command = DestinyCommand::UnlockStone {
        hero_id: 3003,
        stone_id: 300302,
    };

    let first = execute_destiny_command(&db.pool, USER_ID, catalog(), command)
        .await
        .unwrap();
    let repeated = execute_destiny_command(&db.pool, USER_ID, catalog(), command)
        .await
        .unwrap();

    assert!(first.changed);
    assert!(!repeated.changed);
    assert!(repeated.items.is_empty());
    assert_eq!(repeated.unlocked_stones, vec![300302]);
    assert_eq!(db.item_quantity(620104).await, 1);
    db.close().await;
}

#[tokio::test]
async fn stone_use_equip_switch_and_unequip_persist() {
    let db = TestDatabase::new().await;
    db.insert_hero(
        3003,
        DestinyState {
            rank: 1,
            level: 1,
            stone: 0,
        },
    )
    .await;
    db.unlock(300301).await;
    db.unlock(300302).await;

    for stone_id in [300301, 300302, 0] {
        let change = execute_destiny_command(
            &db.pool,
            USER_ID,
            catalog(),
            DestinyCommand::UseStone {
                hero_id: 3003,
                stone_id,
            },
        )
        .await
        .unwrap();
        assert!(change.changed);
        assert_eq!(change.state.stone, stone_id);
        assert_eq!(db.state().await.stone, stone_id);
        assert_eq!(change.unlocked_stones, vec![300301, 300302]);
    }
    db.close().await;
}

#[tokio::test]
async fn foreign_or_locked_stone_use_writes_nothing() {
    let db = TestDatabase::new().await;
    let original = DestinyState {
        rank: 1,
        level: 1,
        stone: 0,
    };
    db.insert_hero(3003, original).await;

    for stone_id in [300302, 307301] {
        let result = execute_destiny_command(
            &db.pool,
            USER_ID,
            catalog(),
            DestinyCommand::UseStone {
                hero_id: 3003,
                stone_id,
            },
        )
        .await;
        assert!(matches!(result, Err(ProgressionError::Invalid(_))));
        assert_eq!(db.state().await, original);
        assert!(db.unlocked_stones().await.is_empty());
    }
    db.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_rank_up_charges_once_and_advances_once() {
    let db = TestDatabase::new().await;
    let destiny_catalog = catalog();
    db.insert_hero(
        3073,
        DestinyState {
            rank: 0,
            level: 0,
            stone: 0,
        },
    )
    .await;
    for item_id in [110704, 111004, 620101, 620102] {
        db.insert_item(item_id, 1_000).await;
    }
    let before = [
        (110704, db.item_quantity(110704).await),
        (111004, db.item_quantity(111004).await),
        (620101, db.item_quantity(620101).await),
        (620102, db.item_quantity(620102).await),
    ];
    let first_pool = db
        .connect_pool(1, std::time::Duration::from_millis(5))
        .await;
    let second_pool = db
        .connect_pool(1, std::time::Duration::from_millis(5))
        .await;
    let holder_pool = db
        .connect_pool(1, std::time::Duration::from_millis(5))
        .await;
    sqlx::query("SELECT 1").execute(&first_pool).await.unwrap();
    sqlx::query("SELECT 1").execute(&second_pool).await.unwrap();
    let holder = holder_pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(3));
    let first_barrier = barrier.clone();
    let second_barrier = barrier.clone();
    let first_task_pool = first_pool.clone();
    let second_task_pool = second_pool.clone();
    let first_catalog = destiny_catalog;
    let second_catalog = destiny_catalog;

    let first = tokio::spawn(async move {
        first_barrier.wait().await;
        execute_destiny_command(
            &first_task_pool,
            USER_ID,
            first_catalog,
            DestinyCommand::RankUp { hero_id: 3073 },
        )
        .await
    });
    let second = tokio::spawn(async move {
        second_barrier.wait().await;
        execute_destiny_command(
            &second_task_pool,
            USER_ID,
            second_catalog,
            DestinyCommand::RankUp { hero_id: 3073 },
        )
        .await
    });
    barrier.wait().await;
    tokio::time::sleep(std::time::Duration::from_millis(35)).await;
    holder.commit().await.unwrap();
    let (first, second) = tokio::join!(first, second);
    let results = [first.unwrap(), second.unwrap()];

    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Ok(change) if change.changed))
            .count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(
                result,
                Err(ProgressionError::Invalid(_) | ProgressionError::Conflict)
            ))
            .count(),
        1,
        "the losing request must fail as a recoverable state conflict, not a database lock"
    );
    assert_eq!(
        db.state().await,
        DestinyState {
            rank: 1,
            level: 1,
            stone: 0
        }
    );
    let expected_costs = [(110704, 1), (111004, 1), (620101, 80), (620102, 20)];
    for ((item_id, quantity), (_, cost)) in before.into_iter().zip(expected_costs) {
        assert_eq!(db.item_quantity(item_id).await, quantity - cost);
    }
    first_pool.close().await;
    second_pool.close().await;
    holder_pool.close().await;
    db.close().await;
}

#[tokio::test]
async fn persistent_lock_retry_exhaustion_returns_conflict_without_writes() {
    let db = TestDatabase::new().await;
    let original = DestinyState {
        rank: 0,
        level: 0,
        stone: 0,
    };
    db.insert_hero(3073, original).await;
    for item_id in [110704, 111004, 620101, 620102] {
        db.insert_item(item_id, 1_000).await;
    }
    let executor_pool = db
        .connect_pool(1, std::time::Duration::from_millis(2))
        .await;
    let holder_pool = db
        .connect_pool(1, std::time::Duration::from_millis(2))
        .await;
    let holder = holder_pool.begin_with("BEGIN IMMEDIATE").await.unwrap();

    let result = execute_destiny_command(
        &executor_pool,
        USER_ID,
        catalog(),
        DestinyCommand::RankUp { hero_id: 3073 },
    )
    .await;

    assert!(matches!(result, Err(ProgressionError::Conflict)));
    assert_eq!(db.state().await, original);
    for item_id in [110704, 111004, 620101, 620102] {
        assert_eq!(db.item_quantity(item_id).await, 1_000);
    }
    holder.rollback().await.unwrap();
    executor_pool.close().await;
    holder_pool.close().await;
    db.close().await;
}

#[tokio::test]
async fn item_deduction_rejects_wrong_kind_and_non_positive_values_without_writes() {
    let db = TestDatabase::new().await;
    db.insert_item(620101, 5).await;
    let before = db.item_snapshot(620101).await;
    let mut tx = db.pool.begin().await.unwrap();
    let results = [
        deduct_item_if_sufficient(&mut tx, USER_ID, currency(620101, 1), 101).await,
        deduct_item_if_sufficient(&mut tx, USER_ID, item(620101, 0), 102).await,
        deduct_item_if_sufficient(&mut tx, USER_ID, item(620101, -1), 103).await,
        deduct_item_if_sufficient(&mut tx, USER_ID, item(0, 1), 104).await,
    ];
    tx.commit().await.unwrap();

    assert!(
        results
            .iter()
            .all(|result| matches!(result, Err(sqlx::Error::Protocol(_))))
    );
    assert_eq!(db.item_snapshot(620101).await, before);
    db.close().await;
}

#[tokio::test]
async fn currency_deduction_rejects_wrong_kind_and_non_positive_values_without_writes() {
    let db = TestDatabase::new().await;
    db.insert_currency(5, 7).await;
    let before = db.currency_snapshot(5).await;
    let mut tx = db.pool.begin().await.unwrap();
    let results = [
        deduct_currency_if_sufficient(&mut tx, USER_ID, item(5, 1), 101).await,
        deduct_currency_if_sufficient(&mut tx, USER_ID, currency(5, 0), 102).await,
        deduct_currency_if_sufficient(&mut tx, USER_ID, currency(5, -1), 103).await,
        deduct_currency_if_sufficient(&mut tx, USER_ID, currency(0, 1), 104).await,
    ];
    tx.commit().await.unwrap();

    assert!(
        results
            .iter()
            .all(|result| matches!(result, Err(sqlx::Error::Protocol(_))))
    );
    assert_eq!(db.currency_snapshot(5).await, before);
    db.close().await;
}

#[tokio::test]
async fn default_pool_persistent_lock_returns_conflict_within_deadline() {
    let db = TestDatabase::new().await;
    let original = DestinyState {
        rank: 0,
        level: 0,
        stone: 0,
    };
    db.insert_hero(3073, original).await;
    for item_id in [110704, 111004, 620101, 620102] {
        db.insert_item(item_id, 1_000).await;
    }
    let executor_pool = db.connect_default_pool(1).await;
    let holder_pool = db
        .connect_pool(1, std::time::Duration::from_millis(5))
        .await;
    let holder = holder_pool.begin_with("BEGIN IMMEDIATE").await.unwrap();

    let result = tokio::time::timeout(
        std::time::Duration::from_millis(800),
        execute_destiny_command(
            &executor_pool,
            USER_ID,
            catalog(),
            DestinyCommand::RankUp { hero_id: 3073 },
        ),
    )
    .await;

    assert!(matches!(result, Ok(Err(ProgressionError::Conflict))));
    assert_eq!(db.state().await, original);
    for item_id in [110704, 111004, 620101, 620102] {
        assert_eq!(db.item_quantity(item_id).await, 1_000);
    }
    holder.rollback().await.unwrap();
    executor_pool.close().await;
    holder_pool.close().await;
    db.close().await;
}

#[tokio::test]
async fn reader_blocked_commit_rolls_back_each_attempt_and_releases_connection() {
    let db = TestDatabase::new().await;
    let original = DestinyState {
        rank: 0,
        level: 0,
        stone: 0,
    };
    db.insert_hero(3073, original).await;
    for item_id in [110704, 111004, 620101, 620102] {
        db.insert_item(item_id, 1_000).await;
    }
    let executor_pool = db
        .connect_pool(1, std::time::Duration::from_millis(5))
        .await;
    let reader_pool = db
        .connect_pool(1, std::time::Duration::from_millis(5))
        .await;
    let mut reader = reader_pool.acquire().await.unwrap();
    sqlx::query("BEGIN").execute(&mut *reader).await.unwrap();
    let _: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM heroes")
        .fetch_one(&mut *reader)
        .await
        .unwrap();

    let result = execute_destiny_command(
        &executor_pool,
        USER_ID,
        catalog(),
        DestinyCommand::RankUp { hero_id: 3073 },
    )
    .await;

    assert!(matches!(result, Err(ProgressionError::Conflict)));
    assert_eq!(db.state().await, original);
    for item_id in [110704, 111004, 620101, 620102] {
        assert_eq!(db.item_quantity(item_id).await, 1_000);
    }
    sqlx::query("ROLLBACK").execute(&mut *reader).await.unwrap();
    drop(reader);

    let committed = execute_destiny_command(
        &executor_pool,
        USER_ID,
        catalog(),
        DestinyCommand::RankUp { hero_id: 3073 },
    )
    .await
    .unwrap();
    assert!(committed.changed);
    assert_eq!(committed.state.rank, 1);

    executor_pool.close().await;
    reader_pool.close().await;
    db.close().await;
}
