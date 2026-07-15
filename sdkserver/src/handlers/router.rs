use crate::AppState;
use crate::handlers::{account, game, index, jsp, trade};
use axum::Router;
use axum::routing::{get, post};
use paste::paste;

// // Example usage:
// router! {
//     name-of-function-and-module;
//     "/route" get route_handler;
//     "/more-route" post more_route_handler;
// }
//
// // it will then become:
// pub fn name-of-function-and-module -> Router {
//     Router::new()
//         .route("/route" get(name-of-function-and-module::route_handler::get))
//         .route("/more-route" post(name-of-function-and-module::more_route_handler::post))
// }
//

macro_rules! router {
    ($module:ident; $($route:literal $method:ident $handler:ident);* $(;)?) => {
        paste! {
            pub fn [<$module _router>]() -> Router<AppState> {
                Router::new()
                    $(.route($route, $method($module::$handler::$method)))*
            }
        }
    };
}

// these use crypto
router! {
    account;
    "/login/autologin" post auto_login;
    "/uidAccount/bindList" post bind_list;
    "/login/config" post login_config;
    "/login/mail" post login_mail;
    "/login/verify" post login_verify;
    "/visitor/login" post visitor_login;
    "/sdk/init" post sdk_init;
}

router! {
    trade;
    "/trade/order" post order;
    "/common/payment/list" post payment_list;
    "/common/pc/goods-list" post good_list;
}

router! {
    jsp;
    "/loadzone.jsp" get load_zone;
    "/login.jsp" get login;
    "/startgame.jsp" get start_game;
}

router! {
    game;
    "/v1.0/c2s/session/app/nativepc/50001" post c2s_session;
    "/v1.0/c2s/session/app/nativepc/60001" post c2s_session;
    "/config" get config;
    "/noticecp/config" get noticecp_config;
    "/noticecp/client/query" get noticecp_query;
    "/patch/50001/version" get patch_version;
    "/patch/60001/version" get patch_version;
    "/prompt/get" get prompt_get;
    "/receiver/app" post receiver_app;
    "/resource/50001/check" get resource_check;
    "/resource/60001/check" get resource_check;
    "/query/summon" get summon_query;
    "/sdk-pc-pay/pcpay.html" get sdk_pay;
    "/SDKStaticPage/pcpay/callback.html" get sdk_pay_complete;

}

router! {
    index;
    "/" get home;
    "/favicon.ico" get favicon;
}

#[cfg(test)]
mod tests {
    use super::game_router;
    use crate::{AppState, SdkState};
    use axum::body::{Body, to_bytes};
    use axum::http::{Method, Request, StatusCode, header};
    use gameserver::state::AppState as GameState;
    use reqwest::Client;
    use sqlx::sqlite::SqlitePoolOptions;
    use std::sync::Arc;
    use tower::ServiceExt;

    fn app() -> axum::Router {
        let db = SqlitePoolOptions::new()
            .connect_lazy("sqlite::memory:")
            .unwrap();
        let state = AppState {
            sdk: SdkState {
                http_client: Client::new(),
            },
            game: Arc::new(GameState::new(db)),
        };

        game_router().with_state(state)
    }

    #[tokio::test]
    async fn prompt_get_route_returns_empty_success_prompt() {
        let response = app()
            .oneshot(
                Request::builder()
                    .uri("/prompt/get")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["code"], 200);
        assert_eq!(json["data"]["prompt"], "");
    }

    #[tokio::test]
    async fn cn_patch_version_route_echoes_requested_version() {
        let response = app()
            .oneshot(
                Request::builder()
                    .uri("/patch/50001/version?version=3.8.0")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["latestVersion"], "3.8.0");
    }

    #[tokio::test]
    async fn cn_resource_check_route_is_registered_as_get_only() {
        let response = app()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/resource/50001/check")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    #[tokio::test]
    async fn cn_native_session_route_accepts_valid_request() {
        let body = serde_json::json!({
            "timestamp": "0",
            "device_os_version": "Android 12",
            "device_model": "MuMu",
            "app_version": "3.8.0",
            "device_ids": [],
            "request_id": "test-request",
            "limit_ad_tracking": false
        })
        .to_string();
        let response = app()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1.0/c2s/session/app/nativepc/50001")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::ACCEPTED);
    }
}
