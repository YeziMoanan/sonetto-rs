mod app;

pub mod battle;
mod connection;
mod gacha;
mod packet;
mod player;

pub use app::AppState;
pub use battle::{
    BattleContext, create_battle, end_fight::send_end_fight_push, generate_auto_opers,
    generate_initial_deck, max_ap_for_fight_group, rewards::generate_dungeon_rewards,
    simulator::BattleSimulator,
};
pub use connection::{ActiveBattle, ConnectionContext};
pub use gacha::{
    BannerType, GachaResult, GachaState, build_gacha, get_rewards, grant_dupe_rewards,
    load_gacha_state, parse_item, parse_store_product, save_gacha_state,
};

pub(crate) use packet::CommandPacket;
pub use player::PlayerState;
