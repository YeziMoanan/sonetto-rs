use crate::error::AppError;
use crate::network::packet::ClientPacket;
use crate::state::ConnectionContext;
use prost::Message;
use sonettobuf::{CmdId, GetFightCardDeckInfoReply, GetFightCardDeckInfoRequest};
use std::sync::Arc;
use tokio::sync::Mutex;

pub async fn on_get_fight_card_deck_info(
    ctx: Arc<Mutex<ConnectionContext>>,
    req: ClientPacket,
) -> Result<(), AppError> {
    let request = GetFightCardDeckInfoRequest::decode(&req.data[..])?;
    tracing::info!("Requested fight card deck type: {:?}", request.r#type);

    ctx.lock()
        .await
        .send_reply(
            CmdId::GetFightCardDeckInfoCmd,
            GetFightCardDeckInfoReply { deck_infos: vec![] },
            0,
            req.up_tag,
        )
        .await?;

    Ok(())
}
