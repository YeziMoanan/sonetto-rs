use crate::error::AppError;
use crate::network::packet::ClientPacket;
use crate::state::ConnectionContext;
use config::destiny::DestinyConfigIndex;
use database::db::game::destiny::CommittedDestinyChange;
use database::models::game::destiny::ProgressionError;
use database::models::game::heros::{HeroModel, UserHeroModel};
use prost::Message;
use sonettobuf::{CmdId, CurrencyChangePush, HeroUpdatePush, ItemChangePush};
use std::sync::Arc;
use tokio::sync::Mutex;

#[cfg(test)]
mod tests {
    use super::*;
    use database::models::game::destiny::DestinyState;

    #[test]
    fn committed_destiny_snapshot_overrides_stale_hero_fields() {
        let stale = sonettobuf::HeroInfo {
            uid: 7,
            hero_id: 3098,
            destiny_rank: Some(1),
            destiny_level: Some(1),
            destiny_stone: Some(0),
            destiny_stone_unlock: vec![309801],
            ..Default::default()
        };
        let change = CommittedDestinyChange {
            hero_id: 3098,
            state: DestinyState {
                rank: 2,
                level: 1,
                stone: 309801,
            },
            unlocked_stones: vec![309801, 309802],
            items: Vec::new(),
            currencies: Vec::new(),
            changed: true,
        };

        let merged = apply_committed_destiny_snapshot(stale, &change);

        assert_eq!(merged.destiny_rank, Some(2));
        assert_eq!(merged.destiny_level, Some(1));
        assert_eq!(merged.destiny_stone, Some(309801));
        assert_eq!(merged.destiny_stone_unlock, vec![309801, 309802]);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DestinyProtocolFailure {
    Invalid,
    Insufficient,
    Conflict,
    Internal,
}

impl DestinyProtocolFailure {
    pub fn result_code(self) -> u16 {
        match self {
            Self::Invalid => 1,
            Self::Insufficient => 2,
            Self::Conflict => 3,
            Self::Internal => 4,
        }
    }
}

impl From<&ProgressionError> for DestinyProtocolFailure {
    fn from(error: &ProgressionError) -> Self {
        match error {
            ProgressionError::Invalid(_) => Self::Invalid,
            ProgressionError::Insufficient(_) => Self::Insufficient,
            ProgressionError::Conflict => Self::Conflict,
            ProgressionError::Database(_) => Self::Internal,
        }
    }
}

pub(super) async fn decode_destiny_request<M, R>(
    ctx: Arc<Mutex<ConnectionContext>>,
    req: &ClientPacket,
    reply_cmd: CmdId,
) -> Result<Option<M>, AppError>
where
    M: Message + Default,
    R: Message + Default,
{
    match M::decode(&req.data[..]) {
        Ok(request) => Ok(Some(request)),
        Err(error) => {
            tracing::warn!(
                command = ?reply_cmd,
                error = %error,
                "Destiny request payload rejected"
            );
            send_destiny_failure(
                ctx,
                reply_cmd,
                R::default(),
                DestinyProtocolFailure::Invalid,
                req.up_tag,
            )
            .await?;
            Ok(None)
        }
    }
}

pub(super) async fn load_destiny_catalog<R, F>(
    ctx: Arc<Mutex<ConnectionContext>>,
    reply_cmd: CmdId,
    reply: R,
    up_tag: u8,
    loader: F,
) -> Result<Option<DestinyConfigIndex>, AppError>
where
    R: Message,
    F: FnOnce() -> anyhow::Result<DestinyConfigIndex>,
{
    match loader() {
        Ok(catalog) => Ok(Some(catalog)),
        Err(error) => {
            tracing::error!(
                command = ?reply_cmd,
                error = %error,
                "Destiny configuration unavailable"
            );
            send_destiny_failure(
                ctx,
                reply_cmd,
                reply,
                DestinyProtocolFailure::Internal,
                up_tag,
            )
            .await?;
            Ok(None)
        }
    }
}

impl ConnectionContext {
    #[doc(hidden)]
    #[allow(dead_code)]
    pub async fn dispatch_destiny_with_catalog<F>(
        ctx: Arc<Mutex<Self>>,
        req: ClientPacket,
        loader: F,
    ) -> Result<(), AppError>
    where
        F: FnOnce() -> anyhow::Result<DestinyConfigIndex>,
    {
        match req.cmd_id {
            x if x == CmdId::DestinyRankUpCmd as i16 => {
                super::destiny_rank_up::on_destiny_rank_up_with_catalog(ctx, req, loader).await
            }
            x if x == CmdId::DestinyLevelUpCmd as i16 => {
                super::destiny_level_up::on_destiny_level_up_with_catalog(ctx, req, loader).await
            }
            x if x == CmdId::DestinyStoneUnlockCmd as i16 => {
                super::destiny_stone_unlock::on_destiny_stone_unlock_with_catalog(
                    ctx, req, loader,
                )
                .await
            }
            x if x == CmdId::DestinyStoneUseCmd as i16 => {
                super::destiny_stone_use::on_destiny_stone_use_with_catalog(ctx, req, loader).await
            }
            _ => Err(AppError::InvalidRequest),
        }
    }
}

fn apply_committed_destiny_snapshot(
    mut hero: sonettobuf::HeroInfo,
    change: &CommittedDestinyChange,
) -> sonettobuf::HeroInfo {
    hero.destiny_rank = Some(change.state.rank);
    hero.destiny_level = Some(change.state.level);
    hero.destiny_stone = Some(change.state.stone);
    hero.destiny_stone_unlock = change.unlocked_stones.clone();
    hero
}

pub async fn send_destiny_success<R>(
    ctx: Arc<Mutex<ConnectionContext>>,
    player_id: i64,
    change: CommittedDestinyChange,
    reply_cmd: CmdId,
    reply: R,
    up_tag: u8,
) -> Result<(), AppError>
where
    R: Message,
{
    let updated_hero = {
        let conn = ctx.lock().await;
        UserHeroModel::new(player_id, conn.state.db.clone())
            .get(change.hero_id)
            .await?
    };
    let updated_hero = apply_committed_destiny_snapshot(updated_hero.into(), &change);

    let mut conn = ctx.lock().await;
    if !change.items.is_empty() {
        conn.notify(
            CmdId::ItemChangePushCmd,
            ItemChangePush {
                items: change.items.into_iter().map(Into::into).collect(),
                power_items: Vec::new(),
                insight_items: Vec::new(),
            },
        )
        .await?;
    }
    if !change.currencies.is_empty() {
        conn.notify(
            CmdId::CurrencyChangePushCmd,
            CurrencyChangePush {
                change_currency: change.currencies.into_iter().map(Into::into).collect(),
            },
        )
        .await?;
    }
    conn.notify(
        CmdId::HeroHeroUpdatePushCmd,
        HeroUpdatePush {
            hero_updates: vec![updated_hero],
        },
    )
    .await?;
    conn.send_reply(reply_cmd, reply, 0, up_tag).await?;
    Ok(())
}

pub async fn send_destiny_failure<R>(
    ctx: Arc<Mutex<ConnectionContext>>,
    reply_cmd: CmdId,
    reply: R,
    failure: DestinyProtocolFailure,
    up_tag: u8,
) -> Result<(), AppError>
where
    R: Message,
{
    ctx.lock()
        .await
        .send_reply(reply_cmd, reply, failure.result_code() as i16, up_tag)
        .await
}
