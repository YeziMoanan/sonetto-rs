use crate::AppState;
use crate::models::request::GoodListReq;
use crate::models::response::{GoodListRsp, GoodListRspData};
use axum::extract::State;
use axum::response::Json;

pub async fn post(
    State(_): State<AppState>,
    axum::Json(_req): axum::Json<GoodListReq>,
) -> Json<GoodListRsp> {
    tracing::info!("Received goods list request");

    let response = GoodListRsp {
        code: 200,
        msg: "success".to_string(),
        data: GoodListRspData {
            country_iso: "US".to_string(),
            goods_info_list: vec![],
        },
    };

    tracing::info!(
        "Returning goods list: country={}, items={}",
        response.data.country_iso,
        response.data.goods_info_list.len()
    );

    Json(response)
}
