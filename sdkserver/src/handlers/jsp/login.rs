use crate::AppState;
use crate::models::request::JspLoginQuery;
use crate::models::response::{JspLoginRsp, ZoneInfo};
use axum::{
    extract::{Query, State},
    response::Json,
};
use common::account::parse_user_id;
use sqlx::Row;

pub async fn get(
    State(state): State<AppState>,
    Query(params): Query<JspLoginQuery>,
) -> Json<JspLoginRsp> {
    tracing::info!("JSP login request");

    let Some(user_id) = parse_user_id(&params.account_id) else {
        tracing::warn!("JSP login rejected invalid account ID format");
        return Json(JspLoginRsp {
            result_code: 1,
            ..Default::default()
        });
    };

    // Fetch token and account_tags from database
    let user = sqlx::query("SELECT token, account_tags FROM users WHERE id = ?1")
        .bind(user_id)
        .fetch_optional(&state.game.db)
        .await
        .ok()
        .flatten();

    match user {
        Some(row) => {
            let token: String = row.try_get("token").unwrap_or_else(|_| {
                tracing::error!("JSP login user record has no token");
                "invalid-token".to_string()
            });

            let account_tags: String = row
                .try_get::<Option<String>, _>("account_tags")
                .ok()
                .flatten()
                .unwrap_or_default();

            tracing::info!("JSP login successful");

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
            tracing::warn!("JSP login user not found");
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

    async fn state() -> AppState {
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

        AppState {
            sdk: SdkState {
                http_client: Client::new(),
            },
            game: Arc::new(GameState::new(db)),
        }
    }

    fn params(account_id: &str) -> JspLoginQuery {
        JspLoginQuery {
            sl_session_id: "token".to_string(),
            client_version: "3.8.0".to_string(),
            sys_type: 2,
            account_id: account_id.to_string(),
            channel_id: "100".to_string(),
            sub_channel_id: "1000".to_string(),
        }
    }

    #[tokio::test]
    async fn cn_login_preserves_observed_decimal_account_id() {
        let state = state().await;
        let params = JspLoginQuery {
            sl_session_id: "token".to_string(),
            client_version: "3.8.0".to_string(),
            sys_type: 2,
            account_id: "42".to_string(),
            channel_id: "100".to_string(),
            sub_channel_id: "1000".to_string(),
        };

        let response = get(State(state), Query(params)).await;

        assert_eq!(response.result_code, 0);
        assert_eq!(response.user_name, "42");
    }

    #[tokio::test]
    async fn cn_login_rejects_unproven_channel_prefix() {
        let response = get(State(state().await), Query(params("100_42"))).await;

        assert_eq!(response.result_code, 1);
    }
}
