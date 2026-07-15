use axum::{Json, http::StatusCode};
use serde::Deserialize;
use serde_json::{Value, json};

const CN_GAME_ID: i64 = 50001;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffUpdateResourceReq {
    game_id: i64,
}

pub async fn post(Json(request): Json<DiffUpdateResourceReq>) -> Result<Json<Value>, StatusCode> {
    if request.game_id != CN_GAME_ID {
        return Err(StatusCode::BAD_REQUEST);
    }

    Ok(Json(json!({
        "code": 200,
        "msg": "ok",
        "data": {
            "forceUpdate": false,
            "diffPackage": null
        }
    })))
}
