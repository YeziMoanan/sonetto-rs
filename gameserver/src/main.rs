use crate::{
    network::client::handle_client,
    state::{AppState, ConnectionContext},
};
use ::config::configs;
use common::{config, excel_data_directory, game_port, host, init_config, init_tracing};
use database::{
    DatabaseSettings, connect_to, db::game::summon::sync_banner_schedule, run_migrations,
};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tracing::info;

mod error;
mod handlers;
mod network;
mod state;
mod util;

fn log_client_connected(_client: SocketAddr) {
    tracing::info!("New client connected");
}

async fn complete_client_session(ctx: Arc<Mutex<ConnectionContext>>, result: anyhow::Result<()>) {
    let conn = ctx.lock().await;
    if let Some(player_id) = conn.player_id {
        if conn.save_current_player_state().await.is_err() {
            tracing::error!("Failed to save player state");
        }

        tracing::warn!("Client disconnected and saved progress");
        conn.state.unregister_session(player_id);
    }
    drop(conn);

    if result.is_err() {
        tracing::error!("Client handler error");
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let config_path = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|p| p.join("config.toml")))
        .unwrap_or_else(|| PathBuf::from("config.toml"));

    let mut cfg = config::ServerConfig::load_or_create(&config_path)?;

    let config_dir = config_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::env::current_dir().unwrap());

    cfg.resolve_paths(&config_dir)?;
    cfg.validate_paths()?;

    info!("Server configuration:");
    info!("  Host: {}:{}", cfg.server.host, cfg.server.game_port);

    init_config(cfg.clone());

    let db_settings = DatabaseSettings {
        db_name: config().database.path.to_string_lossy().to_string(),
    };

    let db = connect_to(&db_settings).await?;
    run_migrations(&db).await?;

    sync_banner_schedule(&db, &cfg.banners).await?;

    info!("Loading game data...");
    configs::init(excel_data_directory().to_str().unwrap())?;
    info!("Game data loaded");

    let state = Arc::new(AppState::new(db));
    let addr = format!("{}:{}", host(), game_port());
    let listener = TcpListener::bind(&addr).await?;
    info!("Listening on tcp://{}", &addr);

    loop {
        let (raw_socket, client) = listener.accept().await?;
        log_client_connected(client);

        let state = state.clone();
        let socket = Arc::new(Mutex::new(raw_socket));

        tokio::spawn(async move {
            let ctx = Arc::new(Mutex::new(ConnectionContext::new(
                socket.clone(),
                state.clone(),
            )));

            let result = handle_client(ctx.clone()).await;
            complete_client_session(ctx, result).await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{complete_client_session, log_client_connected};
    use crate::state::{AppState, ConnectionContext};
    use sqlx::sqlite::SqlitePoolOptions;
    use std::{
        io::{self, Write},
        net::SocketAddr,
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

    #[test]
    fn connection_accept_log_does_not_echo_remote_address() {
        let (writer, _guard) = capture_logs();
        let remote: SocketAddr = "203.0.113.99:54321".parse().unwrap();

        log_client_connected(remote);

        let logs = writer.contents();
        assert!(logs.contains("New client connected"));
        assert!(
            !logs.contains("203.0.113.99"),
            "accept log leaked address: {logs}"
        );
        assert!(!logs.contains("54321"), "accept log leaked port: {logs}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn connection_completion_logs_no_player_identifier_or_error_details() {
        let db = SqlitePoolOptions::new()
            .connect_lazy("sqlite::memory:")
            .unwrap();
        let state = Arc::new(AppState::new(db));
        let (_client, server) = socket_pair().await;
        let mut connection =
            ConnectionContext::new(Arc::new(Mutex::new(server)), Arc::clone(&state));
        let player_id = 8_987_654_321_033_i64;
        let player_id_text = player_id.to_string();
        connection.player_id = Some(player_id);
        let ctx = Arc::new(Mutex::new(connection));
        state.register_session(player_id, Arc::clone(&ctx));
        let (writer, _guard) = capture_logs();

        complete_client_session(
            Arc::clone(&ctx),
            Err(anyhow::anyhow!("private failure for player {player_id}")),
        )
        .await;

        assert!(state.get_connection_context(player_id).is_none());
        let logs = writer.contents();
        assert!(logs.contains("Client disconnected and saved progress"));
        assert!(logs.contains("Client handler error"));
        assert!(
            !logs.contains(&player_id_text),
            "completion log leaked player identifier: {logs}"
        );
        assert!(
            !logs.contains("private failure"),
            "completion log leaked error details: {logs}"
        );
    }
}
