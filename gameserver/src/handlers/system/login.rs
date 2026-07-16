use crate::error::AppError;
use crate::handlers::system::util::*;
use crate::network::packet::ClientPacket;
use crate::state::ConnectionContext;
use crate::util::push::send_red_dot_push;
use common::time::ServerTime;
use database::db::game::sign_in;
use sonettobuf::{CmdId, Mail, NewMailPush};
use sqlx::Row;
use std::sync::Arc;
use tokio::sync::Mutex;

pub async fn on_login(
    ctx: Arc<Mutex<ConnectionContext>>,
    req: ClientPacket,
) -> Result<(), AppError> {
    tracing::info!("→ Starting login handler");
    let login = parse_login_request(&req.data)?;
    let user_id = extract_user_id(&login.account_id)?;
    tracing::info!("→ Login attempt parsed");

    let (stored_token, token_expires_at) = {
        let db = {
            let ctx = ctx.lock().await;
            ctx.state.db.clone()
        };
        let row = sqlx::query("SELECT token, token_expires_at FROM users WHERE id = ?")
            .bind(user_id)
            .fetch_optional(&db)
            .await?
            .ok_or_else(|| AppError::Custom("User not found".into()))?;
        (
            row.try_get::<String, _>("token")?,
            row.try_get::<Option<i64>, _>("token_expires_at")?,
        )
    };

    if stored_token != login.token {
        return login_error(&ctx, "Invalid token", req.up_tag).await;
    }

    let now = ServerTime::now_ms();
    if token_expires_at.is_some_and(|exp| now > exp) {
        return login_error(&ctx, "Token expired", req.up_tag).await;
    }

    tracing::info!("✓ Login token validated");

    {
        let mut conn = ctx.lock().await;
        conn.load_player_state(user_id).await?;
    }

    {
        let db = {
            let ctx = ctx.lock().await;
            ctx.state.db.clone()
        };
        let (is_new_day, is_new_week, is_new_month) =
            sign_in::process_daily_login(&db, user_id).await?;
        if is_new_day {
            sign_in::reset_daily_counters(&db, user_id).await?;
        }
        if is_new_week {
            sign_in::reset_weekly_counters(&db, user_id).await?;
        }
        if is_new_month {
            sign_in::reset_monthly_counters(&db, user_id).await?;
        }
    }

    {
        let mut conn = ctx.lock().await;
        let now = ServerTime::now_ms();
        let today = ServerTime::server_day(now);

        conn.update_and_save_player_state(|state| {
            state.last_login_timestamp = Some(now);

            if state.is_new_server_day(now) {
                state.initial_login_complete = false;
                state.last_sign_in_day = today;
                state.last_daily_reset_time = Some(now);
                state.month_card_claimed = false;
                state.last_month_card_claim_timestamp = None;
            }

            if state.is_new_week(now) {
                state.last_weekly_reset_time = Some(now);
            }

            if state.is_new_month(now) {
                state.last_monthly_reset_time = Some(now);
            }

            state.mark_login_complete(now);
            state.last_sign_in_time = Some(now);
        })
        .await?;
    }

    send_red_dot_push(Arc::clone(&ctx), user_id, Some(vec![2218, 2220, 2221])).await?;
    send_red_dot_push(Arc::clone(&ctx), user_id, Some(vec![2240])).await?;
    send_red_dot_push(Arc::clone(&ctx), user_id, Some(vec![2230])).await?;
    send_critter_push(Arc::clone(&ctx), user_id).await?;

    {
        let conn = ctx.lock().await;
        let pool = &conn.state.db;
        let now = ServerTime::now_ms();

        let new_mails: Vec<(
            i64,
            i32,
            String,
            String,
            i32,
            i64,
            String,
            String,
            String,
            String,
            i64,
            i32,
            String,
            String,
        )> = sqlx::query_as(
            "SELECT incr_id, mail_id, params, attachment, state, create_time,
                    sender, title, content, copy, expire_time, sender_type,
                    jump_title, jump
             FROM user_mails
             WHERE user_id = ? AND state = 0 AND (expire_time = 0 OR expire_time > ?)",
        )
        .bind(user_id)
        .bind(now)
        .fetch_all(pool)
        .await?;

        drop(conn);

        for (
            incr_id,
            mail_id,
            params,
            attachment,
            state,
            create_time,
            sender,
            title,
            content,
            copy,
            expire_time,
            sender_type,
            jump_title,
            jump,
        ) in new_mails.clone()
        {
            let mail = Mail {
                incr_id: Some(incr_id as u64),
                mail_id: Some(mail_id as u32),
                params: Some(params),
                attachment: Some(attachment),
                state: Some(state as u32),
                create_time: Some(create_time as u64),
                sender: Some(sender),
                title: Some(title),
                content: Some(content),
                copy: Some(copy),
                expire_time: Some(expire_time as u64),
                sender_type: Some(sender_type),
                jump_title: Some(jump_title),
                jump: Some(jump),
            };

            let mut conn = ctx.lock().await;
            conn.notify(CmdId::NewMailPushCmd, NewMailPush { mail: Some(mail) })
                .await?;
        }

        if !new_mails.is_empty() {
            tracing::info!("Sent {} new mail notifications", new_mails.len());
        }
    }

    {
        let mut conn = ctx.lock().await;
        let payload = build_login_reply(user_id);
        conn.send_raw_reply_fixed(CmdId::LoginRequestCmd, payload, 0, req.up_tag)
            .await?;
    }

    ConnectionContext::register(Arc::clone(&ctx)).await;
    tracing::info!("✓ Login successful");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::on_login;
    use crate::{
        network::packet::ClientPacket,
        state::{AppState, ConnectionContext},
    };
    use common::time::ServerTime;
    use database::db::game::sign_in;
    use sonettobuf::CmdId;
    use sqlx::sqlite::SqlitePoolOptions;
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

    async fn socket_pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (client, server) = tokio::join!(TcpStream::connect(address), listener.accept());
        (client.unwrap(), server.unwrap().0)
    }

    fn login_payload(account_id: &str, token: &str) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&(account_id.len() as u16).to_be_bytes());
        payload.extend_from_slice(account_id.as_bytes());
        payload.extend_from_slice(&(token.len() as u16).to_be_bytes());
        payload.extend_from_slice(token.as_bytes());
        payload
    }

    #[tokio::test(flavor = "current_thread")]
    async fn tcp_login_success_chain_logs_no_account_token_or_player_identifier() {
        let db = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        database::run_migrations(&db).await.unwrap();
        let user_id = 8_987_654_321_011_i64;
        let account_id = user_id.to_string();
        let token = "tcp-success-token-secret";
        let now = ServerTime::now_ms();
        sqlx::query(
            r#"INSERT INTO users (
                    id, username, token, token_expires_at, created_at, updated_at, last_login_at
                ) VALUES (?1, 'tcp-test-user', ?2, ?3, ?4, ?4, ?4)"#,
        )
        .bind(user_id)
        .bind(token)
        .bind(i64::MAX)
        .bind(now)
        .execute(&db)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO player_state (player_id, created_at, updated_at, last_sign_in_time) VALUES (?1, ?2, ?2, ?2)",
        )
        .bind(user_id)
        .bind(now)
        .execute(&db)
        .await
        .unwrap();
        let (_client, server) = socket_pair().await;
        let state = Arc::new(AppState::new(db));
        let ctx = Arc::new(Mutex::new(ConnectionContext::new(
            Arc::new(Mutex::new(server)),
            Arc::clone(&state),
        )));
        let (writer, _guard) = capture_logs();

        on_login(
            Arc::clone(&ctx),
            ClientPacket {
                sequence: 1,
                cmd_id: CmdId::LoginRequestCmd as i16,
                up_tag: 7,
                data: login_payload(&account_id, token),
            },
        )
        .await
        .unwrap();

        assert!(ctx.lock().await.logged_in);
        let logs = writer.contents();
        for secret in [account_id.as_str(), token] {
            assert!(
                !logs.contains(secret),
                "TCP login log leaked {secret}: {logs}"
            );
        }
        assert!(logs.contains("Login successful"));
        state.unregister_session(user_id);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn tcp_sign_in_failure_error_and_caller_log_do_not_echo_player_identifier() {
        let db = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        database::run_migrations(&db).await.unwrap();
        let user_id = 8_987_654_321_022_i64;
        let user_id_text = user_id.to_string();
        let (writer, _guard) = capture_logs();

        let error = sign_in::process_daily_login(&db, user_id)
            .await
            .unwrap_err();
        tracing::error!("Client handler error: {error}");

        assert!(error.to_string().contains("users row missing"));
        assert!(
            !error.to_string().contains(&user_id_text),
            "sign-in error leaked player identifier: {error}"
        );
        let logs = writer.contents();
        assert!(
            !logs.contains(&user_id_text),
            "caller log leaked player identifier: {logs}"
        );
    }
}
