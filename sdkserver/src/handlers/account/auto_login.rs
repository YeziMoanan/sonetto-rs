use super::helpers::*;
use crate::AppState;
use crate::models::request::AccountAutoLoginReq;
use crate::models::response::AccountLoginRsp;
use axum::{extract::State, response::Json};
use common::time::ServerTime;
use database::db::user::account::{TokenInfo, create_user, update_user_login};

fn should_recover_local_auto_login(error: &anyhow::Error) -> bool {
    let message = error.to_string();
    message.contains("User not found") || message.contains("Invalid token")
}

#[cfg(test)]
mod tests {
    use super::should_recover_local_auto_login;

    #[test]
    fn recovers_missing_or_stale_local_auto_login() {
        assert!(should_recover_local_auto_login(&anyhow::anyhow!(
            "User not found"
        )));
        assert!(should_recover_local_auto_login(&anyhow::anyhow!(
            "Invalid token"
        )));
        assert!(!should_recover_local_auto_login(&anyhow::anyhow!(
            "database is locked"
        )));
    }
}

pub async fn post(
    State(state): State<AppState>,
    axum::Json(req): axum::Json<AccountAutoLoginReq>,
) -> Json<AccountLoginRsp> {
    tracing::info!("Auto-login attempt - User ID: {}", req.user_id);

    // Generate new tokens
    let new_token = generate_token();
    let new_refresh_token = generate_token();
    let now = ServerTime::now_ms();
    let expires_in = 7 * 24 * 60 * 60 * 1000;

    // Update tokens in database
    let token_info = TokenInfo {
        token: new_token.clone(),
        refresh_token: new_refresh_token.clone(),
        expires_at: now + expires_in,
    };

    // Validate token and get user. In local sandbox mode the official client can
    // carry a cached account/token from the upstream service while the local DB
    // is freshly initialized. Recover that state by creating/updating the local
    // account with the client-provided user id, then continue the normal flow.
    let user = match get_user_with_token_validation(&state, req.user_id as i64, &req.token).await {
        Ok(user) => user,
        Err(e) if should_recover_local_auto_login(&e) => {
            let user_id = req.user_id as i64;
            tracing::warn!(
                "Auto-login local recovery for user {} after: {}",
                user_id,
                e
            );

            match get_user_by_id(&state, user_id).await {
                Ok(_) => {
                    if let Err(update_error) =
                        update_user_login(&state.game.db, user_id, &token_info, now).await
                    {
                        tracing::error!(
                            "Failed to refresh recovered local user {}: {}",
                            user_id,
                            update_error
                        );
                        return Json(create_auth_error_response());
                    }
                }
                Err(_) => {
                    let email = format!("cached_{}@local.sonetto", user_id);
                    let password = format!("cached:{}", user_id);
                    if let Err(create_error) = create_user(
                        &state.game.db,
                        user_id,
                        &email,
                        &password,
                        &token_info,
                        now,
                    )
                    .await
                    {
                        tracing::error!(
                            "Failed to create recovered local user {}: {}",
                            user_id,
                            create_error
                        );
                        return Json(create_auth_error_response());
                    }
                }
            }

            match get_user_by_id(&state, user_id).await {
                Ok(user) => user,
                Err(fetch_error) => {
                    tracing::error!(
                        "Failed to fetch recovered local user {}: {}",
                        user_id,
                        fetch_error
                    );
                    return Json(create_auth_error_response());
                }
            }
        }
        Err(e) => {
            tracing::warn!("Auto-login failed: {}", e);
            return Json(create_auth_error_response());
        }
    };

    if let Err(e) = update_user_login(
        &state.game.db,
        user.user_id,
        &token_info,
        now,
    )
    .await
    {
        tracing::error!("Failed to update tokens: {}", e);
    }

    tracing::info!("Auto-login successful for user {}", req.user_id);
    Json(build_login_response(&user, new_token, new_refresh_token))
}
