use crate::error::AppError;
use crate::network::packet::ClientPacket;
use crate::state::ConnectionContext;
use database::models::game::heros::{HeroModel, UserHeroModel};
use prost::Message;
use sonettobuf::{CmdId, DestinyRankUpReply, DestinyRankUpRequest, HeroUpdatePush};
use std::sync::Arc;
use tokio::sync::Mutex;

pub async fn on_destiny_rank_up(
    ctx: Arc<Mutex<ConnectionContext>>,
    req: ClientPacket,
) -> Result<(), AppError> {
    let request = DestinyRankUpRequest::decode(&req.data[..])?;
    let hero_id = request.hero_id.ok_or(AppError::InvalidRequest)?;

    let (updated_hero, new_rank, new_level) = {
        let conn = ctx.lock().await;
        let player_id = conn.player_id.ok_or(AppError::NotLoggedIn)?;
        let hero = UserHeroModel::new(player_id, conn.state.db.clone());
        let (new_rank, new_level) = hero.destiny_rank_up(hero_id).await?;
        let updated_hero = hero.get(hero_id).await?.into();

        (updated_hero, new_rank, new_level)
    };

    tracing::info!(
        "Ranked up destiny for hero {} to rank {} level {}",
        hero_id,
        new_rank,
        new_level
    );

    let mut conn = ctx.lock().await;
    conn.notify(
        CmdId::HeroHeroUpdatePushCmd,
        HeroUpdatePush {
            hero_updates: vec![updated_hero],
        },
    )
    .await?;
    conn.send_reply(
        CmdId::DestinyRankUpCmd,
        DestinyRankUpReply {
            hero_id: Some(hero_id),
        },
        0,
        req.up_tag,
    )
    .await?;

    Ok(())
}
