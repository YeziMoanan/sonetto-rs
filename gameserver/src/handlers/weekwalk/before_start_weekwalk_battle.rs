use crate::error::AppError;
use crate::network::packet::ClientPacket;
use crate::state::ConnectionContext;
use sonettobuf::{BeforeStartWeekwalkBattleReply, BeforeStartWeekwalkBattleRequest, CmdId};
use std::sync::Arc;
use tokio::sync::Mutex;

pub async fn on_before_start_weekwalk_battle(
    ctx: Arc<Mutex<ConnectionContext>>,
    req: ClientPacket,
) -> Result<(), AppError> {
    let request = req.decode_message::<BeforeStartWeekwalkBattleRequest>()?;
    let reply = BeforeStartWeekwalkBattleReply {
        element_id: request.element_id,
        layer_id: request.layer_id,
    };

    ctx.lock()
        .await
        .send_reply(CmdId::BeforeStartWeekwalkBattleCmd, reply, 0, req.up_tag)
        .await?;

    Ok(())
}
