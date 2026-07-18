mod destiny_level_up;
mod destiny_rank_up;
mod destiny_stone_unlock;
mod destiny_stone_use;
mod protocol;

pub use destiny_level_up::on_destiny_level_up;
pub use destiny_rank_up::on_destiny_rank_up;
pub use destiny_stone_unlock::on_destiny_stone_unlock;
pub use destiny_stone_use::on_destiny_stone_use;
#[doc(hidden)]
#[allow(unused_imports)]
pub use protocol::send_destiny_success;
