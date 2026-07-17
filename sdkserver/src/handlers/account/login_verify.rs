use super::helpers::*;
use crate::AppState;
use crate::models::request::AccountLoginVerifyReq;
use crate::models::response::{AccountLoginVerifyRsp, AccountLoginVerifyRspData, VerifyUserInfo};
use axum::{extract::State, response::Json};
use common::account::parse_user_id;

const CN_CHANNEL_ID: i32 = 100;
const INTERNATIONAL_GAME_ID: i64 = 60001;
const INTERNATIONAL_CHANNEL_ID: i32 = 200;

fn requested_channel_id(req: &AccountLoginVerifyReq) -> i32 {
    req.app_package_info
        .channel_id
        .parse()
        .ok()
        .filter(|channel_id| *channel_id > 0)
        .unwrap_or_else(|| {
            if req.app_package_info.game_id == INTERNATIONAL_GAME_ID {
                INTERNATIONAL_CHANNEL_ID
            } else {
                CN_CHANNEL_ID
            }
        })
}

pub async fn post(
    State(state): State<AppState>,
    axum::Json(req): axum::Json<AccountLoginVerifyReq>,
) -> Json<AccountLoginVerifyRsp> {
    let channel_id = requested_channel_id(&req);

    let Some(user_id) = parse_user_id(&req.user_id) else {
        tracing::warn!("Login verify rejected invalid user ID format");
        return Json(create_verify_error_response(channel_id));
    };

    tracing::debug!("Login verify request parsed");

    // Validate token and get user
    let user = match get_user_with_token_validation(&state, user_id, &req.token).await {
        Ok(user) => user,
        Err(error) => {
            tracing::warn!(
                failure_kind = %error.failure_kind(),
                "Login verify failed"
            );
            return Json(create_verify_error_response(channel_id));
        }
    };

    let expires_in = calculate_expires_in(user.token_expires_at);
    let register_time = format_timestamp(user.created_at);
    let first_join_time = format_timestamp(user.last_login_at);

    tracing::info!("Login verify successful");

    let rsp = AccountLoginVerifyRsp {
        code: 200,
        msg: "success".to_string(),
        data: AccountLoginVerifyRspData {
            user_info: VerifyUserInfo {
                channel_id,
                user_id: req.user_id, // Return as string (same as request)
                real_name_status: user.real_name_status,
                age: user.age,
                adult: user.is_adult,
                account_tags: user.account_tags,
                bind_account_type_list: vec![user.email],
                first_join_time,
                register_time,
                is_pay_account: true,
                first_join: user.first_join,
            },
            session_id: generate_session_id(),
            token: user.token,
            expires_in,
            refresh_token: user.refresh_token,
        },
    };

    Json(rsp)
}

fn create_verify_error_response(channel_id: i32) -> AccountLoginVerifyRsp {
    AccountLoginVerifyRsp {
        code: 401,
        msg: "Invalid token or user not found".to_string(),
        data: AccountLoginVerifyRspData {
            user_info: VerifyUserInfo {
                channel_id,
                user_id: String::new(),
                real_name_status: false,
                age: 0,
                adult: false,
                account_tags: String::new(),
                bind_account_type_list: vec![],
                first_join_time: String::new(),
                register_time: String::new(),
                is_pay_account: false,
                first_join: false,
            },
            session_id: String::new(),
            token: String::new(),
            expires_in: 0,
            refresh_token: String::new(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{AccountLoginVerifyRsp, post};
    use crate::{AppState, SdkState};
    use axum::extract::State;
    use gameserver::state::AppState as GameState;
    use reqwest::Client;
    use serde_json::{Value, json};
    use sqlx::sqlite::SqlitePoolOptions;
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

    fn state_with_db(db: sqlx::SqlitePool) -> AppState {
        AppState {
            sdk: SdkState {
                http_client: Client::new(),
            },
            game: Arc::new(GameState::new(db)),
        }
    }

    fn state() -> AppState {
        let db = SqlitePoolOptions::new()
            .connect_lazy("sqlite::memory:")
            .unwrap();
        state_with_db(db)
    }

    fn verify_request(game_id: i64, channel_id: &str) -> serde_json::Value {
        json!({
            "deviceInfo": {
                "networkName": "wifi",
                "deviceId": "test-device",
                "cnadid": "",
                "oaId": "",
                "androidId": "",
                "imsi": "",
                "imei": "",
                "uuid": "",
                "deviceName": "MuMu",
                "deviceManufacturer": "MuMu",
                "osType": 2,
                "osVersion": "Android 12",
                "apiLevel": "32",
                "language": "zh-CN",
                "displayWidth": "1920",
                "displayHeight": "1080",
                "hardware": "virtual",
                "buildName": "test",
                "distinctId": "",
                "anonymousId": ""
            },
            "appPackageInfo": {
                "appPackageName": "com.shenlan.m.reverse1999",
                "appVersion": 190,
                "appVersionName": "3.8.0",
                "gameId": game_id,
                "gameCode": "reverse1999",
                "gameName": "Reverse: 1999",
                "channelId": channel_id,
                "subChannelId": "1000",
                "appInstallTime": "0",
                "appUpdateTime": "0",
                "appSignature": "",
                "sdkVersion": "",
                "channelVersion": "",
                "adFid": "",
                "gclid": "",
                "dataAppId": ""
            },
            "userId": "invalid",
            "token": "test-token"
        })
    }

    fn verify_request_with_credentials(user_id: &str, token: &str) -> Value {
        let mut request = verify_request(50001, "100");
        request["userId"] = json!(user_id);
        request["token"] = json!(token);
        request
    }

    #[tokio::test]
    async fn cn_verify_error_uses_cn_channel() {
        let request = serde_json::from_value(verify_request(50001, "")).unwrap();

        let response = post(State(state()), axum::Json(request)).await;

        assert_eq!(response.0.data.user_info.channel_id, 100);
    }

    #[tokio::test]
    async fn international_verify_error_keeps_international_channel() {
        let request = serde_json::from_value(verify_request(60001, "200")).unwrap();

        let response = post(State(state()), axum::Json(request)).await;

        assert_eq!(response.0.data.user_info.channel_id, 200);
    }

    async fn verify_failure_logs(
        state: AppState,
        user_id: &str,
        token: &str,
    ) -> (AccountLoginVerifyRsp, String) {
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

        let request =
            serde_json::from_value(verify_request_with_credentials(user_id, token)).unwrap();
        let response = post(State(state), axum::Json(request)).await;

        (response.0, writer.contents())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn wrong_token_logs_safe_auth_token_category() {
        let token_db = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        database::run_migrations(&token_db).await.unwrap();
        let token_user_id = "912345678901";
        let wrong_token = "wrong-token-secret";
        sqlx::query(
            r#"INSERT INTO users (
                    id, username, email, token, refresh_token, token_expires_at,
                    created_at, updated_at, last_login_at
                ) VALUES (?1, 'verify-user', 'verify@example.invalid',
                    'stored-token', 'stored-refresh', ?2, 1, 1, 1)"#,
        )
        .bind(token_user_id.parse::<i64>().unwrap())
        .bind(i64::MAX)
        .execute(&token_db)
        .await
        .unwrap();
        let (response, logs) =
            verify_failure_logs(state_with_db(token_db), token_user_id, wrong_token).await;

        assert_eq!(response.code, 401);
        assert!(logs.contains("failure_kind=auth_token"), "logs: {logs}");
        for secret in [token_user_id, wrong_token, "stored-token", "Invalid token"] {
            assert!(
                !logs.contains(secret),
                "verify failure log leaked {secret}: {logs}"
            );
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn missing_user_logs_safe_auth_missing_category() {
        let missing_db = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        database::run_migrations(&missing_db).await.unwrap();
        let missing_user_id = "912345678902";
        let missing_token = "missing-token-secret";
        let (response, logs) =
            verify_failure_logs(state_with_db(missing_db), missing_user_id, missing_token).await;

        assert_eq!(response.code, 401);
        assert!(logs.contains("failure_kind=auth_missing"), "logs: {logs}");
        for secret in [missing_user_id, missing_token, "User not found"] {
            assert!(
                !logs.contains(secret),
                "verify failure log leaked {secret}: {logs}"
            );
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn database_error_logs_safe_database_category() {
        let error_db = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let error_user_id = "912345678903";
        let database_secret = format!("db-error-secret-{error_user_id}");
        sqlx::query(&format!(
            "CREATE VIEW users AS SELECT * FROM \"{database_secret}\""
        ))
        .execute(&error_db)
        .await
        .unwrap();
        let database_token = "database-token-secret";
        let (response, logs) =
            verify_failure_logs(state_with_db(error_db), error_user_id, database_token).await;

        assert_eq!(response.code, 401);
        assert!(logs.contains("failure_kind=database"), "logs: {logs}");
        for secret in [
            error_user_id,
            database_secret.as_str(),
            database_token,
            "no such table",
            "SELECT username",
        ] {
            assert!(
                !logs.contains(secret),
                "verify failure log leaked {secret}: {logs}"
            );
        }
    }
}
