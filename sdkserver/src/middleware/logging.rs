use axum::{
    body::{Body, HttpBody},
    extract::{Query, Request},
    http::StatusCode,
    middleware::Next,
    response::Response,
};
use std::{collections::HashMap, time::Instant};

fn query_keys(req: &Request<Body>) -> Vec<String> {
    let mut keys = Query::<HashMap<String, String>>::try_from_uri(req.uri())
        .map(|Query(query)| query.into_keys().collect::<Vec<_>>())
        .unwrap_or_default();
    keys.sort_unstable();
    keys
}

pub async fn full_logger(req: Request<Body>, next: Next) -> Response<Body> {
    let req_method = req.method().clone();
    let req_path = req.uri().path().to_owned();
    let req_query_keys = query_keys(&req);
    let req_body_length = req.body().size_hint().exact();
    let start_time = Instant::now();

    let response = next.run(req).await;
    let res_status = response.status();
    let res_body_length = response.body().size_hint().exact();
    let duration = start_time.elapsed();

    if res_status == StatusCode::INTERNAL_SERVER_ERROR {
        tracing::error!(
            status = res_status.as_u16(),
            method = %req_method,
            path = %req_path,
            query_keys = ?req_query_keys,
            duration_ms = duration.as_secs_f64() * 1000.0,
            request_length = ?req_body_length,
            response_length = ?res_body_length,
            "SDK request"
        );
    } else {
        tracing::info!(
            status = res_status.as_u16(),
            method = %req_method,
            path = %req_path,
            query_keys = ?req_query_keys,
            duration_ms = duration.as_secs_f64() * 1000.0,
            request_length = ?req_body_length,
            response_length = ?res_body_length,
            "SDK request"
        );
    }

    response
}

#[cfg(test)]
mod tests {
    use super::full_logger;
    use crate::{
        AppState, SdkState,
        handlers::router::{account_router, jsp_router, trade_router},
    };
    use axum::{
        Router,
        body::{Body, to_bytes},
        http::{Method, Request, StatusCode, header},
        middleware,
        response::Response,
        routing::post,
    };
    use gameserver::state::AppState as GameState;
    use reqwest::Client;
    use serde_json::{Value, json};
    use sqlx::sqlite::SqlitePoolOptions;
    use std::{
        io::{self, Write},
        sync::{Arc, Mutex},
    };
    use tower::ServiceExt;
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

    async fn app_state() -> AppState {
        let db = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        database::run_migrations(&db).await.unwrap();

        AppState {
            sdk: SdkState {
                http_client: Client::new(),
            },
            game: Arc::new(GameState::new(db)),
        }
    }

    async fn insert_user(state: &AppState, user_id: i64, email: &str, password: &str, token: &str) {
        let password_hash = bcrypt::hash(password, 4).unwrap();
        sqlx::query(
            r#"INSERT INTO users (
                    id, username, email, password_hash, token, refresh_token,
                    token_expires_at, created_at, updated_at, last_login_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, 'refresh', ?6, 1, 1, 1)"#,
        )
        .bind(user_id)
        .bind(format!("test-user-{user_id}"))
        .bind(email)
        .bind(password_hash)
        .bind(token)
        .bind(i64::MAX)
        .execute(&state.game.db)
        .await
        .unwrap();
    }

    async fn response_json(response: Response) -> Value {
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    async fn auto_login_response(state: AppState, user_id: i64, token: &str) -> Value {
        let response = account_router()
            .with_state(state)
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/login/autologin")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "deviceInfo": device_info("auto-device-secret", "auto-device-name"),
                            "appPackageInfo": app_package_info(),
                            "reactivate": false,
                            "token": token,
                            "userId": user_id,
                            "accountType": 1
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        response_json(response).await
    }

    fn device_info(device_id: &str, device_name: &str) -> Value {
        json!({
            "networkName": "wifi",
            "deviceId": device_id,
            "cnadid": "",
            "oaId": "",
            "androidId": "",
            "imsi": "",
            "imei": "",
            "uuid": "",
            "deviceName": device_name,
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
        })
    }

    fn app_package_info() -> Value {
        json!({
            "appPackageName": "com.shenlan.m.reverse1999",
            "appVersion": 190,
            "appVersionName": "3.8.0",
            "gameId": 50001,
            "gameCode": "reverse1999",
            "gameName": "Reverse: 1999",
            "channelId": "100",
            "subChannelId": "1000",
            "appInstallTime": "0",
            "appUpdateTime": "0",
            "appSignature": "",
            "sdkVersion": "",
            "channelVersion": "",
            "adFid": "",
            "gclid": "",
            "dataAppId": ""
        })
    }

    #[tokio::test(flavor = "current_thread")]
    async fn full_logger_records_metadata_without_sensitive_values() {
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
        let app = Router::new()
            .route(
                "/login",
                post(|| async { (StatusCode::OK, "response-secret") }),
            )
            .layer(middleware::from_fn(full_logger));

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/login?access_token=query-secret&visible=1")
                    .header(header::AUTHORIZATION, "Bearer header-secret")
                    .header(header::COOKIE, "session=cookie-secret")
                    .header("x-sl-info", "sl-info-secret")
                    .header("x-device-id", "device-secret")
                    .body(Body::from("request-body-secret"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let logs = writer.contents();

        for secret in [
            "query-secret",
            "header-secret",
            "cookie-secret",
            "sl-info-secret",
            "device-secret",
            "request-body-secret",
            "response-secret",
        ] {
            assert!(!logs.contains(secret), "log leaked {secret}: {logs}");
        }
        assert!(logs.contains("method=POST"));
        assert!(logs.contains("path=/login"));
        assert!(logs.contains("query_keys=[\"access_token\", \"visible\"]"));
        assert!(logs.contains("status=200"));
        assert!(logs.contains("request_length="));
        assert!(logs.contains("response_length="));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn login_handlers_do_not_log_credentials_or_device_identifiers() {
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
        let state = app_state().await;
        let account = "account-secret@example.invalid";
        let account_password = "password-secret";
        let account_user_id = 8_123_456_789_011_i64;
        let device_id = "device-identifier-secret";
        let device_name = "device-name-secret";
        let visitor_email = format!("visitor_{device_id}@local.sonetto");
        let visitor_password = format!("visitor:{device_id}");
        let visitor_user_id = 8_123_456_789_022_i64;
        let auto_user_id = 8_123_456_789_033_i64;
        let auto_token = "auto-token-secret";
        insert_user(
            &state,
            account_user_id,
            account,
            account_password,
            "mail-token",
        )
        .await;
        insert_user(
            &state,
            visitor_user_id,
            &visitor_email,
            &visitor_password,
            "visitor-token",
        )
        .await;
        insert_user(
            &state,
            auto_user_id,
            "auto-secret@example.invalid",
            "auto-password-secret",
            auto_token,
        )
        .await;
        let app = account_router()
            .merge(jsp_router())
            .merge(trade_router())
            .with_state(state);
        let login_request = json!({
            "deviceInfo": device_info(device_id, device_name),
            "appPackageInfo": app_package_info(),
            "reactivate": false,
            "account": account,
            "pwd": account_password
        });
        let login_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/login/mail")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(login_request.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(login_response.status(), StatusCode::OK);
        assert_eq!(response_json(login_response).await["code"], 200);
        let verify_request = json!({
            "deviceInfo": device_info(device_id, device_name),
            "appPackageInfo": app_package_info(),
            "userId": auto_user_id.to_string(),
            "token": auto_token,
            "extArgs": {}
        });
        let verify_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/login/verify")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(verify_request.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(verify_response.status(), StatusCode::OK);
        assert_eq!(response_json(verify_response).await["code"], 200);
        app.clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/loadzone.jsp?sessionId={auto_token}&zoneId=4"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let visitor_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/visitor/login")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({ "deviceInfo": { "deviceId": device_id } }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response_json(visitor_response).await["code"], 200);
        let auto_login_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/login/autologin")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "deviceInfo": device_info(device_id, device_name),
                            "appPackageInfo": app_package_info(),
                            "reactivate": false,
                            "token": auto_token,
                            "userId": auto_user_id,
                            "accountType": 1
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response_json(auto_login_response).await["code"], 200);
        let failed_login_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/login/mail")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "deviceInfo": device_info(device_id, device_name),
                            "appPackageInfo": app_package_info(),
                            "reactivate": false,
                            "account": account,
                            "pwd": "wrong-password-secret"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response_json(failed_login_response).await["code"], 401);
        let goods_request = json!({
            "deviceInfo": device_info(device_id, device_name),
            "appPackageInfo": app_package_info()
        });
        app.oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/common/pc/goods-list")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(goods_request.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

        let logs = writer.contents();
        for secret in [
            account,
            account_password,
            device_id,
            device_name,
            auto_token,
            "wrong-password-secret",
            &account_user_id.to_string(),
            &visitor_user_id.to_string(),
            &auto_user_id.to_string(),
        ] {
            assert!(
                !logs.contains(secret),
                "handler log leaked {secret}: {logs}"
            );
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn new_user_starter_failure_logs_no_account_or_user_identifier() {
        let db = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        database::run_migrations(&db).await.unwrap();
        sqlx::query("DROP TABLE critters")
            .execute(&db)
            .await
            .unwrap();
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
        let user_id = 8_123_456_789_044_i64;
        let email = "new-user-secret@example.invalid";
        let password = "new-user-password-secret";
        let token = "new-user-token-secret";
        let refresh_token = "new-user-refresh-secret";

        let user = database::db::user::account::create_user(
            &db,
            user_id,
            email,
            password,
            &database::db::user::account::TokenInfo {
                token: token.to_string(),
                refresh_token: refresh_token.to_string(),
                expires_at: i64::MAX,
            },
            1,
        )
        .await
        .unwrap();

        assert_eq!(user.id, user_id);
        let logs = writer.contents();
        for secret in [
            user_id.to_string(),
            email.to_string(),
            password.to_string(),
            token.to_string(),
            refresh_token.to_string(),
        ] {
            assert!(
                !logs.contains(&secret),
                "starter log leaked {secret}: {logs}"
            );
        }
        assert!(logs.contains("Failed to load starter data"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn welcome_mail_creation_does_not_log_user_derived_increment_id() {
        let db = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        database::run_migrations(&db).await.unwrap();
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
        let user_id = 456_789_i64;
        let derived_mail_id = 80_000_000_i64 + user_id * 1_000;
        let mut tx = db.begin().await.unwrap();

        database::db::starter_data::load_starter_mail(&mut tx, user_id)
            .await
            .unwrap();
        tx.commit().await.unwrap();

        let stored_mail_id: i64 =
            sqlx::query_scalar("SELECT incr_id FROM user_mails WHERE user_id = ?1")
                .bind(user_id)
                .fetch_one(&db)
                .await
                .unwrap();
        assert_eq!(stored_mail_id, derived_mail_id);
        let logs = writer.contents();
        for secret in [user_id.to_string(), derived_mail_id.to_string()] {
            assert!(
                !logs.contains(&secret),
                "welcome mail log leaked derived identifier {secret}: {logs}"
            );
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn auto_login_recovery_paths_log_no_user_identifier_or_error_details() {
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

        let refresh_state = app_state().await;
        let refresh_user_id = 8_123_456_789_055_i64;
        insert_user(
            &refresh_state,
            refresh_user_id,
            "refresh-user@example.invalid",
            "refresh-password",
            "stored-refresh-token",
        )
        .await;
        let refresh_error = format!("refresh-private-{refresh_user_id}");
        sqlx::query(&format!(
            "CREATE TRIGGER fail_auto_refresh BEFORE UPDATE ON users BEGIN SELECT RAISE(FAIL, '{refresh_error}'); END"
        ))
        .execute(&refresh_state.game.db)
        .await
        .unwrap();
        assert_eq!(
            auto_login_response(refresh_state, refresh_user_id, "stale-refresh-token").await["code"],
            401
        );

        let create_state = app_state().await;
        let create_user_id = 8_123_456_789_066_i64;
        sqlx::query(
            "INSERT INTO users (id, username, created_at, updated_at) VALUES (1, ?1, 1, 1)",
        )
        .bind(format!("cached_{create_user_id}"))
        .execute(&create_state.game.db)
        .await
        .unwrap();
        assert_eq!(
            auto_login_response(create_state, create_user_id, "missing-user-token").await["code"],
            401
        );

        let fetch_state = app_state().await;
        let fetch_user_id = 8_123_456_789_077_i64;
        insert_user(
            &fetch_state,
            fetch_user_id,
            "fetch-user@example.invalid",
            "fetch-password",
            "stored-fetch-token",
        )
        .await;
        sqlx::query(
            "CREATE TRIGGER remove_auto_user AFTER UPDATE ON users BEGIN DELETE FROM users WHERE id = NEW.id; END",
        )
        .execute(&fetch_state.game.db)
        .await
        .unwrap();
        assert_eq!(
            auto_login_response(fetch_state, fetch_user_id, "stale-fetch-token").await["code"],
            401
        );

        let success_state = app_state().await;
        let success_user_id = 8_123_456_789_088_i64;
        sqlx::query("DROP TABLE critters")
            .execute(&success_state.game.db)
            .await
            .unwrap();
        assert_eq!(
            auto_login_response(success_state, success_user_id, "missing-success-token").await["code"],
            200
        );

        let logs = writer.contents();
        for secret in [
            refresh_user_id.to_string(),
            create_user_id.to_string(),
            fetch_user_id.to_string(),
            success_user_id.to_string(),
            refresh_error,
            "auto-device-secret".to_string(),
            "auto-device-name".to_string(),
            "stale-refresh-token".to_string(),
            "missing-user-token".to_string(),
            "stale-fetch-token".to_string(),
            "missing-success-token".to_string(),
        ] {
            assert!(
                !logs.contains(&secret),
                "auto-login log leaked {secret}: {logs}"
            );
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn jsp_handlers_log_metadata_without_accounts_tokens_or_usernames() {
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
        let db = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, token TEXT, account_tags TEXT, username TEXT, level INTEGER)",
        )
        .execute(&db)
        .await
        .unwrap();
        let account_id = "4242424242";
        let token = "jsp-token-secret";
        let username = "jsp-account-secret";
        sqlx::query(
            "INSERT INTO users (id, token, account_tags, username, level) VALUES (?1, ?2, '', ?3, 1)",
        )
        .bind(4_242_424_242_i64)
        .bind(token)
        .bind(username)
        .execute(&db)
        .await
        .unwrap();
        let state = AppState {
            sdk: SdkState {
                http_client: Client::new(),
            },
            game: Arc::new(GameState::new(db)),
        };
        let app = jsp_router()
            .with_state(state)
            .layer(middleware::from_fn(full_logger));

        let login_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/login.jsp?slSessionId={token}&clientVersion=3.8.0&sysType=2&accountId={account_id}&channelId=100&subChannelId=1000"
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(login_response.status(), StatusCode::OK);
        let load_zone_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/loadzone.jsp?sessionId={token}&zoneId=4"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(load_zone_response.status(), StatusCode::OK);
        let invalid_account = "invalid-account-secret";
        let invalid_login_response = app
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/login.jsp?slSessionId={token}&clientVersion=3.8.0&sysType=2&accountId={invalid_account}&channelId=100&subChannelId=1000"
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(invalid_login_response.status(), StatusCode::OK);

        let logs = writer.contents();
        for secret in [account_id, token, username, invalid_account] {
            assert!(!logs.contains(secret), "JSP log leaked {secret}: {logs}");
        }
        assert!(logs.contains("path=/login.jsp"));
        assert!(logs.contains("path=/loadzone.jsp"));
    }
}
