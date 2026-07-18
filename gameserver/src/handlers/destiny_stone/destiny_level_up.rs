use super::protocol::{DestinyProtocolFailure, send_destiny_failure, send_destiny_success};
use crate::error::AppError;
use crate::network::packet::ClientPacket;
use crate::state::ConnectionContext;
use config::destiny::DestinyConfigIndex;
use database::db::game::destiny::execute_destiny_command;
use database::models::game::destiny::DestinyCommand;
use prost::Message;
use sonettobuf::{CmdId, DestinyLevelUpReply, DestinyLevelUpRequest};
use std::sync::Arc;
use tokio::sync::Mutex;

pub async fn on_destiny_level_up(
    ctx: Arc<Mutex<ConnectionContext>>,
    req: ClientPacket,
) -> Result<(), AppError> {
    let request = DestinyLevelUpRequest::decode(&req.data[..])?;
    let (Some(hero_id), Some(target_level)) = (request.hero_id, request.level) else {
        return send_destiny_failure(
            ctx,
            CmdId::DestinyLevelUpCmd,
            DestinyLevelUpReply {
                hero_id: request.hero_id,
                level: request.level,
            },
            DestinyProtocolFailure::Invalid,
            req.up_tag,
        )
        .await;
    };
    let (player_id, pool) = {
        let conn = ctx.lock().await;
        (
            conn.player_id.ok_or(AppError::NotLoggedIn)?,
            conn.state.db.clone(),
        )
    };
    let catalog = DestinyConfigIndex::try_from_game_db(config::configs::get())?;

    match execute_destiny_command(
        &pool,
        player_id,
        &catalog,
        DestinyCommand::LevelUp {
            hero_id,
            target_level,
        },
    )
    .await
    {
        Ok(change) => {
            let committed_level = change.state.level;
            send_destiny_success(
                Arc::clone(&ctx),
                player_id,
                change,
                CmdId::DestinyLevelUpCmd,
                DestinyLevelUpReply {
                    hero_id: Some(hero_id),
                    level: Some(committed_level),
                },
                req.up_tag,
            )
            .await
        }
        Err(error) => {
            let failure = DestinyProtocolFailure::from(&error);
            tracing::warn!(
                player_id,
                hero_id,
                target_level,
                command = "DestinyLevelUp",
                failure = ?failure,
                error = %error,
                "Destiny command rejected"
            );
            send_destiny_failure(
                ctx,
                CmdId::DestinyLevelUpCmd,
                DestinyLevelUpReply {
                    hero_id: Some(hero_id),
                    level: Some(target_level),
                },
                failure,
                req.up_tag,
            )
            .await
        }
    }
}
