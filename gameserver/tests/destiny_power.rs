use std::sync::{Arc, Once};

use rand::SeedableRng;
use gameserver::state::battle::destiny::{
    DestinyState, HeroBuildContext, HeroSource,
};
use gameserver::state::battle::entity_builder::resolve_entity_kit;
use gameserver::state::battle::fight_builder::{
    attacker_substitute_uid, attacker_trial_uid_count, build_attacker_team, enemy_entity_uid,
    try_attacker_substitute_uid,
};
use gameserver::state::battle::effects::effect_types::EffectType;
use gameserver::state::battle::generate_ai_initial_deck;
use gameserver::state::battle::manager::buff_mgr::BuffMgr;
use gameserver::state::battle::manager::card_mgr::FightCardMgr;
use gameserver::state::battle::manager::calculate_mgr::{
    FightCalculateDataMgr, apply_power_info_change,
};
use gameserver::state::battle::manager::entity_mgr::FightEntityDataMgr;
use gameserver::state::battle::manager::round_mgr::FightRoundMgr;
use gameserver::state::battle::manager::fight_data_mgr::FightDataMgr;
use gameserver::state::battle::round::RoundState;
use gameserver::state::battle::skill_executor::SkillExecutor;
use gameserver::state::battle::destiny::{DestinyModifierMap, ResolvedDestinyAttributes};
use sonettobuf::{
    ActEffect, BeginRoundOper, CardInfo, Fight, FightEntityInfo, FightTeam, HeroAttribute,
    HeroSpAttribute, PowerInfo, FightGroup,
};

static CONFIG_INIT: Once = Once::new();

fn init_config() {
    CONFIG_INIT.call_once(|| {
        let data_dir = std::env::var("JSON_DATA_DIR")
            .expect("JSON_DATA_DIR must point at the international 3.6 runtime excel2json directory");
        config::configs::init(&data_dir).expect("failed to initialize config data");
    });
}

fn power_context(hero_id: i32, facet_id: i32) -> HeroBuildContext {
    init_config();
    let character = config::configs::get()
        .character
        .get(hero_id)
        .expect("power fixture character should exist");
    HeroBuildContext {
        hero_id,
        skin: character.skin_id,
        rank: character.rank,
        ex_skill_level: 5,
        destiny: DestinyState {
            rank: 4,
            level: 1,
            facet_id,
        },
        is_substitute: false,
        hero_type: character.hero_type,
        source: HeroSource::Owned,
    }
}

fn power_entity(uid: i64, power_infos: Vec<PowerInfo>) -> FightEntityInfo {
    FightEntityInfo {
        uid: Some(uid),
        power_infos,
        ..Default::default()
    }
}

#[test]
fn five_runtime_power_add_facets_initialize_one_to_five() {
    let fixtures = [
        (3025, 302501),
        (3039, 303902),
        (3041, 304101),
        (3048, 304801),
        (3053, 305301),
    ];

    for (hero_id, facet_id) in fixtures {
        let kit = resolve_entity_kit(&power_context(hero_id, facet_id))
            .expect("power fixture should resolve");
        assert_eq!(
            kit.power_infos,
            vec![PowerInfo {
                power_id: Some(1),
                num: Some(0),
                max: Some(5),
            }],
            "hero {hero_id} facet {facet_id}"
        );
    }
}

#[test]
fn non_power_destiny_facet_does_not_invent_power_info() {
    let kit = resolve_entity_kit(&power_context(3110, 311001))
        .expect("non-power facet should resolve");
    assert!(kit.power_infos.is_empty());
}

#[test]
fn power_info_change_is_absolute_upsert_and_clamped() {
    let mut entity = power_entity(
        42,
        vec![PowerInfo {
            power_id: Some(1),
            num: Some(2),
            max: Some(5),
        }],
    );

    apply_power_info_change(
        &mut entity,
        &PowerInfo {
            power_id: Some(1),
            num: Some(99),
            max: Some(5),
        },
    )
    .expect("existing power should update");
    assert_eq!(entity.power_infos[0].num, Some(5));

    apply_power_info_change(
        &mut entity,
        &PowerInfo {
            power_id: Some(2),
            num: Some(-4),
            max: Some(3),
        },
    )
    .expect("new power should upsert");
    assert_eq!(
        entity.power_infos,
        vec![
            PowerInfo {
                power_id: Some(1),
                num: Some(5),
                max: Some(5),
            },
            PowerInfo {
                power_id: Some(2),
                num: Some(0),
                max: Some(3),
            },
        ]
    );

    apply_power_info_change(
        &mut entity,
        &PowerInfo {
            power_id: Some(1),
            num: Some(3),
            max: Some(5),
        },
    )
    .expect("ordinary absolute power update should preserve the incoming value");
    assert_eq!(entity.power_infos[0].num, Some(3));
}

#[test]
fn power_info_change_rejects_missing_absolute_num() {
    let mut entity = power_entity(
        42,
        vec![PowerInfo {
            power_id: Some(1),
            num: Some(2),
            max: Some(5),
        }],
    );
    assert!(apply_power_info_change(
        &mut entity,
        &PowerInfo {
            power_id: Some(1),
            num: None,
            max: Some(5),
        },
    )
    .is_err());
    assert_eq!(entity.power_infos[0].num, Some(2));
}

#[test]
fn power_info_change_rejects_missing_absolute_max() {
    let mut entity = power_entity(
        42,
        vec![PowerInfo {
            power_id: Some(1),
            num: Some(2),
            max: Some(5),
        }],
    );
    assert!(apply_power_info_change(
        &mut entity,
        &PowerInfo {
            power_id: Some(1),
            num: Some(3),
            max: None,
        },
    )
    .is_err());
    assert_eq!(entity.power_infos[0].num, Some(2));
}

#[test]
fn skill_executor_retains_destiny_damage_sidecar_without_aliasing_it() {
    let mut modifiers = DestinyModifierMap::new();
    modifiers.insert(
        7,
        ResolvedDestinyAttributes {
            real_hurt_rate: 75,
            poison_add_rate: 125,
            ..Default::default()
        },
    );
    let executor = SkillExecutor::new(std::collections::HashMap::new(), modifiers);
    let modifier = executor
        .modifier_for_uid(7)
        .expect("sidecar should reach skill executor");
    assert_eq!(modifier.real_hurt_rate, 75);
    assert_eq!(modifier.poison_add_rate, 125);
    assert_eq!(modifier.sp_attr.big_skill_rate, None);
}

#[test]
fn power_info_change_updates_substitute_by_uid() {
    let mut fight = Fight {
        attacker: Some(FightTeam {
            sub_entitys: vec![power_entity(
                -42,
                vec![PowerInfo {
                    power_id: Some(1),
                    num: Some(0),
                    max: Some(5),
                }],
            )],
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut calc = FightCalculateDataMgr::new(Arc::new(fight.clone()));
    let mut bloodtithe = Default::default();
    let mut buffs = BuffMgr::new();
    let mut effect = ActEffect {
            effect_type: Some(EffectType::PowerInfoChange as i32),
            target_id: Some(-42),
            power_info: Some(PowerInfo {
                power_id: Some(1),
                num: Some(99),
                max: Some(5),
            }),
            ..Default::default()
        };
    calc.play_act_effect_data(
        &mut effect,
        &mut fight,
        &mut bloodtithe,
        &mut buffs,
    )
    .expect("substitute power update should resolve");

    assert_eq!(
        fight
            .attacker
            .as_ref()
            .unwrap()
            .sub_entitys[0]
            .power_infos[0]
            .num,
        Some(5)
    );
    assert_eq!(effect.power_info.unwrap().num, Some(5));
    let snapshot = calc.build_ex_point_info(&fight);
    assert_eq!(snapshot[0].power_infos[0].num, Some(5));
}

#[test]
fn round_snapshot_retains_power_infos_for_main_and_substitute() {
    let fight = Fight {
        attacker: Some(FightTeam {
            entitys: vec![power_entity(
                1,
                vec![PowerInfo {
                    power_id: Some(1),
                    num: Some(2),
                    max: Some(5),
                }],
            )],
            sub_entitys: vec![power_entity(
                -1,
                vec![PowerInfo {
                    power_id: Some(1),
                    num: Some(3),
                    max: Some(5),
                }],
            )],
            ..Default::default()
        }),
        ..Default::default()
    };
    let state = RoundState::new(&fight).expect("round state should build");
    let snapshot = state.export_snapshot();

    assert_eq!(snapshot.ex_point_info.len(), 2);
    assert!(snapshot
        .ex_point_info
        .iter()
        .any(|info| info.uid == Some(1) && info.power_infos[0].num == Some(2)));
    assert!(snapshot
        .ex_point_info
        .iter()
        .any(|info| info.uid == Some(-1) && info.power_infos[0].num == Some(3)));
}

#[tokio::test]
async fn fight_data_mgr_initial_round_retains_power_infos() {
    let fight = Fight {
        attacker: Some(FightTeam {
            entitys: vec![power_entity(
                1,
                vec![PowerInfo {
                    power_id: Some(1),
                    num: Some(1),
                    max: Some(5),
                }],
            )],
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut modifiers = DestinyModifierMap::new();
    modifiers.insert(
        1,
        ResolvedDestinyAttributes {
            sp_attr: HeroSpAttribute {
                clutch: Some(9),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    let mut manager = FightDataMgr::new_with_destiny_modifiers(fight, modifiers);
    let round = manager
        .build_initial_round(vec![], vec![])
        .expect("initial round should build");
    let info = round
        .ex_point_info
        .iter()
        .find(|info| info.uid == Some(1))
        .expect("main hero power should be present");
    assert_eq!(info.power_infos[0].num, Some(1));
    let sp = round
        .hero_sp_attributes
        .iter()
        .find(|info| info.uid == Some(1))
        .and_then(|info| info.attribute.as_ref())
        .expect("main hero special attributes should be present");
    assert_eq!(sp.clutch, Some(9));
}

#[tokio::test]
async fn fight_round_mgr_later_round_retains_power_infos() {
    let mut fight = Fight {
        attacker: Some(FightTeam {
            entitys: vec![power_entity(
                1,
                vec![PowerInfo {
                    power_id: Some(1),
                    num: Some(2),
                    max: Some(5),
                }],
            )],
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut apply_calc = FightCalculateDataMgr::new(Arc::new(fight.clone()));
    let mut apply_bloodtithe = Default::default();
    let mut apply_buffs = BuffMgr::new();
    let mut effect = ActEffect {
        effect_type: Some(EffectType::PowerInfoChange as i32),
        target_id: Some(1),
        power_info: Some(PowerInfo {
            power_id: Some(1),
            num: Some(4),
            max: Some(5),
        }),
        ..Default::default()
    };
    apply_calc
        .play_act_effect_data(
            &mut effect,
            &mut fight,
            &mut apply_bloodtithe,
            &mut apply_buffs,
        )
        .expect("effect 295 should update authoritative power");
    assert_eq!(effect.power_info.as_ref().unwrap().num, Some(4));
    let fight_arc = Arc::new(fight.clone());
    let round_mgr = FightRoundMgr::new(fight_arc.clone());
    let card_mgr = FightCardMgr::new(fight_arc.clone());
    let mut calc = FightCalculateDataMgr::new(fight_arc);
    let mut rng = rand::rngs::StdRng::seed_from_u64(11);
    let mut bloodtithe = Default::default();
    let mut buffs = BuffMgr::new();
    let round = round_mgr
        .process_round(
            &mut rng,
            &card_mgr,
            &mut calc,
            &mut fight,
            &mut bloodtithe,
            vec![],
            vec![],
            vec![],
            &mut buffs,
        )
        .await
        .expect("later round should build");
    let info = round
        .ex_point_info
        .iter()
        .find(|info| info.uid == Some(1))
        .expect("later-round hero power should be present");
    assert_eq!(info.power_infos[0].num, Some(4));
}

#[tokio::test]
async fn final_hit_marks_the_same_round_finished() {
    init_config();
    let damage_skill = config::configs::get()
        .skill_effect
        .iter()
        .find(|effect| {
            effect.damage_rate > 0
                && [
                    &effect.behavior1,
                    &effect.behavior2,
                    &effect.behavior3,
                    &effect.behavior4,
                    &effect.behavior5,
                    &effect.behavior6,
                    &effect.behavior7,
                    &effect.behavior8,
                    &effect.behavior9,
                    &effect.behavior10,
                    &effect.behavior11,
                    &effect.behavior12,
                    &effect.behavior13,
                    &effect.behavior14,
                    &effect.behavior15,
                    &effect.behavior16,
                    &effect.behavior17,
                    &effect.behavior18,
                    &effect.behavior19,
                    &effect.behavior20,
                ]
                .iter()
                .all(|behavior| behavior.is_empty())
        })
        .expect("runtime should contain a fallback damage skill")
        .id;
    let hero_id = 3001;
    let attacker = FightEntityInfo {
        uid: Some(1),
        model_id: Some(hero_id),
        team_type: Some(1),
        current_hp: Some(100),
        attr: Some(HeroAttribute {
            hp: Some(100),
            attack: Some(1_000),
            defense: Some(0),
            mdefense: Some(0),
            ..Default::default()
        }),
        ..Default::default()
    };
    let defender = FightEntityInfo {
        uid: Some(enemy_entity_uid(0)),
        team_type: Some(2),
        current_hp: Some(1),
        attr: Some(HeroAttribute {
            hp: Some(1),
            attack: Some(0),
            defense: Some(0),
            mdefense: Some(0),
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut fight = Fight {
        attacker: Some(FightTeam {
            entitys: vec![attacker],
            ..Default::default()
        }),
        defender: Some(FightTeam {
            entitys: vec![defender],
            ..Default::default()
        }),
        ..Default::default()
    };
    let fight_arc = Arc::new(fight.clone());
    let round_mgr = FightRoundMgr::new(fight_arc.clone());
    let card_mgr = FightCardMgr::new(fight_arc.clone());
    let mut calc = FightCalculateDataMgr::new(fight_arc);
    let mut rng = rand::rngs::StdRng::seed_from_u64(17);
    let mut bloodtithe = Default::default();
    let mut buffs = BuffMgr::new();
    let round = round_mgr
        .process_round(
            &mut rng,
            &card_mgr,
            &mut calc,
            &mut fight,
            &mut bloodtithe,
            vec![BeginRoundOper {
                oper_type: Some(1),
                param1: Some(0),
                to_id: Some(enemy_entity_uid(0)),
                ..Default::default()
            }],
            vec![CardInfo {
                hero_id: Some(hero_id),
                skill_id: Some(damage_skill),
                ..Default::default()
            }],
            vec![],
            &mut buffs,
        )
        .await
        .expect("final-hit round should build");

    assert_eq!(fight.defender.as_ref().unwrap().entitys[0].current_hp, Some(0));
    assert_eq!(round.is_finish, Some(true));
}

#[test]
fn substitute_entity_is_globally_addressable() {
    let fight = Fight {
        attacker: Some(FightTeam {
            sub_entitys: vec![power_entity(-7, vec![])],
            ..Default::default()
        }),
        ..Default::default()
    };
    let mgr = FightEntityDataMgr::new(Arc::new(fight));
    assert!(mgr.get_by_id(-7).is_some());
    assert!(mgr.get_location(-7).is_some());
}

#[test]
fn trial_and_defender_uids_use_distinct_global_namespaces() {
    let fight = Fight {
        attacker: Some(FightTeam {
            entitys: vec![FightEntityInfo {
                uid: Some(-1),
                team_type: Some(1),
                current_hp: Some(100),
                ..Default::default()
            }],
            ..Default::default()
        }),
        defender: Some(FightTeam {
            entitys: vec![FightEntityInfo {
                uid: Some(enemy_entity_uid(0)),
                team_type: Some(2),
                current_hp: Some(100),
                ..Default::default()
            }],
            ..Default::default()
        }),
        ..Default::default()
    };
    assert_eq!(enemy_entity_uid(0), -1001);
    let entities = FightEntityDataMgr::new(Arc::new(fight.clone()));
    assert!(entities.get_by_id(-1).is_some());
    assert!(entities.get_by_id(-1001).is_some());

    let state = RoundState::new(&fight).unwrap();
    let rounds = FightRoundMgr::new(Arc::new(fight));
    assert!(!rounds.check_battle_end(&state));
}

#[test]
fn substitute_uids_start_after_trial_uids() {
    assert_eq!(attacker_substitute_uid(0, 0), -1);
    assert_eq!(attacker_substitute_uid(1, 0), -2);
    assert_eq!(attacker_substitute_uid(2, 1), -4);
    assert_ne!(attacker_substitute_uid(1, 0), -1);
}

#[test]
fn sparse_legacy_trial_uids_reserve_their_wire_ordinals() {
    let trial_count = attacker_trial_uid_count(&[-2], 0);
    assert_eq!(trial_count, 2);
    assert_eq!(attacker_substitute_uid(trial_count, 0), -3);
}

#[test]
fn attacker_uid_allocator_rejects_the_defender_namespace() {
    assert!(try_attacker_substitute_uid(1_000, 0).is_err());
    assert_eq!(attacker_trial_uid_count(&[-1_001], 0), 1_001);
}

#[tokio::test]
async fn legacy_trial_minus_1000_reports_namespace_error_before_db_lookup() {
    init_config();
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    let group = FightGroup {
        hero_list: vec![-1_000],
        sub_hero_list: vec![1],
        ..Default::default()
    };

    let error = build_attacker_team(&pool, 7, &group)
        .await
        .expect_err("substitute UID must not enter the defender namespace");
    assert!(error.to_string().contains("crosses the defender UID namespace"));
}

#[tokio::test]
async fn legacy_trial_minus_1001_is_rejected_before_namespace_collision() {
    init_config();
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    let group = FightGroup {
        hero_list: vec![-1_001],
        ..Default::default()
    };

    let error = build_attacker_team(&pool, 7, &group)
        .await
        .expect_err("legacy trial UID must not enter the defender namespace");
    assert!(error.to_string().contains("crosses the defender UID namespace"));
}

#[test]
fn round_state_inherits_an_already_finished_fight() {
    let fight = Fight {
        is_finish: Some(true),
        ..Default::default()
    };
    let state = RoundState::new(&fight).expect("round state should build");
    assert!(state.is_finish);
}

#[tokio::test]
async fn trial_entities_can_cast_cards_and_receive_ai_targets() {
    init_config();
    let enemy_skill = config::configs::get()
        .skill
        .iter()
        .find(|skill| skill.skill_effect != 0)
        .expect("runtime should contain an AI skill")
        .id;
    let trial = FightEntityInfo {
        uid: Some(-1),
        model_id: Some(3041),
        team_type: Some(1),
        current_hp: Some(100),
        attr: Some(HeroAttribute {
            hp: Some(100),
            attack: Some(100),
            defense: Some(10),
            mdefense: Some(10),
            ..Default::default()
        }),
        ..Default::default()
    };
    let enemy = FightEntityInfo {
        uid: Some(enemy_entity_uid(0)),
        model_id: Some(4030701),
        team_type: Some(2),
        current_hp: Some(100),
        skill_group1: vec![enemy_skill],
        attr: Some(HeroAttribute {
            hp: Some(100),
            attack: Some(100),
            defense: Some(10),
            mdefense: Some(10),
            ..Default::default()
        }),
        ..Default::default()
    };
    let fight = Fight {
        attacker: Some(FightTeam {
            entitys: vec![trial],
            ..Default::default()
        }),
        defender: Some(FightTeam {
            entitys: vec![enemy],
            ..Default::default()
        }),
        ..Default::default()
    };

    let ai_cards = generate_ai_initial_deck(&fight, 7).await;
    assert_eq!(ai_cards.len(), 1);
    assert_eq!(ai_cards[0].target_uid, Some(-1));

    let mut state = RoundState::new(&fight).unwrap();
    state.player_deck = vec![CardInfo {
        hero_id: Some(3041),
        skill_id: Some(0),
        ..Default::default()
    }];
    let card_mgr = FightCardMgr::new(Arc::new(fight));
    let mut rng = rand::rngs::StdRng::seed_from_u64(7);
    let step = card_mgr
        .execute_operation(
            &mut rng,
            &mut state,
            BeginRoundOper {
                oper_type: Some(1),
                param1: Some(0),
                to_id: Some(enemy_entity_uid(0)),
                ..Default::default()
            },
        )
        .await
        .expect("trial card should find its negative UID caster");
    assert_eq!(step.from_id, Some(-1));
}
