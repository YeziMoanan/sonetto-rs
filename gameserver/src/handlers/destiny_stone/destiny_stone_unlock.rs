use super::protocol::{DestinyProtocolFailure, send_destiny_failure, send_destiny_success};
use crate::error::AppError;
use crate::network::packet::ClientPacket;
use crate::state::ConnectionContext;
use config::destiny::DestinyConfigIndex;
use database::db::game::destiny::execute_destiny_command;
use database::models::game::destiny::DestinyCommand;
use prost::Message;
use sonettobuf::{CmdId, DestinyStoneUnlockReply, DestinyStoneUnlockRequest};
use std::sync::Arc;
use tokio::sync::Mutex;

pub async fn on_destiny_stone_unlock(
    ctx: Arc<Mutex<ConnectionContext>>,
    req: ClientPacket,
) -> Result<(), AppError> {
    let request = DestinyStoneUnlockRequest::decode(&req.data[..])?;
    let hero_id = request.hero_id.ok_or(AppError::InvalidRequest)?;
    let stone_id = request.stone_id.ok_or(AppError::InvalidRequest)?;
    let (player_id, pool) = {
        let conn = ctx.lock().await;
        (
            conn.player_id.ok_or(AppError::NotLoggedIn)?,
            conn.state.db.clone(),
        )
    };
    let catalog = DestinyConfigIndex::try_from_game_db(config::configs::get())?;
    let reply = DestinyStoneUnlockReply {
        hero_id: Some(hero_id),
        stone_id: Some(stone_id),
    };

    match execute_destiny_command(
        &pool,
        player_id,
        &catalog,
        DestinyCommand::UnlockStone { hero_id, stone_id },
    )
    .await
    {
        Ok(change) => {
            send_destiny_success(
                Arc::clone(&ctx),
                player_id,
                change,
                CmdId::DestinyStoneUnlockCmd,
                reply,
                req.up_tag,
            )
            .await
        }
        Err(error) => {
            let failure = DestinyProtocolFailure::from(&error);
            tracing::warn!(
                player_id,
                hero_id,
                stone_id,
                command = "DestinyStoneUnlock",
                failure = ?failure,
                error = %error,
                "Destiny command rejected"
            );
            send_destiny_failure(
                ctx,
                CmdId::DestinyStoneUnlockCmd,
                reply,
                failure,
                req.up_tag,
            )
            .await
        }
    }
}
