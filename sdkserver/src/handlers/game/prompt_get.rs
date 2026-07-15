use crate::models::response::GamePromptGetRsp;
use axum::Json;

pub async fn get() -> Json<GamePromptGetRsp> {
    Json(GamePromptGetRsp::empty_success())
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn prompt_get_returns_empty_success_prompt() {
        let response = super::get().await;
        let body = serde_json::to_value(response.0).unwrap();

        assert_eq!(body["code"], 200);
        assert_eq!(body["msg"], "成功");
        assert_eq!(body["data"]["prompt"], "");
    }
}
