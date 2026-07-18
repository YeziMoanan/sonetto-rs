use common::time::ServerTime;
use config::destiny::DestinyConfigIndex;
use sqlx::{Sqlite, SqliteConnection, SqlitePool, pool::PoolConnection};

use crate::{
    db::game::{
        currencies::deduct_currency_if_sufficient_on_connection,
        items::deduct_item_if_sufficient_on_connection,
    },
    models::game::{
        currencies::Currency,
        destiny::{
            DestinyCommand, DestinyState, MaterialCost, MaterialKind, MutationKind,
            OwnedDestinyHero, ProgressionError, plan_transition,
        },
        items::Item,
        heros::get_hero_data_from_connection,
    },
};

const MAX_TRANSACTION_ATTEMPTS: usize = 5;
const DESTINY_BUSY_TIMEOUT_MS: u64 = 25;
const CONNECTION_ACQUIRE_TIMEOUT_MS: u64 = 50;

#[derive(Debug, Clone)]
pub struct CommittedDestinyChange {
    pub hero_id: i32,
    pub hero: sonettobuf::HeroInfo,
    pub state: DestinyState,
    pub unlocked_stones: Vec<i32>,
    pub items: Vec<Item>,
    pub currencies: Vec<Currency>,
    pub changed: bool,
}

pub async fn execute_destiny_command(
    pool: &SqlitePool,
    user_id: i64,
    catalog: &DestinyConfigIndex,
    command: DestinyCommand,
) -> Result<CommittedDestinyChange, ProgressionError> {
    for attempt in 0..MAX_TRANSACTION_ATTEMPTS {
        let mut tx = match ImmediateTransaction::begin(pool).await {
            Ok(tx) => tx,
            Err(error) if is_retryable_begin_error(&error) => {
                retry_backoff(attempt).await;
                continue;
            }
            Err(error) => return Err(ProgressionError::Database(error)),
        };

        match execute_attempt(tx.connection(), user_id, catalog, command).await {
            Ok(change) => {
                if let Err(commit_error) = tx.commit().await {
                    let retry = is_sqlite_lock_error(&commit_error);
                    tx.rollback().await.map_err(ProgressionError::Database)?;
                    if retry {
                        retry_backoff(attempt).await;
                        continue;
                    }
                    return Err(ProgressionError::Database(commit_error));
                }
                return Ok(change);
            }
            Err(AttemptError::RetryConflict) => {
                tx.rollback().await.map_err(ProgressionError::Database)?;
                retry_backoff(attempt).await;
            }
            Err(AttemptError::Progression(error)) => {
                let retry = matches!(
                    &error,
                    ProgressionError::Database(source) if is_sqlite_lock_error(source)
                );
                tx.rollback().await.map_err(ProgressionError::Database)?;
                if retry {
                    retry_backoff(attempt).await;
                    continue;
                }
                return Err(error);
            }
        }
    }

    Err(ProgressionError::Conflict)
}

struct ImmediateTransaction {
    connection: PoolConnection<Sqlite>,
    open: bool,
}

impl ImmediateTransaction {
    async fn begin(pool: &SqlitePool) -> sqlx::Result<Self> {
        let mut connection = tokio::time::timeout(
            std::time::Duration::from_millis(CONNECTION_ACQUIRE_TIMEOUT_MS),
            pool.acquire(),
        )
        .await
        .map_err(|_| sqlx::Error::PoolTimedOut)??;
        // Closing each low-frequency Destiny connection prevents this command's PRAGMA from
        // changing the behavior of unrelated users of the shared pool.
        connection.close_on_drop();
        let busy_timeout = format!("PRAGMA busy_timeout = {DESTINY_BUSY_TIMEOUT_MS}");
        sqlx::query(&busy_timeout).execute(&mut *connection).await?;
        sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut *connection)
            .await?;
        Ok(Self {
            connection,
            open: true,
        })
    }

    fn connection(&mut self) -> &mut SqliteConnection {
        &mut self.connection
    }

    async fn commit(&mut self) -> sqlx::Result<()> {
        sqlx::query("COMMIT").execute(&mut *self.connection).await?;
        self.open = false;
        Ok(())
    }

    async fn rollback(&mut self) -> sqlx::Result<()> {
        sqlx::query("ROLLBACK")
            .execute(&mut *self.connection)
            .await?;
        self.open = false;
        Ok(())
    }
}

impl Drop for ImmediateTransaction {
    fn drop(&mut self) {
        if self.open {
            self.connection.close_on_drop();
        }
    }
}

async fn retry_backoff(attempt: usize) {
    if attempt + 1 < MAX_TRANSACTION_ATTEMPTS {
        tokio::time::sleep(std::time::Duration::from_millis(10 * (attempt as u64 + 1))).await;
    }
}

enum AttemptError {
    RetryConflict,
    Progression(ProgressionError),
}

impl From<sqlx::Error> for AttemptError {
    fn from(error: sqlx::Error) -> Self {
        Self::Progression(ProgressionError::Database(error))
    }
}

impl From<ProgressionError> for AttemptError {
    fn from(error: ProgressionError) -> Self {
        Self::Progression(error)
    }
}

async fn execute_attempt(
    connection: &mut SqliteConnection,
    user_id: i64,
    catalog: &DestinyConfigIndex,
    command: DestinyCommand,
) -> Result<CommittedDestinyChange, AttemptError> {
    let hero_id = command_hero_id(command);
    let heroes = sqlx::query_as::<_, (i64, i64, i32, i32, i32, i32)>(
        "SELECT uid, user_id, hero_id, destiny_rank, destiny_level, destiny_stone
         FROM heroes
         WHERE user_id = ? AND hero_id = ?
         LIMIT 2",
    )
    .bind(user_id)
    .bind(hero_id)
    .fetch_all(&mut *connection)
    .await?;

    if heroes.len() != 1 {
        return Err(ProgressionError::Invalid(format!(
            "expected exactly one owned hero {hero_id}, found {}",
            heroes.len()
        ))
        .into());
    }
    let (hero_uid, owned_user_id, owned_hero_id, rank, level, stone) = heroes[0];
    let unlocked_stones = read_unlocked_stones(connection, hero_uid).await?;
    let owned = OwnedDestinyHero {
        hero_uid,
        user_id: owned_user_id,
        hero_id: owned_hero_id,
        state: DestinyState { rank, level, stone },
        unlocked_stones,
    };
    let plan = plan_transition(catalog, &owned, command)?;

    if plan.kind == MutationKind::NoChange {
        let hero = get_hero_data_from_connection(connection, user_id, hero_id)
            .await?
            .into();
        return Ok(CommittedDestinyChange {
            hero_id,
            hero,
            state: owned.state,
            unlocked_stones: owned.unlocked_stones,
            items: Vec::new(),
            currencies: Vec::new(),
            changed: false,
        });
    }

    let now_ms = ServerTime::now_ms();
    for cost in plan.costs.iter().copied() {
        let sufficient = match cost.kind {
            MaterialKind::Item => {
                deduct_item_if_sufficient_on_connection(connection, user_id, cost, now_ms).await?
            }
            MaterialKind::Currency => {
                deduct_currency_if_sufficient_on_connection(connection, user_id, cost, now_ms)
                    .await?
            }
        };
        if !sufficient {
            return Err(ProgressionError::Insufficient(cost).into());
        }
    }

    let cas_result = match plan.kind {
        MutationKind::Progress {
            target_rank,
            target_level,
        } => {
            sqlx::query(
                "UPDATE heroes
                 SET destiny_rank = ?, destiny_level = ?
                 WHERE uid = ? AND user_id = ? AND hero_id = ?
                   AND destiny_rank = ? AND destiny_level = ? AND destiny_stone = ?",
            )
            .bind(target_rank)
            .bind(target_level)
            .bind(hero_uid)
            .bind(user_id)
            .bind(hero_id)
            .bind(plan.expected.rank)
            .bind(plan.expected.level)
            .bind(plan.expected.stone)
            .execute(&mut *connection)
            .await?
        }
        MutationKind::UnlockStone { stone_id } => {
            let result = sqlx::query(
                "UPDATE heroes
                 SET destiny_rank = destiny_rank,
                     destiny_level = destiny_level,
                     destiny_stone = destiny_stone
                 WHERE uid = ? AND user_id = ? AND hero_id = ?
                   AND destiny_rank = ? AND destiny_level = ? AND destiny_stone = ?",
            )
            .bind(hero_uid)
            .bind(user_id)
            .bind(hero_id)
            .bind(plan.expected.rank)
            .bind(plan.expected.level)
            .bind(plan.expected.stone)
            .execute(&mut *connection)
            .await?;
            if result.rows_affected() == 1 {
                sqlx::query(
                    "INSERT INTO hero_destiny_stone_unlocks (hero_uid, stone_id) VALUES (?, ?)",
                )
                .bind(hero_uid)
                .bind(stone_id)
                .execute(&mut *connection)
                .await?;
            }
            result
        }
        MutationKind::UseStone { stone_id } => {
            sqlx::query(
                "UPDATE heroes
                 SET destiny_stone = ?
                 WHERE uid = ? AND user_id = ? AND hero_id = ?
                   AND destiny_rank = ? AND destiny_level = ? AND destiny_stone = ?",
            )
            .bind(stone_id)
            .bind(hero_uid)
            .bind(user_id)
            .bind(hero_id)
            .bind(plan.expected.rank)
            .bind(plan.expected.level)
            .bind(plan.expected.stone)
            .execute(&mut *connection)
            .await?
        }
        MutationKind::NoChange => unreachable!(),
    };

    if cas_result.rows_affected() != 1 {
        return Err(AttemptError::RetryConflict);
    }

    let state = read_destiny_state(connection, hero_uid).await?;
    let unlocked_stones = read_unlocked_stones(connection, hero_uid).await?;
    let (items, currencies) = read_resource_snapshots(connection, user_id, &plan.costs).await?;
    let hero = get_hero_data_from_connection(connection, user_id, hero_id)
        .await?
        .into();

    Ok(CommittedDestinyChange {
        hero_id,
        hero,
        state,
        unlocked_stones,
        items,
        currencies,
        changed: true,
    })
}

fn command_hero_id(command: DestinyCommand) -> i32 {
    match command {
        DestinyCommand::RankUp { hero_id }
        | DestinyCommand::LevelUp { hero_id, .. }
        | DestinyCommand::UnlockStone { hero_id, .. }
        | DestinyCommand::UseStone { hero_id, .. } => hero_id,
    }
}

async fn read_destiny_state(
    connection: &mut SqliteConnection,
    hero_uid: i64,
) -> sqlx::Result<DestinyState> {
    let (rank, level, stone) = sqlx::query_as::<_, (i32, i32, i32)>(
        "SELECT destiny_rank, destiny_level, destiny_stone FROM heroes WHERE uid = ?",
    )
    .bind(hero_uid)
    .fetch_one(&mut *connection)
    .await?;
    Ok(DestinyState { rank, level, stone })
}

async fn read_unlocked_stones(
    connection: &mut SqliteConnection,
    hero_uid: i64,
) -> sqlx::Result<Vec<i32>> {
    sqlx::query_scalar(
        "SELECT stone_id FROM hero_destiny_stone_unlocks
         WHERE hero_uid = ? ORDER BY stone_id",
    )
    .bind(hero_uid)
    .fetch_all(&mut *connection)
    .await
}

async fn read_resource_snapshots(
    connection: &mut SqliteConnection,
    user_id: i64,
    costs: &[MaterialCost],
) -> sqlx::Result<(Vec<Item>, Vec<Currency>)> {
    let mut items = Vec::new();
    let mut currencies = Vec::new();

    for cost in costs {
        match cost.kind {
            MaterialKind::Item => {
                items.push(
                    sqlx::query_as::<_, Item>(
                        "SELECT user_id, item_id, quantity, last_use_time, last_update_time,
                                total_gain_count
                         FROM items WHERE user_id = ? AND item_id = ?",
                    )
                    .bind(user_id)
                    .bind(cost.id)
                    .fetch_one(&mut *connection)
                    .await?,
                );
            }
            MaterialKind::Currency => {
                currencies.push(
                    sqlx::query_as::<_, Currency>(
                        "SELECT user_id, currency_id, quantity, last_recover_time, expired_time
                         FROM currencies WHERE user_id = ? AND currency_id = ?",
                    )
                    .bind(user_id)
                    .bind(cost.id)
                    .fetch_one(&mut *connection)
                    .await?,
                );
            }
        }
    }

    Ok((items, currencies))
}

fn is_sqlite_lock_error(error: &sqlx::Error) -> bool {
    let sqlx::Error::Database(database_error) = error else {
        return false;
    };
    let primary_code = database_error
        .code()
        .and_then(|code| code.parse::<i32>().ok())
        .map(|code| code & 0xff);
    matches!(primary_code, Some(5 | 6))
}

fn is_retryable_begin_error(error: &sqlx::Error) -> bool {
    matches!(error, sqlx::Error::PoolTimedOut) || is_sqlite_lock_error(error)
}
