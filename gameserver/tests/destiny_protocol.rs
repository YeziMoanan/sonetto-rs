use database::models::game::destiny::{
    DestinyCommand, DestinyState, MutationKind, OwnedDestinyHero, plan_transition,
};
use gameserver::network::{
    handler::dispatch_command,
    packet::{ClientPacket, ServerPacket},
};
use gameserver::state::{AppState, ConnectionContext};
use prost::Message;
use sonettobuf::{
    CmdId, DestinyLevelUpReply, DestinyLevelUpRequest, DestinyRankUpReply, DestinyRankUpRequest,
    DestinyStoneUnlockReply, DestinyStoneUnlockRequest, DestinyStoneUseReply,
    DestinyStoneUseRequest, GetServerTimeReply, GetServerTimeRequest, HeroInfoListReply,
    HeroInfoListRequest, HeroUpdatePush,
};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{
    Arc, Once,
    atomic::{AtomicU64, Ordering},
};
use tokio::io::AsyncReadExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio::time::{Duration, timeout};

const USER_ID: i64 = 16_356_367;
const HERO_UID: i64 = 1;
const HERO_ID: i32 = 3098;
const FIRST_STONE_ID: i32 = 309801;

static CONFIG_INIT: Once = Once::new();
static TEMP_DB_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TempDatabase {
    path: PathBuf,
}

impl TempDatabase {
    fn new() -> Self {
        let suffix = TEMP_DB_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        Self {
            path: std::env::temp_dir().join(format!(
                "sonetto-destiny-protocol-{}-{suffix}.db",
                std::process::id()
            )),
        }
    }
}

impl TempDatabase {
    fn cleanup(&self) {
        for path in [
            self.path.clone(),
            self.path.with_extension("db-shm"),
            self.path.with_extension("db-wal"),
        ] {
            for _ in 0..100 {
                match std::fs::remove_file(&path) {
                    Ok(()) => break,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                    Err(_) => std::thread::sleep(std::time::Duration::from_millis(20)),
                }
            }
        }
    }
}

impl Drop for TempDatabase {
    fn drop(&mut self) {
        self.cleanup();
    }
}

async fn close_and_cleanup(
    temp_db: TempDatabase,
    ctx: Arc<Mutex<ConnectionContext>>,
    client: TcpStream,
) {
    let pool = ctx.lock().await.state.db.clone();
    pool.close().await;
    drop(ctx);
    drop(client);
    temp_db.cleanup();
}

fn init_config() {
    CONFIG_INIT.call_once(|| {
        let data_dir = std::env::var("JSON_DATA_DIR").expect(
            "JSON_DATA_DIR must point at the international 3.6 runtime excel2json directory",
        );
        config::configs::init(&data_dir).expect("failed to initialize config data");
    });
}

async fn test_connection_with_destiny_hero(
    destiny_rank: i32,
    destiny_level: i32,
    destiny_stone: i32,
) -> (TempDatabase, Arc<Mutex<ConnectionContext>>, TcpStream) {
    init_config();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (client, server) = tokio::join!(TcpStream::connect(address), listener.accept());
    let client = client.unwrap();
    let (server, _) = server.unwrap();

    let temp_db = TempDatabase::new();
    let options = SqliteConnectOptions::from_str(temp_db.path.to_str().unwrap())
        .unwrap()
        .create_if_missing(true)
        .foreign_keys(true)
        .busy_timeout(std::time::Duration::from_millis(25));
    let db = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    database::run_migrations(&db).await.unwrap();
    sqlx::query("INSERT INTO users (id, username, created_at, updated_at) VALUES (?, ?, 0, 0)")
        .bind(USER_ID)
        .bind("destiny-protocol-test")
        .execute(&db)
        .await
        .unwrap();
    sqlx::query(
        r#"INSERT INTO heroes (
            uid, user_id, hero_id, create_time, level, exp, rank, breakthrough,
            skin, faith, active_skill_level, ex_skill_level, destiny_rank,
            destiny_level, destiny_stone, base_hp, base_attack, base_defense, base_mdefense,
            base_technic
        ) VALUES (?, ?, ?, 0, 180, 0, 4, 0, 309801, 0, 1, 1,
                  ?, ?, ?, 1, 1, 1, 1, 1)"#,
    )
    .bind(HERO_UID)
    .bind(USER_ID)
    .bind(HERO_ID)
    .bind(destiny_rank)
    .bind(destiny_level)
    .bind(destiny_stone)
    .execute(&db)
    .await
    .unwrap();

    for item_id in [620101, 620102, 620103, 111003, 111004, 111008] {
        sqlx::query(
            "INSERT INTO items (user_id, item_id, quantity, last_use_time, last_update_time, total_gain_count) VALUES (?, ?, 10000, 0, 0, 10000)",
        )
        .bind(USER_ID)
        .bind(item_id)
        .execute(&db)
        .await
        .unwrap();
    }

    let state = Arc::new(AppState::new(db));
    let mut context = ConnectionContext::new(Arc::new(Mutex::new(server)), state);
    context.player_id = Some(USER_ID);

    (temp_db, Arc::new(Mutex::new(context)), client)
}

async fn test_connection_with_pool(pool: SqlitePool) -> (Arc<Mutex<ConnectionContext>>, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (client, server) = tokio::join!(TcpStream::connect(address), listener.accept());
    let client = client.unwrap();
    let (server, _) = server.unwrap();
    let state = Arc::new(AppState::new(pool));
    let mut context = ConnectionContext::new(Arc::new(Mutex::new(server)), state);
    context.player_id = Some(USER_ID);
    (Arc::new(Mutex::new(context)), client)
}
async fn read_server_packet(client: &mut TcpStream) -> ServerPacket {
    let mut length = [0_u8; 4];
    timeout(Duration::from_secs(1), client.read_exact(&mut length))
        .await
        .expect("timed out waiting for packet length")
        .unwrap();

    let body_length = u32::from_be_bytes(length) as usize;
    let mut encoded = Vec::with_capacity(body_length + 4);
    encoded.extend_from_slice(&length);
    encoded.resize(body_length + 4, 0);
    timeout(Duration::from_secs(1), client.read_exact(&mut encoded[4..]))
        .await
        .expect("timed out waiting for packet body")
        .unwrap();

    ServerPacket::decode(&encoded).unwrap()
}

async fn dispatch_and_flush(ctx: Arc<Mutex<ConnectionContext>>, request: Vec<u8>) {
    dispatch_command(Arc::clone(&ctx), &request).await.unwrap();
    ctx.lock().await.flush_send_queue().await.unwrap();
}

fn client_packet<T: Message>(cmd: CmdId, up_tag: u8, message: T) -> Vec<u8> {
    ClientPacket {
        sequence: 1,
        cmd_id: cmd as i16,
        up_tag,
        data: message.encode_to_vec(),
    }
    .encode()
}

#[tokio::test]
async fn destiny_rank_up_packet_pushes_committed_state_and_resources() {
    let (temp_db, ctx, mut client) = test_connection_with_destiny_hero(0, 0, 0).await;

    dispatch_and_flush(
        Arc::clone(&ctx),
        client_packet(
            CmdId::DestinyRankUpCmd,
            11,
            DestinyRankUpRequest {
                hero_id: Some(HERO_ID),
            },
        ),
    )
    .await;

    let item_push = read_server_packet(&mut client).await;
    assert_eq!(item_push.cmd_id, CmdId::ItemChangePushCmd as i16);
    let hero_push = read_server_packet(&mut client).await;
    assert_eq!(hero_push.cmd_id, CmdId::HeroHeroUpdatePushCmd as i16);
    let push = hero_push.decode_message::<HeroUpdatePush>().unwrap();
    assert_eq!(push.hero_updates[0].destiny_rank, Some(1));
    assert_eq!(push.hero_updates[0].destiny_level, Some(1));
    assert_eq!(push.hero_updates[0].destiny_stone, Some(0));
    let reply_packet = read_server_packet(&mut client).await;
    let reply = reply_packet.decode_message::<DestinyRankUpReply>().unwrap();
    assert_eq!(reply_packet.cmd_id, CmdId::DestinyRankUpCmd as i16);
    assert_eq!(reply_packet.result_code, 0);
    assert_eq!(reply_packet.up_tag, 11);
    assert_eq!(reply.hero_id, Some(HERO_ID));
    close_and_cleanup(temp_db, ctx, client).await;
}

#[tokio::test]
async fn destiny_level_up_packet_dispatches_batch_and_replies_committed_level() {
    let (temp_db, ctx, mut client) = test_connection_with_destiny_hero(1, 1, 0).await;

    dispatch_and_flush(
        Arc::clone(&ctx),
        client_packet(
            CmdId::DestinyLevelUpCmd,
            12,
            DestinyLevelUpRequest {
                hero_id: Some(HERO_ID),
                level: Some(2),
            },
        ),
    )
    .await;

    let item_push = read_server_packet(&mut client).await;
    assert_eq!(item_push.cmd_id, CmdId::ItemChangePushCmd as i16);
    let hero_push = read_server_packet(&mut client).await;
    let push = hero_push.decode_message::<HeroUpdatePush>().unwrap();
    assert_eq!(push.hero_updates[0].destiny_rank, Some(1));
    assert_eq!(push.hero_updates[0].destiny_level, Some(2));
    let reply_packet = read_server_packet(&mut client).await;
    let reply = reply_packet
        .decode_message::<DestinyLevelUpReply>()
        .unwrap();
    assert_eq!(reply_packet.cmd_id, CmdId::DestinyLevelUpCmd as i16);
    assert_eq!(reply_packet.result_code, 0);
    assert_eq!(reply.level, Some(2));
    close_and_cleanup(temp_db, ctx, client).await;
}

#[tokio::test]
async fn destiny_stone_unlock_packet_pushes_committed_unlock_list() {
    let (temp_db, ctx, mut client) = test_connection_with_destiny_hero(1, 1, 0).await;

    dispatch_and_flush(
        Arc::clone(&ctx),
        client_packet(
            CmdId::DestinyStoneUnlockCmd,
            13,
            DestinyStoneUnlockRequest {
                hero_id: Some(HERO_ID),
                stone_id: Some(FIRST_STONE_ID),
            },
        ),
    )
    .await;

    let item_push = read_server_packet(&mut client).await;
    assert_eq!(item_push.cmd_id, CmdId::ItemChangePushCmd as i16);
    let hero_push = read_server_packet(&mut client).await;
    let push = hero_push.decode_message::<HeroUpdatePush>().unwrap();
    assert_eq!(
        push.hero_updates[0].destiny_stone_unlock,
        vec![FIRST_STONE_ID]
    );
    let reply_packet = read_server_packet(&mut client).await;
    let reply = reply_packet
        .decode_message::<DestinyStoneUnlockReply>()
        .unwrap();
    assert_eq!(reply_packet.cmd_id, CmdId::DestinyStoneUnlockCmd as i16);
    assert_eq!(reply_packet.result_code, 0);
    assert_eq!(reply.stone_id, Some(FIRST_STONE_ID));
    close_and_cleanup(temp_db, ctx, client).await;
}

#[tokio::test]
async fn destiny_stone_use_packet_pushes_new_stone_not_preupdate_value() {
    let (temp_db, ctx, mut client) = test_connection_with_destiny_hero(1, 1, 0).await;
    let db = ctx.lock().await.state.db.clone();
    sqlx::query("INSERT INTO hero_destiny_stone_unlocks (hero_uid, stone_id) VALUES (?, ?)")
        .bind(HERO_UID)
        .bind(FIRST_STONE_ID)
        .execute(&db)
        .await
        .unwrap();

    dispatch_and_flush(
        Arc::clone(&ctx),
        client_packet(
            CmdId::DestinyStoneUseCmd,
            14,
            DestinyStoneUseRequest {
                hero_id: Some(HERO_ID),
                stone_id: Some(FIRST_STONE_ID),
            },
        ),
    )
    .await;

    let hero_push = read_server_packet(&mut client).await;
    let push = hero_push.decode_message::<HeroUpdatePush>().unwrap();
    assert_eq!(push.hero_updates[0].destiny_stone, Some(FIRST_STONE_ID));
    let reply_packet = read_server_packet(&mut client).await;
    let reply = reply_packet
        .decode_message::<DestinyStoneUseReply>()
        .unwrap();
    assert_eq!(reply_packet.cmd_id, CmdId::DestinyStoneUseCmd as i16);
    assert_eq!(reply_packet.result_code, 0);
    assert_eq!(reply.stone_id, Some(FIRST_STONE_ID));
    drop(db);
    close_and_cleanup(temp_db, ctx, client).await;
}

#[tokio::test]
async fn all_four_destiny_commands_round_trip_real_packet_encoding() {
    let (temp_db, ctx, mut client) = test_connection_with_destiny_hero(0, 0, 0).await;

    dispatch_and_flush(
        Arc::clone(&ctx),
        client_packet(
            CmdId::DestinyRankUpCmd,
            21,
            DestinyRankUpRequest {
                hero_id: Some(HERO_ID),
            },
        ),
    )
    .await;
    assert_eq!(
        read_server_packet(&mut client).await.cmd_id,
        CmdId::ItemChangePushCmd as i16
    );
    assert_eq!(
        read_server_packet(&mut client).await.cmd_id,
        CmdId::HeroHeroUpdatePushCmd as i16
    );
    assert_eq!(
        read_server_packet(&mut client).await.cmd_id,
        CmdId::DestinyRankUpCmd as i16
    );

    dispatch_and_flush(
        Arc::clone(&ctx),
        client_packet(
            CmdId::DestinyLevelUpCmd,
            22,
            DestinyLevelUpRequest {
                hero_id: Some(HERO_ID),
                level: Some(2),
            },
        ),
    )
    .await;
    assert_eq!(
        read_server_packet(&mut client).await.cmd_id,
        CmdId::ItemChangePushCmd as i16
    );
    assert_eq!(
        read_server_packet(&mut client).await.cmd_id,
        CmdId::HeroHeroUpdatePushCmd as i16
    );
    assert_eq!(
        read_server_packet(&mut client).await.cmd_id,
        CmdId::DestinyLevelUpCmd as i16
    );

    dispatch_and_flush(
        Arc::clone(&ctx),
        client_packet(
            CmdId::DestinyStoneUnlockCmd,
            23,
            DestinyStoneUnlockRequest {
                hero_id: Some(HERO_ID),
                stone_id: Some(FIRST_STONE_ID),
            },
        ),
    )
    .await;
    assert_eq!(
        read_server_packet(&mut client).await.cmd_id,
        CmdId::ItemChangePushCmd as i16
    );
    assert_eq!(
        read_server_packet(&mut client).await.cmd_id,
        CmdId::HeroHeroUpdatePushCmd as i16
    );
    assert_eq!(
        read_server_packet(&mut client).await.cmd_id,
        CmdId::DestinyStoneUnlockCmd as i16
    );

    dispatch_and_flush(
        Arc::clone(&ctx),
        client_packet(
            CmdId::DestinyStoneUseCmd,
            24,
            DestinyStoneUseRequest {
                hero_id: Some(HERO_ID),
                stone_id: Some(FIRST_STONE_ID),
            },
        ),
    )
    .await;
    assert_eq!(
        read_server_packet(&mut client).await.cmd_id,
        CmdId::HeroHeroUpdatePushCmd as i16
    );
    assert_eq!(
        read_server_packet(&mut client).await.cmd_id,
        CmdId::DestinyStoneUseCmd as i16
    );
    close_and_cleanup(temp_db, ctx, client).await;
}

#[tokio::test]
async fn destiny_validation_failure_reply_keeps_connection_open() {
    let (temp_db, ctx, mut client) = test_connection_with_destiny_hero(1, 1, 0).await;

    dispatch_and_flush(
        Arc::clone(&ctx),
        client_packet(
            CmdId::DestinyLevelUpCmd,
            31,
            DestinyLevelUpRequest {
                hero_id: Some(HERO_ID),
                level: Some(99),
            },
        ),
    )
    .await;
    let failure = read_server_packet(&mut client).await;
    assert_eq!(failure.cmd_id, CmdId::DestinyLevelUpCmd as i16);
    assert_eq!(failure.result_code, 1);

    dispatch_and_flush(
        Arc::clone(&ctx),
        client_packet(CmdId::GetServerTimeCmd, 32, GetServerTimeRequest {}),
    )
    .await;
    let time_packet = read_server_packet(&mut client).await;
    let time_reply = time_packet.decode_message::<GetServerTimeReply>().unwrap();
    assert_eq!(time_packet.cmd_id, CmdId::GetServerTimeCmd as i16);
    assert_eq!(time_packet.result_code, 0);
    assert!(time_reply.server_time.is_some());

    close_and_cleanup(temp_db, ctx, client).await;
}

#[tokio::test]
async fn destiny_missing_required_field_reply_keeps_connection_open() {
    let (temp_db, ctx, mut client) = test_connection_with_destiny_hero(1, 1, 0).await;

    let invalid_requests = [
        (
            CmdId::DestinyRankUpCmd,
            36,
            client_packet(
                CmdId::DestinyRankUpCmd,
                36,
                DestinyRankUpRequest { hero_id: None },
            ),
        ),
        (
            CmdId::DestinyLevelUpCmd,
            37,
            client_packet(
                CmdId::DestinyLevelUpCmd,
                37,
                DestinyLevelUpRequest {
                    hero_id: None,
                    level: Some(2),
                },
            ),
        ),
        (
            CmdId::DestinyStoneUnlockCmd,
            38,
            client_packet(
                CmdId::DestinyStoneUnlockCmd,
                38,
                DestinyStoneUnlockRequest {
                    hero_id: Some(HERO_ID),
                    stone_id: None,
                },
            ),
        ),
        (
            CmdId::DestinyStoneUseCmd,
            39,
            client_packet(
                CmdId::DestinyStoneUseCmd,
                39,
                DestinyStoneUseRequest {
                    hero_id: None,
                    stone_id: Some(0),
                },
            ),
        ),
    ];

    for (cmd, up_tag, request) in invalid_requests {
        dispatch_command(Arc::clone(&ctx), &request).await.unwrap();
        ctx.lock().await.flush_send_queue().await.unwrap();

        let failure = read_server_packet(&mut client).await;
        assert_eq!(failure.cmd_id, cmd as i16);
        assert_eq!(failure.result_code, 1);
        assert_eq!(failure.up_tag, up_tag);
        match cmd {
            CmdId::DestinyRankUpCmd => {
                assert_eq!(
                    failure
                        .decode_message::<DestinyRankUpReply>()
                        .unwrap()
                        .hero_id,
                    None
                );
            }
            CmdId::DestinyLevelUpCmd => {
                let reply = failure.decode_message::<DestinyLevelUpReply>().unwrap();
                assert_eq!(reply.hero_id, None);
                assert_eq!(reply.level, Some(2));
            }
            CmdId::DestinyStoneUnlockCmd => {
                let reply = failure
                    .decode_message::<DestinyStoneUnlockReply>()
                    .unwrap();
                assert_eq!(reply.hero_id, Some(HERO_ID));
                assert_eq!(reply.stone_id, None);
            }
            CmdId::DestinyStoneUseCmd => {
                let reply = failure.decode_message::<DestinyStoneUseReply>().unwrap();
                assert_eq!(reply.hero_id, None);
                assert_eq!(reply.stone_id, Some(0));
            }
            _ => unreachable!(),
        }

        dispatch_and_flush(
            Arc::clone(&ctx),
            client_packet(CmdId::GetServerTimeCmd, up_tag + 1, GetServerTimeRequest {}),
        )
        .await;
        let time_packet = read_server_packet(&mut client).await;
        let time_reply = time_packet.decode_message::<GetServerTimeReply>().unwrap();
        assert_eq!(time_packet.cmd_id, CmdId::GetServerTimeCmd as i16);
        assert_eq!(time_packet.result_code, 0);
        assert_eq!(time_packet.up_tag, up_tag + 1);
        assert!(time_reply.server_time.is_some());
    }

    close_and_cleanup(temp_db, ctx, client).await;
}

#[tokio::test]
async fn destiny_malformed_payloads_reply_invalid_and_keep_connection_open() {
    let (temp_db, ctx, mut client) = test_connection_with_destiny_hero(1, 1, 0).await;
    let commands = [
        CmdId::DestinyRankUpCmd,
        CmdId::DestinyLevelUpCmd,
        CmdId::DestinyStoneUnlockCmd,
        CmdId::DestinyStoneUseCmd,
    ];

    for (index, cmd) in commands.into_iter().enumerate() {
        let up_tag = 60 + index as u8;
        let request = ClientPacket {
            sequence: 1,
            cmd_id: cmd as i16,
            up_tag,
            data: vec![0x08, 0x80],
        }
        .encode();
        dispatch_command(Arc::clone(&ctx), &request).await.unwrap();
        ctx.lock().await.flush_send_queue().await.unwrap();

        let failure = read_server_packet(&mut client).await;
        assert_eq!(failure.cmd_id, cmd as i16);
        assert_eq!(failure.result_code, 1);
        assert_eq!(failure.up_tag, up_tag);
        match cmd {
            CmdId::DestinyRankUpCmd => {
                let reply = failure.decode_message::<DestinyRankUpReply>().unwrap();
                assert_eq!(reply.hero_id, None);
            }
            CmdId::DestinyLevelUpCmd => {
                let reply = failure.decode_message::<DestinyLevelUpReply>().unwrap();
                assert_eq!(reply.hero_id, None);
                assert_eq!(reply.level, None);
            }
            CmdId::DestinyStoneUnlockCmd => {
                let reply = failure
                    .decode_message::<DestinyStoneUnlockReply>()
                    .unwrap();
                assert_eq!(reply.hero_id, None);
                assert_eq!(reply.stone_id, None);
            }
            CmdId::DestinyStoneUseCmd => {
                let reply = failure.decode_message::<DestinyStoneUseReply>().unwrap();
                assert_eq!(reply.hero_id, None);
                assert_eq!(reply.stone_id, None);
            }
            _ => unreachable!(),
        }

        dispatch_and_flush(
            Arc::clone(&ctx),
            client_packet(CmdId::GetServerTimeCmd, up_tag + 20, GetServerTimeRequest {}),
        )
        .await;
        let time_packet = read_server_packet(&mut client).await;
        assert_eq!(time_packet.cmd_id, CmdId::GetServerTimeCmd as i16);
        assert_eq!(time_packet.result_code, 0);
        assert_eq!(time_packet.up_tag, up_tag + 20);
    }

    close_and_cleanup(temp_db, ctx, client).await;
}

#[tokio::test]
async fn destiny_catalog_failures_reply_internal_and_keep_connection_open() {
    let (temp_db, ctx, mut client) = test_connection_with_destiny_hero(1, 1, 0).await;
    let requests = [
        ClientPacket {
            sequence: 1,
            cmd_id: CmdId::DestinyRankUpCmd as i16,
            up_tag: 70,
            data: DestinyRankUpRequest {
                hero_id: Some(HERO_ID),
            }
            .encode_to_vec(),
        },
        ClientPacket {
            sequence: 1,
            cmd_id: CmdId::DestinyLevelUpCmd as i16,
            up_tag: 71,
            data: DestinyLevelUpRequest {
                hero_id: Some(HERO_ID),
                level: Some(2),
            }
            .encode_to_vec(),
        },
        ClientPacket {
            sequence: 1,
            cmd_id: CmdId::DestinyStoneUnlockCmd as i16,
            up_tag: 72,
            data: DestinyStoneUnlockRequest {
                hero_id: Some(HERO_ID),
                stone_id: Some(FIRST_STONE_ID),
            }
            .encode_to_vec(),
        },
        ClientPacket {
            sequence: 1,
            cmd_id: CmdId::DestinyStoneUseCmd as i16,
            up_tag: 73,
            data: DestinyStoneUseRequest {
                hero_id: Some(HERO_ID),
                stone_id: Some(0),
            }
            .encode_to_vec(),
        },
    ];

    for request in requests {
        let cmd_id = request.cmd_id;
        let up_tag = request.up_tag;
        ConnectionContext::dispatch_destiny_with_catalog(
            Arc::clone(&ctx),
            request,
            || anyhow::bail!("injected Destiny catalog failure"),
        )
        .await
        .unwrap();
        ctx.lock().await.flush_send_queue().await.unwrap();

        let failure = read_server_packet(&mut client).await;
        assert_eq!(failure.cmd_id, cmd_id);
        assert_eq!(failure.result_code, 4);
        assert_eq!(failure.up_tag, up_tag);
        match cmd_id {
            id if id == CmdId::DestinyRankUpCmd as i16 => {
                let reply = failure.decode_message::<DestinyRankUpReply>().unwrap();
                assert_eq!(reply.hero_id, Some(HERO_ID));
            }
            id if id == CmdId::DestinyLevelUpCmd as i16 => {
                let reply = failure.decode_message::<DestinyLevelUpReply>().unwrap();
                assert_eq!(reply.hero_id, Some(HERO_ID));
                assert_eq!(reply.level, Some(2));
            }
            id if id == CmdId::DestinyStoneUnlockCmd as i16 => {
                let reply = failure
                    .decode_message::<DestinyStoneUnlockReply>()
                    .unwrap();
                assert_eq!(reply.hero_id, Some(HERO_ID));
                assert_eq!(reply.stone_id, Some(FIRST_STONE_ID));
            }
            id if id == CmdId::DestinyStoneUseCmd as i16 => {
                let reply = failure.decode_message::<DestinyStoneUseReply>().unwrap();
                assert_eq!(reply.hero_id, Some(HERO_ID));
                assert_eq!(reply.stone_id, Some(0));
            }
            _ => unreachable!(),
        }

        dispatch_and_flush(
            Arc::clone(&ctx),
            client_packet(CmdId::GetServerTimeCmd, up_tag + 20, GetServerTimeRequest {}),
        )
        .await;
        let time_packet = read_server_packet(&mut client).await;
        assert_eq!(time_packet.cmd_id, CmdId::GetServerTimeCmd as i16);
        assert_eq!(time_packet.result_code, 0);
        assert_eq!(time_packet.up_tag, up_tag + 20);
    }

    close_and_cleanup(temp_db, ctx, client).await;
}

#[tokio::test]
async fn destiny_insufficient_material_reply_keeps_connection_open() {
    let (temp_db, ctx, mut client) = test_connection_with_destiny_hero(0, 0, 0).await;
    let db = ctx.lock().await.state.db.clone();
    sqlx::query("UPDATE items SET quantity = 0 WHERE user_id = ? AND item_id = 620101")
        .bind(USER_ID)
        .execute(&db)
        .await
        .unwrap();

    dispatch_and_flush(
        Arc::clone(&ctx),
        client_packet(
            CmdId::DestinyRankUpCmd,
            33,
            DestinyRankUpRequest {
                hero_id: Some(HERO_ID),
            },
        ),
    )
    .await;
    let failure = read_server_packet(&mut client).await;
    assert_eq!(failure.cmd_id, CmdId::DestinyRankUpCmd as i16);
    assert_eq!(failure.result_code, 2);

    dispatch_and_flush(
        Arc::clone(&ctx),
        client_packet(CmdId::GetServerTimeCmd, 34, GetServerTimeRequest {}),
    )
    .await;
    let time_packet = read_server_packet(&mut client).await;
    assert_eq!(time_packet.cmd_id, CmdId::GetServerTimeCmd as i16);
    assert_eq!(time_packet.result_code, 0);

    drop(db);
    close_and_cleanup(temp_db, ctx, client).await;
}

#[tokio::test]
async fn idempotent_destiny_requests_emit_no_resource_push() {
    let (temp_db, ctx, mut client) = test_connection_with_destiny_hero(1, 1, 0).await;

    dispatch_and_flush(
        Arc::clone(&ctx),
        client_packet(
            CmdId::DestinyLevelUpCmd,
            35,
            DestinyLevelUpRequest {
                hero_id: Some(HERO_ID),
                level: Some(1),
            },
        ),
    )
    .await;

    let first_packet = read_server_packet(&mut client).await;
    assert_eq!(first_packet.cmd_id, CmdId::HeroHeroUpdatePushCmd as i16);
    let reply_packet = read_server_packet(&mut client).await;
    let reply = reply_packet
        .decode_message::<DestinyLevelUpReply>()
        .unwrap();
    assert_eq!(reply_packet.cmd_id, CmdId::DestinyLevelUpCmd as i16);
    assert_eq!(reply_packet.result_code, 0);
    assert_eq!(reply.level, Some(1));

    close_and_cleanup(temp_db, ctx, client).await;
}

#[tokio::test]
async fn destiny_reconnect_returns_committed_state() {
    let (temp_db, ctx, mut client) = test_connection_with_destiny_hero(0, 0, 0).await;
    let pool = ctx.lock().await.state.db.clone();

    dispatch_and_flush(
        Arc::clone(&ctx),
        client_packet(
            CmdId::DestinyRankUpCmd,
            41,
            DestinyRankUpRequest {
                hero_id: Some(HERO_ID),
            },
        ),
    )
    .await;
    let _item_push = read_server_packet(&mut client).await;
    let _hero_push = read_server_packet(&mut client).await;
    let _reply = read_server_packet(&mut client).await;
    drop(client);
    drop(ctx);

    let (reconnected_ctx, mut reconnected_client) = test_connection_with_pool(pool.clone()).await;
    dispatch_and_flush(
        Arc::clone(&reconnected_ctx),
        client_packet(CmdId::HeroInfoListCmd, 42, HeroInfoListRequest {}),
    )
    .await;
    let reply_packet = read_server_packet(&mut reconnected_client).await;
    let reply = reply_packet.decode_message::<HeroInfoListReply>().unwrap();
    let hero = reply
        .heros
        .iter()
        .find(|hero| hero.hero_id == HERO_ID)
        .expect("reconnected hero list should contain the committed Destiny hero");
    assert_eq!(hero.destiny_rank, Some(1));
    assert_eq!(hero.destiny_level, Some(1));
    assert_eq!(hero.destiny_stone, Some(0));

    pool.close().await;
    drop(reconnected_ctx);
    drop(reconnected_client);
    temp_db.cleanup();
}

#[test]
fn all_37_configured_heroes_can_enter_stage_one_from_zero() {
    init_config();
    let catalog = config::destiny::DestinyConfigIndex::try_from_game_db(config::configs::get())
        .expect("Destiny config index should build");
    let heroes = &config::configs::get().character_destiny;
    assert_eq!(heroes.len(), 37);

    for hero in heroes.iter() {
        let current = OwnedDestinyHero {
            hero_uid: hero.hero_id as i64,
            user_id: USER_ID,
            hero_id: hero.hero_id,
            state: DestinyState {
                rank: 0,
                level: 0,
                stone: 0,
            },
            unlocked_stones: Vec::new(),
        };
        let plan = plan_transition(
            &catalog,
            &current,
            DestinyCommand::RankUp {
                hero_id: hero.hero_id,
            },
        )
        .expect("configured hero should be able to enter Destiny stage one");
        assert_eq!(plan.expected, current.state);
        assert!(matches!(
            plan.kind,
            MutationKind::Progress {
                target_rank: 1,
                target_level: 1
            }
        ));
    }
}

#[test]
fn every_configured_hero_can_plan_all_25_nodes() {
    init_config();
    let catalog = config::destiny::DestinyConfigIndex::try_from_game_db(config::configs::get())
        .expect("Destiny config index should build");

    for hero in config::configs::get().character_destiny.iter() {
        let mut current = OwnedDestinyHero {
            hero_uid: hero.hero_id as i64,
            user_id: USER_ID,
            hero_id: hero.hero_id,
            state: DestinyState {
                rank: 0,
                level: 0,
                stone: 0,
            },
            unlocked_stones: Vec::new(),
        };

        for (target_rank, max_node) in [(1, 5), (2, 5), (3, 5), (4, 10)] {
            let rank_plan = plan_transition(
                &catalog,
                &current,
                DestinyCommand::RankUp {
                    hero_id: hero.hero_id,
                },
            )
            .expect("configured hero should be able to plan the next Destiny stage");
            assert!(matches!(
                rank_plan.kind,
                MutationKind::Progress {
                    target_rank: planned_rank,
                    target_level: 1
                } if planned_rank == target_rank
            ));
            current.state.rank = target_rank;
            current.state.level = 1;

            let level_plan = plan_transition(
                &catalog,
                &current,
                DestinyCommand::LevelUp {
                    hero_id: hero.hero_id,
                    target_level: max_node,
                },
            )
            .expect("configured hero should be able to plan through node five");
            assert!(matches!(
                level_plan.kind,
                MutationKind::Progress {
                    target_rank: planned_rank,
                    target_level: max_node
                } if planned_rank == target_rank
            ));
            current.state.level = max_node;
        }
    }
}
