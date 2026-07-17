use crate::error::AppError;
use crate::state::ConnectionContext;
use database::db::game::destiny::CommittedDestinyChange;
use database::models::game::destiny::ProgressionError;
use database::models::game::heros::{HeroModel, UserHeroModel};
use prost::Message;
use sonettobuf::{CmdId, CurrencyChangePush, HeroUpdatePush, ItemChangePush};
use std::sync::Arc;
use tokio::sync::Mutex;

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
            hero_updates: vec![updated_hero.into()],
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
