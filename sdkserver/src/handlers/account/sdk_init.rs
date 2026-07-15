use crate::models::response::{
    AccountSdkInitRsp, AccountSdkInitRspData, BizSwitch, GameChannel, ShowButtons, UserCenterItem,
};
use axum::{extract::Query, response::Json};
use std::collections::HashMap;

const CN_GAME_ID: i32 = 50001;
const CN_CHANNEL_ID: i32 = 100;
const INTERNATIONAL_GAME_ID: i32 = 60001;
const INTERNATIONAL_CHANNEL_ID: i32 = 200;

fn positive_query_value(query: &HashMap<String, String>, key: &str) -> Option<i32> {
    query
        .iter()
        .find(|(candidate, _)| candidate.eq_ignore_ascii_case(key))
        .and_then(|(_, value)| value.parse::<i32>().ok())
        .filter(|value| *value > 0)
}

fn requested_game_channel(query: &HashMap<String, String>) -> (i32, i32) {
    let game_id = match positive_query_value(query, "gameId") {
        Some(INTERNATIONAL_GAME_ID) => INTERNATIONAL_GAME_ID,
        Some(CN_GAME_ID) | None => CN_GAME_ID,
        Some(_) => CN_GAME_ID,
    };
    let default_channel_id = if game_id == INTERNATIONAL_GAME_ID {
        INTERNATIONAL_CHANNEL_ID
    } else {
        CN_CHANNEL_ID
    };
    let channel_id = positive_query_value(query, "channelId").unwrap_or(default_channel_id);

    (game_id, channel_id)
}

pub async fn post(query: Query<HashMap<String, String>>) -> Json<AccountSdkInitRsp> {
    let has_query = !query.is_empty();

    let data = if has_query {
        let (game_id, channel_id) = requested_game_channel(&query);
        AccountSdkInitRspData {
            game_channel: Some(GameChannel {
                game_id,
                channel_id,
                cp_name: "重返未来：1999".to_string(),
                app_id: "1".to_string(),
                app_key: "1".to_string(),
                call_interval: 600,
                relogin_interval: 60,
                relogin_times: 5,
                is_record_debug: false,
            }),
            biz_switch: Some(BizSwitch {
                open_real_name_window: false,
                force_real_name_auth: false,
            }),
            is_download_service: Some(true),
            is_show_stop_service_baffle: Some(false),
            is_ignore_file_missing: Some(false),
            is_open_c_m_p: Some(false),
            show_buttons: Some(ShowButtons { notice: true }),
            login_account_types: None,
            user_center_items: None,
            only_mail: None,
            is_unsupport_change_volume: false,
        }
    } else {
        AccountSdkInitRspData {
            login_account_types: Some(vec![1, 5, 10, 11, 12, 13, 14]), //needed else client doesnt show login???
            user_center_items: Some(vec![
                UserCenterItem {
                    r#type: 1,
                    lab_title: "账号管理".to_string(),
                },
                UserCenterItem {
                    r#type: 2,
                    lab_title: "客服".to_string(),
                },
                UserCenterItem {
                    r#type: 3,
                    lab_title: "隐私条约".to_string(),
                },
                UserCenterItem {
                    r#type: 4,
                    lab_title: "账号注销".to_string(),
                },
            ]),
            only_mail: Some(false),
            is_unsupport_change_volume: false,
            game_channel: None,
            biz_switch: None,
            is_download_service: None,
            is_show_stop_service_baffle: None,
            is_ignore_file_missing: None,
            is_open_c_m_p: None,
            show_buttons: None,
        }
    };

    let rsp = AccountSdkInitRsp {
        code: 200,
        msg: "success".to_string(),
        data,
    };

    Json(rsp)
}

#[cfg(test)]
mod tests {
    use super::post;
    use axum::extract::Query;
    use std::collections::HashMap;

    #[tokio::test]
    async fn cn_query_returns_cn_game_channel() {
        let response = post(Query(HashMap::from([(
            "gameId".to_string(),
            "50001".to_string(),
        )])))
        .await;
        let channel = response.0.data.game_channel.unwrap();

        assert_eq!(channel.game_id, 50001);
        assert_eq!(channel.channel_id, 100);
    }

    #[tokio::test]
    async fn international_query_keeps_international_game_channel() {
        let response = post(Query(HashMap::from([(
            "gameId".to_string(),
            "60001".to_string(),
        )])))
        .await;
        let channel = response.0.data.game_channel.unwrap();

        assert_eq!(channel.game_id, 60001);
        assert_eq!(channel.channel_id, 200);
    }

    #[tokio::test]
    async fn lowercase_query_can_override_channel_id() {
        let response = post(Query(HashMap::from([
            ("gameid".to_string(), "50001".to_string()),
            ("channelid".to_string(), "321".to_string()),
        ])))
        .await;
        let channel = response.0.data.game_channel.unwrap();

        assert_eq!(channel.game_id, 50001);
        assert_eq!(channel.channel_id, 321);
    }

    #[tokio::test]
    async fn nonempty_query_without_game_id_uses_cn_defaults() {
        let response = post(Query(HashMap::from([(
            "osType".to_string(),
            "2".to_string(),
        )])))
        .await;
        let channel = response.0.data.game_channel.unwrap();

        assert_eq!(channel.game_id, 50001);
        assert_eq!(channel.channel_id, 100);
    }
}
