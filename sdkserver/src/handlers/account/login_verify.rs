use super::helpers::*;
use crate::AppState;
use crate::models::request::AccountLoginVerifyReq;
use crate::models::response::{AccountLoginVerifyRsp, AccountLoginVerifyRspData, VerifyUserInfo};
use axum::{extract::State, response::Json};

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

    // Parse user_id from string
    let user_id: i64 = match req.user_id.parse() {
        Ok(id) => id,
        Err(_) => {
            tracing::error!("Invalid user_id format: {}", req.user_id);
            return Json(create_verify_error_response(channel_id));
        }
    };

    tracing::debug!("Login verify request - User ID: {}", user_id);

    // Validate token and get user
    let user = match get_user_with_token_validation(&state, user_id, &req.token).await {
        Ok(user) => user,
        Err(e) => {
            tracing::warn!("Login verify failed: {}", e);
            return Json(create_verify_error_response(channel_id));
        }
    };

    let expires_in = calculate_expires_in(user.token_expires_at);
    let register_time = format_timestamp(user.created_at);
    let first_join_time = format_timestamp(user.last_login_at);

    tracing::info!("Login verify successful for user {}", user_id);

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
    use super::post;
    use crate::{AppState, SdkState};
    use axum::extract::State;
    use gameserver::state::AppState as GameState;
    use reqwest::Client;
    use serde_json::json;
    use sqlx::sqlite::SqlitePoolOptions;
    use std::sync::Arc;

    fn state() -> AppState {
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
}
