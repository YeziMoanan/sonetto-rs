use crate::AppState;
use crate::models::request::JspLoginQuery;
use crate::models::response::{JspLoginRsp, ZoneInfo};
use axum::{
    extract::{Query, State},
    response::Json,
};
use sqlx::Row;

pub async fn get(
    State(state): State<AppState>,
    Query(params): Query<JspLoginQuery>,
) -> Json<JspLoginRsp> {
    tracing::info!("JSP login request - Account ID: {}", params.account_id);

    // Extract user_id from accountId format: "channelId_userId".
    let user_id: u64 = params
        .account_id
        .split('_')
        .last()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    if user_id == 0 {
        tracing::error!("Invalid accountId format: {}", params.account_id);
        return Json(JspLoginRsp {
            result_code: 1,
            ..Default::default()
        });
    }

    tracing::info!("Extracted user_id: {}", user_id);

    // Fetch token and account_tags from database
    let user = sqlx::query("SELECT token, account_tags FROM users WHERE id = ?1")
        .bind(user_id as i64)
        .fetch_optional(&state.game.db)
        .await
        .ok()
        .flatten();

    match user {
        Some(row) => {
            let token: String = row.try_get("token").unwrap_or_else(|_| {
                tracing::error!("No token for user {}", user_id);
                "invalid-token".to_string()
            });

            let account_tags: String = row
                .try_get::<Option<String>, _>("account_tags")
                .ok()
                .flatten()
                .unwrap_or_default();

            tracing::info!("JSP login successful for user {}", user_id);

            let rsp = JspLoginRsp {
                account_tags,
                area_id: 4,
                is_admin: false,
                result_code: 0,
                session_id: token, // Real token from database
                user_name: params.account_id,
                zone_info: ZoneInfo::zone_four(),
            };

            Json(rsp)
        }
        None => {
            tracing::warn!("User {} not found in database", user_id);
            Json(JspLoginRsp {
                result_code: 1,
                session_id: String::new(),
                ..Default::default()
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::get;
    use crate::models::request::JspLoginQuery;
    use crate::{AppState, SdkState};
    use axum::extract::{Query, State};
    use gameserver::state::AppState as GameState;
    use reqwest::Client;
    use sqlx::sqlite::SqlitePoolOptions;
    use std::sync::Arc;

    #[tokio::test]
    async fn cn_login_preserves_cn_account_prefix() {
        let db = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY, token TEXT, account_tags TEXT)")
            .execute(&db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO users (id, token, account_tags) VALUES (42, 'token', '')")
            .execute(&db)
            .await
            .unwrap();

        let state = AppState {
            sdk: SdkState {
                http_client: Client::new(),
            },
            game: Arc::new(GameState::new(db)),
        };
        let params = JspLoginQuery {
            sl_session_id: "token".to_string(),
            client_version: "3.8.0".to_string(),
            sys_type: 2,
            account_id: "100_42".to_string(),
            channel_id: "100".to_string(),
            sub_channel_id: "1000".to_string(),
        };

        let response = get(State(state), Query(params)).await;

        assert_eq!(response.user_name, "100_42");
    }
}
