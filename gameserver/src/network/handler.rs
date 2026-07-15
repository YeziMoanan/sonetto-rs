use crate::error::AppError;
use crate::handlers::*;
use crate::network::packet::{ClientPacket, CompatibilityCommand};
use crate::state::ConnectionContext;
use sonettobuf::CmdId;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone, Copy)]
enum RawCommandPolicy {
    StaticSuccessReply {
        command: CompatibilityCommand,
        body: &'static [u8],
    },
}

fn raw_command_policy(cmd_id: i16) -> Option<RawCommandPolicy> {
    CompatibilityCommand::from_raw_id(cmd_id).map(|command| RawCommandPolicy::StaticSuccessReply {
        command,
        body: command.success_body(),
    })
}

macro_rules! dispatch {
    ($cmd_id:expr, $ctx:expr, $packet:expr, {
        $($variant:path => $handler:expr),* $(,)?
    }) => {
        match $cmd_id {
            $(
                $variant => $handler($ctx, $packet).await?,
            )*
            v => {
                tracing::warn!("Replying with an error for registered command without a handler: {:?}", v);
                $ctx.lock()
                    .await
                    .send_empty_reply(v, Vec::new(), 1, $packet.up_tag)
                    .await?;
                return Ok(());
            },
        }
    };
}

pub async fn dispatch_command(
    ctx: Arc<Mutex<ConnectionContext>>,
    req: &[u8],
) -> Result<(), AppError> {
    let req = ClientPacket::decode(req)?;

    if let Some(policy) = raw_command_policy(req.cmd_id) {
        match policy {
            RawCommandPolicy::StaticSuccessReply { command, body } => {
                tracing::info!(
                    "Received compatibility command: {} (raw ID {}); replying with static success body ({} bytes)",
                    command.name(),
                    command.raw_id(),
                    body.len()
                );
                ctx.lock()
                    .await
                    .send_compatibility_reply(command, body.to_vec(), 0, req.up_tag)
                    .await?;
            }
        }

        return Ok(());
    }

    let cmd_id = match TryInto::<CmdId>::try_into(req.cmd_id as i32) {
        Ok(cmd_id) => cmd_id,
        Err(_) => {
            tracing::warn!("Ignoring unregistered command ID: {}", req.cmd_id);
            return Ok(());
        }
    };

    tracing::info!("Received Cmd: {:?}", cmd_id);

    dispatch!(cmd_id, ctx, req, {
        // === System ===
        CmdId::LoginRequestCmd => system::on_login,
        CmdId::ReconnectRequestCmd => system::on_reconnect,
        CmdId::RenameCmd => system::on_rename,
        CmdId::UpdateClientStatBaseInfoCmd => stat::on_update_client_stat_base_info,
        CmdId::ClientStatBaseInfoCmd => stat::on_client_stat_base_info,

        // === Common ===
        CmdId::GetServerTimeCmd => common::on_get_server_time,

        // === Player ===
        CmdId::GetPlayerInfoCmd => player::on_get_player_info,
        CmdId::GetClothInfoCmd => player::on_get_cloth_info,
        CmdId::MarkMainThumbnailCmd => misc::on_mark_main_thumbnail,
        CmdId::GetAssistBonusCmd => player::on_get_assist_bonus,
        CmdId::GetPlayerCardInfoCmd => player_card::on_get_player_card_info,
        CmdId::SetPortraitCmd => misc::on_set_portrait,

        // === Hero ===
        CmdId::HeroInfoListCmd => hero::on_hero_info_list,
        CmdId::HeroRedDotReadCmd => hero::on_hero_red_dot_read,
        CmdId::HeroTouchCmd => hero::on_hero_touch,
        CmdId::HeroDefaultEquipCmd => hero::on_hero_default_equip,
        CmdId::MarkHeroFavorCmd => hero::on_mark_hero_favor,
        CmdId::SetShowHeroUniqueIdsCmd => hero::on_set_show_hero_unique_ids,
        CmdId::GetHeroBirthdayCmd => hero::on_get_hero_birthday,
        // special equipment for ezio
        CmdId::ChoiceHero3123WeaponCmd => hero::on_choice_hero_3123_weapon,
        // sets euphoria for heros
        CmdId::DestinyStoneUseCmd => destiny_stone::on_destiny_stone_use,
        CmdId::DestinyRankUpCmd => destiny_stone::on_destiny_rank_up,
        CmdId::HeroUpgradeSkillCmd => hero::on_hero_upgrade_skill,
        CmdId::UnMarkIsNewCmd => hero::on_unmark_is_new,
        CmdId::HeroLevelUpCmd => hero::on_hero_level_up,
        CmdId::HeroRankUpCmd => hero::on_hero_rank_up,

        // === Hero Groups ===
        CmdId::GetHeroGroupCommonListCmd => hero_group::on_get_hero_group_common_list,
        CmdId::GetHeroGroupListCmd => hero_group::on_get_hero_group_list,
        CmdId::GetHeroGroupSnapshotListCmd => hero_group::on_get_hero_group_snapshot_list,
        CmdId::SetHeroGroupEquipCmd => hero_group::on_set_hero_group_equip,
        CmdId::SetHeroGroupSnapshotCmd => hero_group::on_set_hero_group_snapshot,

        // === Currency & Economy ===
        CmdId::GetCurrencyListCmd => currency::on_get_currency_list,
        CmdId::GetBuyPowerInfoCmd => currency::on_get_buy_power_info,

        // === Items & Equipment ===
        CmdId::GetItemListCmd => item::on_get_item_list,
        CmdId::AutoUseExpirePowerItemCmd => item::on_auto_use_expire_power_item,
        CmdId::GetEquipInfoCmd => equip::on_get_equip_info,
        CmdId::UseItemCmd => item::on_use_item,
        CmdId::EquipLockCmd => equip::on_equip_lock,
        CmdId::UseInsightItemCmd => item::on_use_insight_item,
        CmdId::EquipStrengthenCmd => equip::on_equip_strengthen,
        CmdId::EquipBreakCmd => equip::on_equip_break,
        CmdId::EquipRefineCmd => equip::on_equip_refine,

        // === Skin & Cosmetics ===
        CmdId::UseSkinCmd => misc::on_use_skin,

        // === Story & Dialog ===
        CmdId::GetStoryCmd => story::on_get_story,
        CmdId::UpdateStoryCmd => story::on_update_story,
        CmdId::GetDialogInfoCmd => dialog::on_get_dialog_info,
        CmdId::GetNecrologistStoryCmd => necro_story::on_get_necrologist_story,
        CmdId::GetHeroStoryCmd => hero_story::on_get_hero_story,

        // === Dungeons & Combat ===
        CmdId::GetDungeonCmd => dungeon::on_get_dungeon,
        CmdId::DungeonInstructionDungeonInfoCmd => dungeon::on_instruction_dungeon_info,
        CmdId::StartDungeonCmd => dungeon::on_start_dungeon,
        CmdId::BeginRoundCmd => dungeon::on_begin_round,
        CmdId::AutoRoundCmd => dungeon::on_auto_round,
        CmdId::FightEndFightCmd => dungeon::on_fight_end_fight,
        CmdId::GetFightRecordGroupCmd => dungeon::on_get_fight_record_group,
        CmdId::GetFightOperCmd => dungeon::on_get_fight_oper,
        CmdId::ChangeHeroGroupSelectCmd => dungeon::on_change_hero_group_select,
        CmdId::DungeonEndDungeonCmd => dungeon::on_dungeon_end_dungeon,
        CmdId::ReconnectFightCmd => fight::on_reconnect_fight,
        CmdId::GetFightCardDeckInfoCmd => fight::on_get_fight_card_deck_info,

        // === Tower ===
        CmdId::GetTowerInfoCmd => tower::on_get_tower_info,
        CmdId::StartTowerBattleCmd => tower::on_start_tower_battle,

        // === Exploration ===
        CmdId::GetExploreSimpleInfoCmd => explore::on_get_explore_simple_info,

        // === Rouge ===
        CmdId::GetRougeOutsideInfoCmd => rouge::on_get_rouge_outside_info, // need to implement / static data for now

        // === Room & Building ===
        CmdId::GetBlockPackageInfoRequsetCmd => room::on_get_block_package_info,
        CmdId::GetBuildingInfoCmd => room::on_get_building_info,
        CmdId::GetCharacterInteractionInfoCmd => room::on_get_character_interaction_info,
        CmdId::GetRoomObInfoCmd => room::on_get_room_ob_info,
        CmdId::GetRoomPlanInfoCmd => room::on_get_room_plan_info,
        CmdId::GetRoomLogCmd => room::on_get_room_log,
        CmdId::GetRoomInfoCmd => room::on_get_room_info,

        // === Summons ===
        CmdId::GetSummonInfoCmd => gacha::on_get_summon_info,
        CmdId::SummonQueryTokenCmd => gacha::on_summon_query_token,
        CmdId::SummonCmd => gacha::on_summon,
        CmdId::ChooseEnhancedPoolHeroCmd => gacha::on_choose_enhanced_pool_hero,

        // === Mail ===
        CmdId::GetAllMailsCmd => mail::on_get_all_mails,
        CmdId::ReadMailBatchCmd => mail::on_read_mail_batch,
        CmdId::ReadMailCmd => mail::on_read_mail,

        // === Charge & Monetization ===
        CmdId::GetChargeInfoCmd => charge::on_get_charge_info,
        CmdId::GetMonthCardInfoCmd => charge::on_get_month_card_info,
        CmdId::GetChargePushInfoCmd => charge::on_get_charge_push_info,
        CmdId::ReadChargeNewCmd => charge::on_read_charge_new,

        // === Store ===
        CmdId::GetStoreInfosCmd => store::on_get_store_infos, // keep this static for now it controlls the items in shop
        CmdId::BuyGoodsCmd => store::on_buy_goods,
        CmdId::NewOrderCmd => store::on_new_order,

        // === Sign In & Daily Rewards ===
        CmdId::GetSignInInfoCmd => sign_in::on_get_sign_in_info,
        CmdId::SignInCmd => sign_in::on_sign_in,
        CmdId::SignInTotalRewardAllCmd => sign_in::on_sign_in_total_reward_all,
        CmdId::SignInAddupCmd => sign_in::on_sign_in_addup,
        CmdId::SignInHistoryCmd => sign_in::on_sign_in_history,

        // === Achievements & Tasks ===
        CmdId::GetAchievementInfoCmd => achievements::on_get_achievement_info,
        CmdId::GetTaskInfoCmd => task::on_get_task_info,

        // === Battle Pass ===
        CmdId::GetBpInfoCmd => bp::on_get_bp_info,

        // === Guides & Tutorials ===
        CmdId::GetGuideInfoCmd => guide::on_get_guide_info,
        CmdId::GetHandbookInfoCmd => handbook::on_get_handbook_info,
        CmdId::FinishGuideCmd => guide::on_finish_guide,

        // === Social & Friends ===
        CmdId::LoadFriendInfosCmd => chat::on_load_friend_infos,
        CmdId::GetFriendInfoListCmd => chat::on_get_friend_info_list,
        CmdId::GetRecommendedFriendsCmd => chat::on_get_recommended_friends,
        CmdId::GetApplyListCmd => chat::on_get_apply_list,
        CmdId::GetBlacklistCmd => chat::on_get_blacklist,
        CmdId::SendMsgCmd => chat::on_send_msg,
        CmdId::DeleteOfflineMsgCmd => chat::on_delete_offline_msg,

        // === UI & Settings ===
        CmdId::GetRedDotInfosCmd => red_dot::on_get_red_dot_infos,
        CmdId::GetSettingInfosCmd => user_setting::on_get_setting_infos,

        // === Properties ===
        CmdId::GetSimplePropertyCmd => property::on_get_simple_property,
        CmdId::SetSimplePropertyCmd => property::on_set_simple_property,

        // === Miscellaneous Systems ===
        CmdId::DiceHeroGetInfoCmd => dice::on_dice_hero_get_info,
        CmdId::GetAntiqueInfoCmd => antique::on_get_antique_info,
        CmdId::GetUnlockVoucherInfoCmd => voucher::on_get_unlock_voucher_info,
        CmdId::GetWeekwalkInfoCmd => weekwalk::on_get_weekwalk_info,
        CmdId::WeekwalkVer2GetInfoCmd => weekwalk::on_weekwalk_ver2_get_info,
        CmdId::BeforeStartWeekwalkBattleCmd => weekwalk::on_before_start_weekwalk_battle,
        CmdId::GetCommandPostInfoCmd => command_post::on_get_command_post_info,
        CmdId::GetTurnbackInfoCmd => turnback::on_get_turnback_info,
        CmdId::GetPowerMakerInfoCmd => power_maker::on_get_power_maker_info,
        CmdId::CritterGetInfoCmd => critter::on_critter_get_info,

        // === Talent ===
        //Todo add option for talent upgrades
        CmdId::TalentStyleReadCmd => talent::on_talent_style_read, // just echos back the hero id
        CmdId::PutTalentCubeCmd => talent::on_put_talent_cube,
        CmdId::HeroTalentUpCmd => talent::on_hero_talent_up,
        CmdId::PutTalentSchemeCmd => talent::on_put_talent_scheme,
        CmdId::HeroTalentStyleStatCmd => talent::on_hero_talent_style_stat,
        CmdId::UnlockTalentStyleCmd => talent::on_unlock_talent_style,
        CmdId::UseTalentStyleCmd => talent::on_use_talent_style,
        CmdId::UseTalentTemplateCmd => talent::on_use_talent_template,

        // === BGM ===
        CmdId::GetBgmInfoCmd => misc::on_get_bgm_info, // we're loading all the bgm from the excel table for starter data
        CmdId::SetUseBgmCmd => misc::on_set_use_bgm,
        CmdId::SetFavoriteBgmCmd => misc::on_set_favorite_bgm,

        // === Wilderness ===
        CmdId::GetManufactureInfoCmd => manufacture::on_get_manufacture_info,

        // === Activities ===
        CmdId::GetActivityInfosCmd => events::on_get_activity_infos,
        CmdId::GetActivityInfosWithParamCmd => events::on_get_activity_infos_with_param,
        // Controls the ui for the latest euphoria not implemented yet tho
        CmdId::GetAct125InfosCmd => events::on_get_act125_infos,
        // controls ui for bonus currency at the start usually for 7 days
        // state 0 = not started state 1 = not completed, state 2 = completed
        CmdId::Get101InfosCmd => events::on_get101_infos,
        CmdId::Get101BonusCmd => events::on_get101_bonus,
        CmdId::Act160GetInfoCmd => events::on_act160_get_info,
        CmdId::Act165GetInfoCmd => events::on_act165_get_info,
        CmdId::GetAct208InfoCmd => events::on_get_act208_info,
        CmdId::GetAct209InfoCmd => events::on_get_act209_info,
        CmdId::GetAct212InfoCmd => events::on_act212_get_info,
    });

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{RawCommandPolicy, dispatch_command, raw_command_policy};
    use crate::network::packet::{ClientPacket, ServerPacket};
    use crate::state::{AppState, ConnectionContext};
    use sonettobuf::{
        BeforeStartWeekwalkBattleReply, BeforeStartWeekwalkBattleRequest, CmdId,
        DestinyRankUpRequest, GetActivityInfosWithParamReply, GetActivityInfosWithParamRequest,
        GetFightCardDeckInfoReply, GetFightCardDeckInfoRequest, prost::Message,
    };
    use sqlx::sqlite::SqlitePoolOptions;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::Once;
    use tokio::io::AsyncReadExt;
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::Mutex;
    use tokio::time::{Duration, timeout};

    async fn test_connection() -> (Arc<Mutex<ConnectionContext>>, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (client, server) = tokio::join!(TcpStream::connect(address), listener.accept());
        let client = client.unwrap();
        let (server, _) = server.unwrap();

        let db = SqlitePoolOptions::new()
            .connect_lazy("sqlite::memory:")
            .unwrap();
        let state = Arc::new(AppState::new(db));
        let context = ConnectionContext::new(Arc::new(Mutex::new(server)), state);

        (Arc::new(Mutex::new(context)), client)
    }

    async fn test_connection_with_migrated_db() -> (Arc<Mutex<ConnectionContext>>, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (client, server) = tokio::join!(TcpStream::connect(address), listener.accept());
        let client = client.unwrap();
        let (server, _) = server.unwrap();

        let db = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        database::run_migrations(&db).await.unwrap();
        sqlx::query(
            r#"INSERT INTO users (
                id, username, account_type, created_at, updated_at
            ) VALUES (90000001, 'test-user', 10, 0, 0)"#,
        )
        .execute(&db)
        .await
        .unwrap();
        sqlx::query(
            r#"INSERT INTO heroes (
                uid, user_id, hero_id, create_time, level, exp, rank, breakthrough,
                skin, faith, active_skill_level, ex_skill_level, destiny_rank,
                destiny_level, base_hp, base_attack, base_defense, base_mdefense,
                base_technic
            ) VALUES (1, 90000001, 3098, 0, 180, 0, 4, 0, 309801, 0, 1, 1,
                      0, 0, 1, 1, 1, 1, 1)"#,
        )
        .execute(&db)
        .await
        .unwrap();

        let state = Arc::new(AppState::new(db));
        let mut context = ConnectionContext::new(Arc::new(Mutex::new(server)), state);
        context.player_id = Some(90000001);

        (Arc::new(Mutex::new(context)), client)
    }

    fn ensure_test_config() {
        static INIT: Once = Once::new();

        INIT.call_once(|| {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
            common::init_config(common::config::ServerConfig {
                server: common::config::ServerSettings {
                    host: "127.0.0.1".to_string(),
                    dns: "127.0.0.1".to_string(),
                    http_port: 21100,
                    game_port: 23401,
                },
                paths: common::config::PathConfig {
                    data_dir: root.join("assets"),
                    excel_data: root.join("assets"),
                    static_data: root.join("assets/static"),
                },
                database: common::config::DatabaseConfig {
                    path: root.join("target/test-sonetto-3.8-cn.db"),
                },
                banners: vec![],
            });
        });
    }

    async fn read_server_packet(client: &mut TcpStream) -> ServerPacket {
        let mut length = [0_u8; 4];
        timeout(Duration::from_secs(1), client.read_exact(&mut length))
            .await
            .expect("timed out waiting for reply length")
            .unwrap();

        let payload_length = u32::from_be_bytes(length) as usize;
        let mut encoded = length.to_vec();
        encoded.resize(4 + payload_length, 0);
        timeout(Duration::from_secs(1), client.read_exact(&mut encoded[4..]))
            .await
            .expect("timed out waiting for reply body")
            .unwrap();

        ServerPacket::decode(&encoded).unwrap()
    }

    #[test]
    fn compatibility_policies_have_exact_static_success_bodies() {
        for (raw_id, expected_body) in [
            (20144, &[][..]),
            (13540, &[0x0A, 0x00][..]),
            (-16527, &[][..]),
        ] {
            let RawCommandPolicy::StaticSuccessReply { command, body } =
                raw_command_policy(raw_id).expect("missing compatibility policy");

            assert_eq!(command.raw_id(), raw_id);
            assert_eq!(body, expected_body);
        }
    }

    #[tokio::test]
    async fn raw_20144_returns_empty_success_reply() {
        let (ctx, mut client) = test_connection().await;
        let consumed_down_tag = ctx.lock().await.state.reserve_down_tag().await;
        let request = ClientPacket {
            sequence: 1,
            cmd_id: 20144,
            up_tag: 37,
            data: Vec::new(),
        }
        .encode();

        dispatch_command(Arc::clone(&ctx), &request).await.unwrap();
        ctx.lock().await.flush_send_queue().await.unwrap();

        let mut encoded_reply = vec![0; ServerPacket::PACKET_HEADER];
        timeout(
            Duration::from_secs(1),
            client.read_exact(&mut encoded_reply),
        )
        .await
        .expect("timed out waiting for compatibility reply")
        .unwrap();
        let reply = ServerPacket::decode(&encoded_reply).unwrap();

        assert_eq!(reply.cmd_id, 20144);
        assert_eq!(reply.result_code, 0);
        assert_eq!(reply.up_tag, 37);
        assert_eq!(reply.down_tag, consumed_down_tag + 1);
        assert!(reply.data.is_empty());
    }

    #[tokio::test]
    async fn raw_13540_returns_present_empty_tower_compose_info() {
        let (ctx, mut client) = test_connection().await;
        let consumed_down_tag = ctx.lock().await.state.reserve_down_tag().await;
        let request = ClientPacket {
            sequence: 1,
            cmd_id: 13540,
            up_tag: 38,
            data: vec![0x08, 0x01],
        }
        .encode();

        dispatch_command(Arc::clone(&ctx), &request).await.unwrap();
        ctx.lock().await.flush_send_queue().await.unwrap();

        let mut encoded_reply = vec![0; ServerPacket::PACKET_HEADER + 2];
        timeout(
            Duration::from_secs(1),
            client.read_exact(&mut encoded_reply),
        )
        .await
        .expect("timed out waiting for compatibility reply")
        .unwrap();
        let reply = ServerPacket::decode(&encoded_reply).unwrap();

        assert_eq!(reply.cmd_id, 13540);
        assert_eq!(reply.result_code, 0);
        assert_eq!(reply.up_tag, 38);
        assert_eq!(reply.down_tag, consumed_down_tag + 1);
        assert_eq!(reply.data, [0x0A, 0x00]);
    }

    #[tokio::test]
    async fn raw_negative_16527_returns_empty_party_server_list() {
        let (ctx, mut client) = test_connection().await;
        let consumed_down_tag = ctx.lock().await.state.reserve_down_tag().await;
        let request = ClientPacket {
            sequence: 1,
            cmd_id: -16527,
            up_tag: 39,
            data: Vec::new(),
        }
        .encode();

        dispatch_command(Arc::clone(&ctx), &request).await.unwrap();
        ctx.lock().await.flush_send_queue().await.unwrap();

        let mut encoded_reply = vec![0; ServerPacket::PACKET_HEADER];
        timeout(
            Duration::from_secs(1),
            client.read_exact(&mut encoded_reply),
        )
        .await
        .expect("timed out waiting for PartyMatch.PartyServerListReply")
        .unwrap();
        let reply = ServerPacket::decode(&encoded_reply).unwrap();

        assert_eq!(reply.cmd_id, -16527);
        assert_eq!(reply.result_code, 0);
        assert_eq!(reply.up_tag, 39);
        assert_eq!(reply.down_tag, consumed_down_tag + 1);
        assert!(reply.data.is_empty());
    }

    #[tokio::test]
    async fn before_start_weekwalk_battle_echoes_requested_element_and_layer() {
        let (ctx, mut client) = test_connection().await;
        let consumed_down_tag = ctx.lock().await.state.reserve_down_tag().await;
        let request_body = BeforeStartWeekwalkBattleRequest {
            element_id: Some(4321),
            layer_id: Some(7),
        }
        .encode_to_vec();
        let request = ClientPacket {
            sequence: 1,
            cmd_id: CmdId::BeforeStartWeekwalkBattleCmd as i16,
            up_tag: 40,
            data: request_body,
        }
        .encode();

        dispatch_command(Arc::clone(&ctx), &request).await.unwrap();
        ctx.lock().await.flush_send_queue().await.unwrap();

        let mut encoded_reply = vec![0; ServerPacket::PACKET_HEADER + 5];
        timeout(
            Duration::from_secs(1),
            client.read_exact(&mut encoded_reply),
        )
        .await
        .expect("timed out waiting for BeforeStartWeekwalkBattleReply")
        .unwrap();
        let reply = ServerPacket::decode(&encoded_reply).unwrap();
        let body = reply
            .decode_message::<BeforeStartWeekwalkBattleReply>()
            .unwrap();

        assert_eq!(reply.cmd_id, CmdId::BeforeStartWeekwalkBattleCmd as i16);
        assert_eq!(reply.result_code, 0);
        assert_eq!(reply.up_tag, 40);
        assert_eq!(reply.down_tag, consumed_down_tag + 1);
        assert_eq!(body.element_id, Some(4321));
        assert_eq!(body.layer_id, Some(7));
    }

    #[tokio::test]
    async fn destiny_rank_up_unlocks_first_rank_for_zero_rank_hero() {
        let (ctx, _client) = test_connection_with_migrated_db().await;
        let request = ClientPacket {
            sequence: 1,
            cmd_id: CmdId::DestinyRankUpCmd as i16,
            up_tag: 41,
            data: DestinyRankUpRequest {
                hero_id: Some(3098),
            }
            .encode_to_vec(),
        }
        .encode();

        dispatch_command(Arc::clone(&ctx), &request).await.unwrap();

        let db = ctx.lock().await.state.db.clone();
        let updated: (i32, i32) = sqlx::query_as(
            "SELECT destiny_rank, destiny_level FROM heroes WHERE user_id = ? AND hero_id = ?",
        )
        .bind(90000001_i64)
        .bind(3098_i32)
        .fetch_one(&db)
        .await
        .unwrap();

        assert_eq!(updated, (1, 1));
    }

    #[tokio::test]
    async fn activity_infos_with_param_returns_only_requested_activities() {
        ensure_test_config();
        let (ctx, mut client) = test_connection().await;
        let request = ClientPacket {
            sequence: 1,
            cmd_id: CmdId::GetActivityInfosWithParamCmd as i16,
            up_tag: 42,
            data: GetActivityInfosWithParamRequest {
                activity_ids: vec![13316, 12301],
            }
            .encode_to_vec(),
        }
        .encode();

        dispatch_command(Arc::clone(&ctx), &request).await.unwrap();
        ctx.lock().await.flush_send_queue().await.unwrap();

        let reply = read_server_packet(&mut client).await;
        let body = reply
            .decode_message::<GetActivityInfosWithParamReply>()
            .unwrap();
        let ids = body
            .activity_infos
            .into_iter()
            .filter_map(|activity| activity.id)
            .collect::<Vec<_>>();

        assert_eq!(reply.cmd_id, CmdId::GetActivityInfosWithParamCmd as i16);
        assert_eq!(reply.result_code, 0);
        assert_eq!(reply.up_tag, 42);
        assert_eq!(ids, vec![13316, 12301]);
    }

    #[tokio::test]
    async fn fight_card_deck_info_returns_empty_success_reply() {
        let (ctx, mut client) = test_connection().await;
        let request = ClientPacket {
            sequence: 1,
            cmd_id: CmdId::GetFightCardDeckInfoCmd as i16,
            up_tag: 43,
            data: GetFightCardDeckInfoRequest { r#type: Some(0) }.encode_to_vec(),
        }
        .encode();

        dispatch_command(Arc::clone(&ctx), &request).await.unwrap();
        ctx.lock().await.flush_send_queue().await.unwrap();

        let reply = read_server_packet(&mut client).await;
        let body = reply.decode_message::<GetFightCardDeckInfoReply>().unwrap();

        assert_eq!(reply.cmd_id, CmdId::GetFightCardDeckInfoCmd as i16);
        assert_eq!(reply.result_code, 0);
        assert_eq!(reply.up_tag, 43);
        assert!(body.deck_infos.is_empty());
    }

    #[tokio::test]
    async fn unregistered_raw_command_is_ignored() {
        let (ctx, _client) = test_connection().await;
        let request = ClientPacket {
            sequence: 1,
            cmd_id: 20145,
            up_tag: 37,
            data: Vec::new(),
        }
        .encode();

        let result = dispatch_command(Arc::clone(&ctx), &request).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn registered_unhandled_command_returns_error_reply() {
        let (ctx, mut client) = test_connection().await;
        let request = ClientPacket {
            sequence: 1,
            cmd_id: CmdId::ReadNewAchievementCmd as i16,
            up_tag: 38,
            data: Vec::new(),
        }
        .encode();

        let result = dispatch_command(Arc::clone(&ctx), &request).await;

        assert!(result.is_ok());
        ctx.lock().await.flush_send_queue().await.unwrap();

        let reply = read_server_packet(&mut client).await;

        assert_eq!(reply.cmd_id, CmdId::ReadNewAchievementCmd as i16);
        assert_eq!(reply.result_code, 1);
        assert_eq!(reply.up_tag, 38);
        assert!(reply.data.is_empty());
    }
}
