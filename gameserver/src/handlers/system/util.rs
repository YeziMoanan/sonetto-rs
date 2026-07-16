use crate::error::{AppError, PacketError};
use crate::state::ConnectionContext;
use byteorder::{BE, ByteOrder};
use common::account::parse_user_id;
use sonettobuf::CmdId;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug)]
pub struct LoginRequest {
    pub account_id: String,
    pub token: String,
}

pub fn parse_login_request(data: &[u8]) -> Result<LoginRequest, AppError> {
    if data.len() < 2 {
        return Err(AppError::Packet(PacketError::Custom(
            "LoginRequest too short".into(),
        )));
    }

    let account_id_len = BE::read_u16(&data[0..2]) as usize;

    tracing::debug!(
        "Login packet - Length prefix: {}, Total packet size: {}",
        account_id_len,
        data.len()
    );

    if data.len() < 2 + account_id_len {
        return Err(AppError::Packet(PacketError::Custom(
            "LoginRequest length mismatch".into(),
        )));
    }

    // Parse the account_id part
    let account_str = std::str::from_utf8(&data[2..2 + account_id_len])?;

    // Check if there's more data after account_id (the token might be separate)
    let remaining_data = &data[2 + account_id_len..];
    tracing::debug!(
        remaining_length = remaining_data.len(),
        "Login packet trailing data"
    );

    // Try to parse token from remaining data
    let token = if remaining_data.len() >= 2 {
        let token_len = BE::read_u16(&remaining_data[0..2]) as usize;
        if remaining_data.len() >= 2 + token_len {
            let token_str = std::str::from_utf8(&remaining_data[2..2 + token_len])?;
            tracing::debug!(token_length = token_len, "Parsed separate login token");
            token_str.to_string()
        } else {
            tracing::warn!(
                expected_token_length = token_len,
                available_token_bytes = remaining_data.len().saturating_sub(2),
                "Token length mismatch"
            );
            String::new()
        }
    } else if account_str.contains('#') || account_str.contains('$') {
        // Token might be in the same string
        let separator = if account_str.contains('#') { '#' } else { '$' };
        let parts: Vec<&str> = account_str.splitn(2, separator).collect();
        if parts.len() == 2 {
            tracing::debug!(token_length = parts[1].len(), "Parsed embedded login token");
            parts[1].to_string()
        } else {
            String::new()
        }
    } else {
        tracing::warn!("No token found in packet");
        String::new()
    };

    let account_id = if account_str.contains('#') || account_str.contains('$') {
        let separator = if account_str.contains('#') { '#' } else { '$' };
        account_str.split(separator).next().unwrap_or(account_str)
    } else {
        account_str
    }
    .to_string();

    Ok(LoginRequest { account_id, token })
}

pub fn extract_user_id(account_id: &str) -> Result<i64, AppError> {
    parse_user_id(account_id).ok_or_else(|| AppError::Custom("Invalid account_id format".into()))
}

pub fn build_login_reply(user_id: i64) -> Vec<u8> {
    let mut payload = Vec::new();

    // Reason string (custom binary format: u16 length + string bytes)
    let reason = "OK";
    let reason_bytes = reason.as_bytes();
    let reason_len = reason_bytes.len() as u16;

    payload.extend_from_slice(&reason_len.to_be_bytes());
    payload.extend_from_slice(reason_bytes);

    // user_id as i64 (ReadLong expects signed)
    payload.extend_from_slice(&user_id.to_be_bytes());

    payload
}

pub fn build_login_error(reason: &str) -> Vec<u8> {
    let mut payload = Vec::new();

    let reason_bytes = reason.as_bytes();
    let reason_len = reason_bytes.len() as u16;

    payload.extend_from_slice(&reason_len.to_be_bytes());
    payload.extend_from_slice(reason_bytes);

    // user_id = 0 for failed login
    payload.extend_from_slice(&0i64.to_be_bytes());

    payload
}

/// Load critters from database and send push
pub async fn send_critter_push(
    ctx: Arc<Mutex<ConnectionContext>>,
    user_id: i64,
) -> Result<(), AppError> {
    let critters = {
        let conn = ctx.lock().await;
        database::db::game::critters::get_player_critters(&conn.state.db, user_id)
            .await
            .unwrap_or_default()
    };

    let mut conn = ctx.lock().await;
    let push = sonettobuf::CritterInfoPush {
        critter_infos: critters.into_iter().map(Into::into).collect(),
    };
    conn.notify(CmdId::CritterInfoPushCmd, push).await?;

    Ok(())
}

pub async fn login_error(
    ctx: &Arc<Mutex<ConnectionContext>>,
    msg: &str,
    up_tag: u8,
) -> Result<(), AppError> {
    let mut ctx = ctx.lock().await;
    let payload = build_login_error(msg);
    ctx.send_raw_reply_fixed(CmdId::LoginRequestCmd, payload, 1, up_tag)
        .await?;
    Err(AppError::Custom(msg.to_string()))
}

#[cfg(test)]
mod tests {
    use super::{extract_user_id, parse_login_request};
    use std::{
        io::{self, Write},
        sync::{Arc, Mutex},
    };
    use tracing_subscriber::fmt::MakeWriter;

    #[derive(Clone, Default)]
    struct CaptureWriter(Arc<Mutex<Vec<u8>>>);

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

    fn login_payload(account_id: &str, token: &str) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&(account_id.len() as u16).to_be_bytes());
        payload.extend_from_slice(account_id.as_bytes());
        payload.extend_from_slice(&(token.len() as u16).to_be_bytes());
        payload.extend_from_slice(token.as_bytes());
        payload
    }

    #[test]
    fn tcp_login_parser_logs_metadata_without_packet_account_or_token() {
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
        let _guard = tracing::dispatcher::set_default(&dispatch);
        let account_id = "4242424242";
        let token = "tcp-token-secret";
        let payload = login_payload(account_id, token);

        let parsed = parse_login_request(&payload).unwrap();

        assert_eq!(parsed.account_id, account_id);
        assert_eq!(parsed.token, token);
        let logs = writer.contents();
        for secret in [
            account_id,
            token,
            "Full packet hex",
            "Account string",
            "hex:",
        ] {
            assert!(
                !logs.contains(secret),
                "TCP login log leaked {secret}: {logs}"
            );
        }
        assert!(logs.contains("Login packet"));
        assert!(logs.contains(&payload.len().to_string()));
    }

    #[test]
    fn tcp_account_id_accepts_observed_decimal_contract() {
        assert_eq!(extract_user_id("4242424242").unwrap(), 4_242_424_242);
    }

    #[test]
    fn tcp_account_id_rejects_unproven_channel_prefix() {
        assert!(extract_user_id("100_42").is_err());
    }

    #[test]
    fn invalid_tcp_account_error_does_not_echo_account() {
        let account_id = "invalid-account-secret";

        let error = extract_user_id(account_id).unwrap_err().to_string();

        assert!(!error.contains(account_id), "error leaked account: {error}");
    }
}
