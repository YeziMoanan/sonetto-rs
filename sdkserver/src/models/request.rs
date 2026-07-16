use serde::Deserialize;

#[allow(dead_code)]
#[derive(Deserialize)]
pub struct GameC2SSessionReq {
    pub timestamp: String,
    pub device_os_version: String,
    pub device_model: String,
    pub app_version: String,
    pub device_ids: Vec<DeviceId>,
    pub request_id: String,
    pub limit_ad_tracking: bool,
}

#[allow(dead_code)]
#[derive(Deserialize)]
pub struct DeviceId {
    #[serde(rename = "type")]
    pub device_id_type: String,
    pub value: String,
}

#[derive(Deserialize)]
pub struct GamePatchVersionReq {
    // query
    pub version: String,
}

#[allow(dead_code)]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub network_name: String,
    pub device_id: String,
    pub cnadid: String,
    pub oa_id: String,
    pub android_id: String,
    pub imsi: String,
    pub imei: String,
    pub uuid: String,
    pub device_name: String,
    pub device_manufacturer: String,
    pub os_type: i64,
    pub os_version: String,
    pub api_level: String,
    pub language: String,
    pub display_width: String,
    pub display_height: String,
    pub hardware: String,
    pub build_name: String,
    pub distinct_id: String,
    pub anonymous_id: String,
}

#[allow(dead_code)]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppPackageInfo {
    pub app_package_name: String,
    pub app_version: i64,
    pub app_version_name: String,
    pub game_id: i64,
    pub game_code: String,
    pub game_name: String,
    pub channel_id: String,
    pub sub_channel_id: String,
    pub app_install_time: String,
    pub app_update_time: String,
    pub app_signature: String,
    pub sdk_version: String,
    pub channel_version: String,
    pub ad_fid: String,
    pub gclid: String,
    pub data_app_id: String,
}

#[allow(dead_code)]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountLoginMailReq {
    pub device_info: DeviceInfo,
    pub app_package_info: AppPackageInfo,
    pub reactivate: bool,
    pub account: String,
    pub pwd: String,
}

#[allow(dead_code)]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountBindListReq {
    pub device_info: DeviceInfo,
    pub app_package_info: AppPackageInfo,
    pub reactivate: bool,
    pub token: String,
    pub user_id: u64,
}

#[allow(dead_code)]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountAutoLoginReq {
    pub device_info: DeviceInfo,
    pub app_package_info: AppPackageInfo,
    pub reactivate: bool,
    pub token: String,
    pub user_id: u64,
    pub account_type: i32,
}

#[allow(dead_code)]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountLoginVerifyReq {
    pub device_info: DeviceInfo,
    pub app_package_info: AppPackageInfo,
    pub user_id: String,
    pub token: String,
    #[serde(default)]
    pub ext_args: serde_json::Value,
}

#[allow(dead_code)]
#[derive(Deserialize)]
pub struct JspLoginQuery {
    #[serde(rename = "slSessionId")]
    pub sl_session_id: String,
    #[serde(rename = "clientVersion")]
    pub client_version: String,
    #[serde(rename = "sysType")]
    pub sys_type: i32,
    #[serde(rename = "accountId")]
    pub account_id: String, // Client-provided positive decimal user ID.
    #[serde(rename = "channelId")]
    pub channel_id: String,
    #[serde(rename = "subChannelId")]
    pub sub_channel_id: String,
}

#[allow(dead_code)]
#[derive(Deserialize)]
pub struct LoadZoneQuery {
    #[serde(rename = "sessionId")]
    pub session_id: String, // This is the token
    #[serde(rename = "zoneId")]
    pub zone_id: i32,
}

#[allow(dead_code)]
#[derive(Deserialize)]
pub struct SummonQueryReq {
    #[serde(rename = "userId")]
    pub user_id: i64,
    #[serde(rename = "token")]
    pub token: String,
}

#[allow(dead_code)]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderReq {
    pub device_info: DeviceInfo,
    pub app_package_info: AppPackageInfo,
    pub user_id: String,
    pub token: String,
    pub role_id: String,
    pub role_name: String,
    pub current_level: i64,
    pub role_vip_lvl: i64,
    pub server_id: String,
    pub server_name: String,
    pub role_establish_time: i64,
    pub role_type: String,
    pub give_currency_num: i64,
    pub paid_currency_num: i64,
    pub currency_num: i64,
    pub amount: i64,
    pub origin_amount: i64,
    pub origin_currency: String,
    pub goods_id: String,
    pub currency: String,
    pub goods_name: String,
    pub goods_desc: String,
    pub game_order_id: String,
    pub pass_back_param: String,
    pub notify_url: String,
    pub timestamp: String,
    pub sign: String,
    pub product_id: String,
    pub current_progress: String,
}

#[allow(dead_code)]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoodListReq {
    pub device_info: DeviceInfo,
    pub app_package_info: AppPackageInfo,
}

#[allow(dead_code)]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentListReq {
    pub device_info: DeviceInfo,
    pub app_package_info: AppPackageInfo,
    pub user_id: String,
    pub language: String,
}

#[allow(dead_code)]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentPageQuery {
    pub timestamp: Option<String>,
    pub goods_name: Option<String>,
    pub amount: Option<String>,
    pub currency: Option<String>,
    pub language: Option<String>,
    pub price: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallbackQuery {
    pub order_key: Option<String>,
    pub payment_status: Option<String>,
    pub payment_amount: Option<String>,
    pub payment_currency: Option<String>,
    pub mac2: Option<String>,
    pub user_id: Option<String>,
    pub foreign_invoice: Option<String>,
    pub invoice_id: Option<String>,
}
