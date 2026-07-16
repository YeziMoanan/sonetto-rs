use crate::network::packet::ClientPacket;
use crate::state::ConnectionContext;
use crate::util::push;
use crate::{error::AppError, handlers::item::util::can_claim_month_card_with_lookup};
use prost::Message;
use sonettobuf::{AutoUseExpirePowerItemReply, AutoUseExpirePowerItemRequest, CmdId};
use std::sync::Arc;
use tokio::sync::Mutex;

pub async fn on_auto_use_expire_power_item(
    ctx: Arc<Mutex<ConnectionContext>>,
    req: ClientPacket,
) -> Result<(), AppError> {
    on_auto_use_expire_power_item_with_lookups(
        ctx,
        req,
        |item_id| {
            config::configs::get()
                .power_item
                .iter()
                .find(|item| item.id == item_id)
                .map(|item| item.effect)
        },
        |card_id| {
            config::configs::get()
                .month_card
                .iter()
                .find(|card| card.id == card_id)
                .map(|card| card.daily_bonus.clone())
        },
    )
    .await
}

async fn on_auto_use_expire_power_item_with_lookups<PowerEffect, MonthCardBonus>(
    ctx: Arc<Mutex<ConnectionContext>>,
    req: ClientPacket,
    power_effect: PowerEffect,
    month_card_bonus: MonthCardBonus,
) -> Result<(), AppError>
where
    PowerEffect: Fn(i32) -> Option<i32>,
    MonthCardBonus: Fn(i32) -> Option<String>,
{
    let request = AutoUseExpirePowerItemRequest::decode(&req.data[..])?;
    tracing::info!("Received AutoUseExpirePowerItemRequest: {:?}", request);

    let (user_id, used_any) = {
        let conn = ctx.lock().await;
        let player_id = conn.player_id.ok_or(AppError::NotLoggedIn)?;
        let pool = &conn.state.db;

        let now = common::time::ServerTime::now_ms();
        let now_seconds = now / 1000;

        let expired_items: Vec<(i64, i32, i32, i64)> = sqlx::query_as(
            "SELECT uid, item_id, quantity, expire_time
             FROM power_items
             WHERE user_id = ? AND expire_time > 0 AND expire_time < ?",
        )
        .bind(player_id)
        .bind(now_seconds)
        .fetch_all(pool)
        .await?;

        if expired_items.is_empty() {
            tracing::info!("Player has no expired power items");
            (player_id, false)
        } else {
            let mut total_stamina = 0;

            for (uid, item_id, quantity, expire_time) in &expired_items {
                if let Some(effect) = power_effect(*item_id) {
                    let stamina_gain = effect * quantity;
                    total_stamina += stamina_gain;

                    tracing::info!(
                        "Auto-using expired power item {} (uid: {}, qty: {}, effect: {}, expired at: {})",
                        item_id,
                        uid,
                        quantity,
                        effect,
                        expire_time
                    );

                    sqlx::query("DELETE FROM power_items WHERE uid = ? AND user_id = ?")
                        .bind(uid)
                        .bind(player_id)
                        .execute(pool)
                        .await?;
                }
            }

            if total_stamina > 0 {
                let current_stamina: i32 = sqlx::query_scalar(
                    "SELECT quantity FROM currencies WHERE user_id = ? AND currency_id = 4",
                )
                .bind(player_id)
                .fetch_optional(pool)
                .await?
                .unwrap_or(0);

                let new_stamina = current_stamina + total_stamina;

                sqlx::query(
                    "INSERT INTO currencies (user_id, currency_id, quantity, last_recover_time, expired_time)
                     VALUES (?, 4, ?, ?, 0)
                     ON CONFLICT(user_id, currency_id)
                     DO UPDATE SET quantity = ?"
                )
                .bind(player_id)
                .bind(new_stamina)
                .bind(now)
                .bind(new_stamina)
                .execute(pool)
                .await?;

                tracing::info!(
                    "Expired items auto-converted {} expired power items into {} stamina (total: {})",
                    expired_items.len(),
                    total_stamina,
                    new_stamina
                );
            }

            (player_id, true)
        }
    };

    let data = AutoUseExpirePowerItemReply {
        used: Some(used_any),
    };

    {
        let mut conn = ctx.lock().await;
        conn.send_reply(CmdId::AutoUseExpirePowerItemCmd, data, 0, req.up_tag)
            .await?;
    }

    if used_any {
        push::send_currency_change_push(ctx.clone(), user_id, vec![(4, 0)]).await?;
    }

    can_claim_month_card_with_lookup(ctx.clone(), user_id, month_card_bonus).await?;

    let should_save = {
        let mut conn = ctx.lock().await;
        if let Some(ps) = conn.player_state.as_mut() {
            if !ps.initial_login_complete {
                tracing::info!("Completing initial login");

                ps.last_state_push_sent_timestamp = None;
                ps.last_activity_push_sent_timestamp = None;
                true
            } else {
                false
            }
        } else {
            false
        }
    };

    if should_save {
        let conn = ctx.lock().await;
        conn.save_current_player_state().await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{on_auto_use_expire_power_item, on_auto_use_expire_power_item_with_lookups, push};
    use crate::{
        handlers::item::util::can_claim_month_card_with_lookup,
        network::packet::ClientPacket,
        state::{AppState, ConnectionContext, PlayerState},
    };
    use common::time::ServerTime;
    use prost::Message;
    use sonettobuf::{AutoUseExpirePowerItemRequest, CmdId};
    use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};
    use std::{
        io::{self, Write},
        sync::{Arc, Mutex as StdMutex},
    };
    use tokio::{
        net::{TcpListener, TcpStream},
        sync::Mutex,
    };
    use tracing_subscriber::fmt::MakeWriter;

    #[derive(Clone, Default)]
    struct CaptureWriter(Arc<StdMutex<Vec<u8>>>);

    impl CaptureWriter {
        fn contents(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }
    }

    impl Write for CaptureWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for CaptureWriter {
        type Writer = CaptureWriter;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    fn capture_logs() -> (CaptureWriter, tracing::dispatcher::DefaultGuard) {
        let writer = CaptureWriter::default();
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .without_time()
            .with_target(false)
            .with_level(false)
            .with_max_level(tracing::Level::TRACE)
            .with_writer(writer.clone())
            .finish();
        let dispatch = tracing::Dispatch::new(subscriber);
        let guard = tracing::dispatcher::set_default(&dispatch);
        (writer, guard)
    }

    async fn migrated_pool() -> SqlitePool {
        let db = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        database::run_migrations(&db).await.unwrap();
        db
    }

    async fn insert_user(db: &SqlitePool, user_id: i64) {
        sqlx::query(
            "INSERT INTO users (id, username, created_at, updated_at) VALUES (?1, ?2, 1, 1)",
        )
        .bind(user_id)
        .bind(format!("auto-use-{user_id}"))
        .execute(db)
        .await
        .unwrap();
    }

    async fn context(
        db: SqlitePool,
        player_id: Option<i64>,
        player_state: Option<PlayerState>,
    ) -> (Arc<Mutex<ConnectionContext>>, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (client, server) = tokio::join!(TcpStream::connect(address), listener.accept());
        let mut conn = ConnectionContext::new(
            Arc::new(Mutex::new(server.unwrap().0)),
            Arc::new(AppState::new(db)),
        );
        conn.player_id = player_id;
        conn.player_state = player_state;
        (Arc::new(Mutex::new(conn)), client.unwrap())
    }

    fn request(up_tag: u8) -> ClientPacket {
        ClientPacket {
            sequence: 1,
            cmd_id: CmdId::AutoUseExpirePowerItemCmd as i16,
            up_tag,
            data: AutoUseExpirePowerItemRequest {}.encode_to_vec(),
        }
    }

    async fn insert_active_month_card(db: &SqlitePool, user_id: i64) {
        sqlx::query(
            "INSERT INTO user_month_card_history (user_id, card_id, start_time, end_time) VALUES (?1, 610001, 1, ?2)",
        )
        .bind(user_id)
        .bind(ServerTime::now_ms() / 1000 + 3_600)
        .execute(db)
        .await
        .unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn login_initialization_entry_and_early_month_card_paths_log_no_player_ids() {
        let (writer, _guard) = capture_logs();

        let no_player_db = migrated_pool().await;
        let (no_player_ctx, _no_player_client) = context(no_player_db, None, None).await;
        assert!(
            on_auto_use_expire_power_item(no_player_ctx, request(1))
                .await
                .is_err()
        );

        let initial_user_id = 7_654_321_001_i64;
        let initial_db = migrated_pool().await;
        insert_user(&initial_db, initial_user_id).await;
        let initial_state = PlayerState::new(initial_user_id, ServerTime::now_ms());
        let (initial_ctx, _initial_client) =
            context(initial_db, Some(initial_user_id), Some(initial_state)).await;
        on_auto_use_expire_power_item(initial_ctx, request(2))
            .await
            .unwrap();

        let claimed_user_id = 7_654_321_002_i64;
        let claimed_db = migrated_pool().await;
        insert_user(&claimed_db, claimed_user_id).await;
        insert_active_month_card(&claimed_db, claimed_user_id).await;
        sqlx::query(
            "INSERT INTO user_month_card_days (user_id, server_day, day_of_month) VALUES (?1, ?2, 1)",
        )
        .bind(claimed_user_id)
        .bind(ServerTime::server_day(ServerTime::now_ms()))
        .execute(&claimed_db)
        .await
        .unwrap();
        let (claimed_ctx, _claimed_client) = context(
            claimed_db,
            Some(claimed_user_id),
            Some(PlayerState::new(claimed_user_id, ServerTime::now_ms())),
        )
        .await;
        can_claim_month_card_with_lookup(claimed_ctx, claimed_user_id, |_| None)
            .await
            .unwrap();

        let skipped_user_id = 7_654_321_003_i64;
        let skipped_db = migrated_pool().await;
        insert_user(&skipped_db, skipped_user_id).await;
        insert_active_month_card(&skipped_db, skipped_user_id).await;
        let mut skipped_state = PlayerState::new(skipped_user_id, ServerTime::now_ms());
        skipped_state.last_sign_in_day -= 1;
        let (skipped_ctx, _skipped_client) =
            context(skipped_db, Some(skipped_user_id), Some(skipped_state)).await;
        can_claim_month_card_with_lookup(skipped_ctx, skipped_user_id, |_| None)
            .await
            .unwrap();

        let push_user_id = 7_654_321_004_i64;
        let push_db = migrated_pool().await;
        insert_user(&push_db, push_user_id).await;
        sqlx::query(
            "INSERT INTO currencies (user_id, currency_id, quantity, last_recover_time, expired_time) VALUES (?1, 4, 99, 1, 0)",
        )
        .bind(push_user_id)
        .execute(&push_db)
        .await
        .unwrap();
        let (push_ctx, _push_client) = context(push_db, Some(push_user_id), None).await;
        push::send_currency_change_push(push_ctx, push_user_id, vec![(4, 0)])
            .await
            .unwrap();

        let logs = writer.contents();
        assert!(logs.contains("no expired power items"));
        assert!(logs.contains("Completing initial login"));
        assert!(logs.contains("already claimed month card"));
        assert!(logs.contains("skipping month card claim"));
        assert!(logs.contains("Sent CurrencyChangePush"));
        for user_id in [
            initial_user_id,
            claimed_user_id,
            skipped_user_id,
            push_user_id,
        ] {
            let secret = user_id.to_string();
            assert!(
                !logs.contains(&secret),
                "login initialization log leaked player ID {secret}: {logs}"
            );
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn eligible_month_card_and_expired_item_conversion_log_no_player_ids() {
        let (writer, _guard) = capture_logs();

        let month_user_id = 7_654_321_005_i64;
        let month_db = migrated_pool().await;
        insert_user(&month_db, month_user_id).await;
        insert_active_month_card(&month_db, month_user_id).await;
        let (month_ctx, _month_client) = context(
            month_db.clone(),
            Some(month_user_id),
            Some(PlayerState::new(month_user_id, ServerTime::now_ms())),
        )
        .await;
        can_claim_month_card_with_lookup(month_ctx, month_user_id, |card_id| {
            (card_id == 610001).then(|| "2#2#90".to_string())
        })
        .await
        .unwrap();
        let claimed_days: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM user_month_card_days WHERE user_id = ?1")
                .bind(month_user_id)
                .fetch_one(&month_db)
                .await
                .unwrap();
        let month_currency: i32 = sqlx::query_scalar(
            "SELECT quantity FROM currencies WHERE user_id = ?1 AND currency_id = 2",
        )
        .bind(month_user_id)
        .fetch_one(&month_db)
        .await
        .unwrap();
        assert_eq!(claimed_days, 1);
        assert_eq!(month_currency, 90);

        let expired_user_id = 7_654_321_006_i64;
        let expired_db = migrated_pool().await;
        insert_user(&expired_db, expired_user_id).await;
        sqlx::query(
            "INSERT INTO currencies (user_id, currency_id, quantity, last_recover_time, expired_time) VALUES (?1, 4, 10, 1, 0)",
        )
        .bind(expired_user_id)
        .execute(&expired_db)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO power_items (uid, user_id, item_id, quantity, expire_time, created_at) VALUES (7654000006, ?1, 10, 2, ?2, 1)",
        )
        .bind(expired_user_id)
        .bind(ServerTime::now_ms() / 1000 - 1)
        .execute(&expired_db)
        .await
        .unwrap();
        let mut expired_state = PlayerState::new(expired_user_id, ServerTime::now_ms());
        expired_state.initial_login_complete = true;
        let (expired_ctx, _expired_client) = context(
            expired_db.clone(),
            Some(expired_user_id),
            Some(expired_state),
        )
        .await;
        on_auto_use_expire_power_item_with_lookups(
            expired_ctx,
            request(3),
            |item_id| (item_id == 10).then_some(60),
            |_| None,
        )
        .await
        .unwrap();
        let expired_currency: i32 = sqlx::query_scalar(
            "SELECT quantity FROM currencies WHERE user_id = ?1 AND currency_id = 4",
        )
        .bind(expired_user_id)
        .fetch_one(&expired_db)
        .await
        .unwrap();
        let expired_items: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM power_items WHERE user_id = ?1")
                .bind(expired_user_id)
                .fetch_one(&expired_db)
                .await
                .unwrap();
        assert_eq!(expired_currency, 130);
        assert_eq!(expired_items, 0);

        let logs = writer.contents();
        assert!(logs.contains("Auto-claiming month card daily bonus"));
        assert!(logs.contains("auto-converted 1 expired power items into 120 stamina"));
        assert!(logs.contains("Sent CurrencyChangePush"));
        for user_id in [month_user_id, expired_user_id] {
            let secret = user_id.to_string();
            assert!(
                !logs.contains(&secret),
                "eligible login initialization log leaked player ID {secret}: {logs}"
            );
        }
    }
}
