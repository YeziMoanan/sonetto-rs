use crate::network::handler;
use crate::state::ConnectionContext;
use byteorder::{BE, ByteOrder};
use std::sync::Arc;
use tokio::{io::AsyncReadExt, sync::Mutex};

pub async fn handle_client(ctx: Arc<Mutex<ConnectionContext>>) -> anyhow::Result<()> {
    loop {
        let packet = {
            let conn = ctx.lock().await;
            let mut socket = conn.socket.lock().await;

            let mut header = [0u8; 4];
            if socket.read_exact(&mut header).await.is_err() {
                tracing::debug!("Client disconnected");
                return Ok(());
            }

            let packet_len = BE::read_i32(&header) as usize;
            let mut buffer = vec![0u8; packet_len];
            if socket.read_exact(&mut buffer).await.is_err() {
                tracing::warn!(packet_length = packet_len, "Failed to read packet body");
                return Ok(());
            }

            let mut packet = Vec::with_capacity(4 + packet_len);
            packet.extend_from_slice(&header);
            packet.extend_from_slice(&buffer);
            packet
        };

        if handler::dispatch_command(ctx.clone(), &packet[..])
            .await
            .is_err()
        {
            tracing::error!("Command dispatch failed");
            break;
        }

        {
            let mut conn = ctx.lock().await;
            if conn.flush_send_queue().await.is_err() {
                tracing::error!("Failed to flush send queue");
                break;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::handle_client;
    use crate::{
        network::packet::ClientPacket,
        state::{AppState, ConnectionContext},
    };
    use sonettobuf::CmdId;
    use sqlx::sqlite::SqlitePoolOptions;
    use std::{
        io::{self, Write},
        sync::{Arc, Mutex as StdMutex},
    };
    use tokio::{
        io::AsyncWriteExt,
        net::{TcpListener, TcpStream},
        sync::Mutex,
        time::{Duration, timeout},
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

    fn context(server: TcpStream) -> Arc<Mutex<ConnectionContext>> {
        let db = SqlitePoolOptions::new()
            .connect_lazy("sqlite::memory:")
            .unwrap();
        Arc::new(Mutex::new(ConnectionContext::new(
            Arc::new(Mutex::new(server)),
            Arc::new(AppState::new(db)),
        )))
    }

    async fn handle_with_timeout(ctx: Arc<Mutex<ConnectionContext>>) {
        timeout(Duration::from_secs(1), handle_client(ctx))
            .await
            .expect("handle_client did not finish after the induced terminal condition")
            .unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_error_paths_log_only_stage_and_packet_length() {
        let (writer, _guard) = capture_logs();

        let (header_client, header_server) = socket_pair().await;
        drop(header_client);
        handle_with_timeout(context(header_server)).await;

        let (mut body_client, body_server) = socket_pair().await;
        body_client.write_all(&8_i32.to_be_bytes()).await.unwrap();
        body_client.write_all(&[0xAA]).await.unwrap();
        body_client.shutdown().await.unwrap();
        handle_with_timeout(context(body_server)).await;

        let (mut dispatch_client, dispatch_server) = socket_pair().await;
        dispatch_client
            .write_all(&[0, 0, 0, 1, 0xAA])
            .await
            .unwrap();
        handle_with_timeout(context(dispatch_server)).await;

        let (mut flush_client, mut flush_server) = socket_pair().await;
        flush_server.shutdown().await.unwrap();
        let request = ClientPacket {
            sequence: 1,
            cmd_id: CmdId::ReadNewAchievementCmd as i16,
            up_tag: 38,
            data: Vec::new(),
        }
        .encode();
        flush_client.write_all(&request).await.unwrap();
        handle_with_timeout(context(flush_server)).await;

        let logs = writer.contents();
        assert!(logs.contains("Client disconnected"));
        assert!(logs.contains("Failed to read packet body"));
        assert!(logs.contains("packet_length=8"));
        assert!(logs.contains("Command dispatch failed"));
        assert!(logs.contains("Failed to flush send queue"));
        for unsafe_marker in [
            "Client disconnected:",
            "bytes):",
            "Dispatch error:",
            "send queue:",
            "early eof",
            "os error",
        ] {
            assert!(
                !logs.contains(unsafe_marker),
                "client log leaked bottom error marker {unsafe_marker}: {logs}"
            );
        }
    }
}
