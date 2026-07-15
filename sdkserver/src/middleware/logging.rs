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
        body::Body,
        http::{Method, Request, StatusCode, header},
        middleware,
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
            .connect_lazy("sqlite::memory:")
            .unwrap();

        AppState {
            sdk: SdkState {
                http_client: Client::new(),
            },
            game: Arc::new(GameState::new(db)),
        }
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
        let device_id = "device-identifier-secret";
        let device_name = "device-name-secret";
        let app = account_router()
            .merge(jsp_router())
            .merge(trade_router())
            .with_state(state);
        let login_request = json!({
            "deviceInfo": device_info(device_id, device_name),
            "appPackageInfo": app_package_info(),
            "reactivate": false,
            "account": account,
            "pwd": "password-secret"
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
        let token = "token-secret-value";
        let token_prefix = token.chars().take(8).collect::<String>();
        let verify_request = json!({
            "deviceInfo": device_info(device_id, device_name),
            "appPackageInfo": app_package_info(),
            "userId": "1337",
            "token": token,
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
        app.clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/loadzone.jsp?sessionId={token}&zoneId=4"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        app.clone()
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
        for secret in [account, device_id, device_name, token_prefix.as_str()] {
            assert!(
                !logs.contains(secret),
                "handler log leaked {secret}: {logs}"
            );
        }
    }
}
