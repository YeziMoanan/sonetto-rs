use crate::AppState;
use crate::models::response::{AccountLoginRsp, AccountLoginRspData, AccountType, RealNameInfo};
use axum::{Json, extract::State};
use common::time::ServerTime;
use database::db::user::account::{TokenInfo, handle_user_login};
use serde_json::Value;

pub fn visitor_credentials(device_id: &str) -> (String, String) {
    let normalized = device_id.trim();
    let suffix = if normalized.is_empty() {
        "anonymous"
    } else {
        normalized
    };

    (
        format!("visitor_{}@local.sonetto", suffix),
        format!("visitor:{}", suffix),
    )
}

fn device_id_from_payload(payload: &Value) -> &str {
    payload
        .get("deviceInfo")
        .and_then(|device| device.get("deviceId"))
        .and_then(Value::as_str)
        .unwrap_or("anonymous")
}

pub async fn post(State(state): State<AppState>, Json(req): Json<Value>) -> Json<AccountLoginRsp> {
    let device_id = device_id_from_payload(&req);
    let (email, password) = visitor_credentials(device_id);
    let now = ServerTime::now_ms() as i64;
    let expires_in = 7 * 24 * 60 * 60;

    let token = super::helpers::generate_token();
    let refresh_token = super::helpers::generate_token();
    let token_info = TokenInfo {
        token: token.clone(),
        refresh_token: refresh_token.clone(),
        expires_at: now + (expires_in * 1000),
    };

    tracing::info!("Visitor login attempt - device_id={}", device_id);

    let user =
        match handle_user_login(&state.game.db, &email, &password, token_info, now).await {
            Ok(user) => user,
            Err(e) => {
                tracing::warn!("Visitor login failed for {}: {}", email, e);
                return Json(error_response("Visitor login failed"));
            }
        };

    tracing::info!(
        "Visitor login successful - user_id={}, account={}",
        user.id,
        email
    );

    Json(AccountLoginRsp {
        code: 200,
        msg: "success".to_string(),
        data: AccountLoginRspData {
            token,
            expires_in,
            refresh_token,
            user_id: user.id as u64,
            account_type: AccountType::Email,
            registration_account_type: 1,
            account: email,
            real_name_info: RealNameInfo {
                need_real_name: user.need_real_name,
                real_name_status: user.real_name_status,
                age: user.age as u8,
                adult: user.is_adult,
            },
            need_activate: user.need_activate,
            cipher_mark: user.cipher_mark,
            first_join: user.first_join,
            account_tags: user.account_tags,
        },
    })
}

fn error_response(msg: &str) -> AccountLoginRsp {
    AccountLoginRsp {
        code: 401,
        msg: msg.to_string(),
        data: AccountLoginRspData {
            token: String::new(),
            expires_in: 0,
            refresh_token: String::new(),
            user_id: 0,
            account_type: AccountType::Email,
            registration_account_type: 0,
            account: String::new(),
            real_name_info: RealNameInfo {
                need_real_name: false,
                real_name_status: false,
                age: 0,
                adult: false,
            },
            need_activate: false,
            cipher_mark: false,
            first_join: false,
            account_tags: String::new(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::visitor_credentials;

    #[test]
    fn derives_stable_visitor_credentials_from_device_id() {
        assert_eq!(
            visitor_credentials("1289722253"),
            (
                "visitor_1289722253@local.sonetto".to_string(),
                "visitor:1289722253".to_string()
            )
        );
    }

    #[test]
    fn empty_device_id_uses_anonymous_fallback() {
        assert_eq!(
            visitor_credentials(" "),
            (
                "visitor_anonymous@local.sonetto".to_string(),
                "visitor:anonymous".to_string()
            )
        );
    }

    #[test]
    fn reads_device_id_from_visitor_payload() {
        let payload = serde_json::json!({
            "deviceInfo": {
                "deviceId": "device-1"
            }
        });

        assert_eq!(super::device_id_from_payload(&payload), "device-1");
    }
}
