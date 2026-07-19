use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    sync::Once,
};

use database::models::game::heros::{Hero, HeroData};
use gameserver::state::battle::destiny::{
    DestinyResolveError, DestinyState, HeroBaseAttributes, HeroBuildContext, HeroSource,
    ResolvedHeroKit, resolve_destiny_attributes,
};
use gameserver::state::battle::entity_builder::{
    build_hero_entity, resolve_entity_destiny_attributes, resolve_entity_kit,
};
use gameserver::state::battle::fight_builder::{
    build_attacker_team, build_attacker_team_with_destiny_modifiers,
};
use gameserver::state::battle::generate_initial_deck;
use gameserver::state::battle::trial::normalize_trial_requests;
use sonettobuf::{FightGroup, TrialHero};
use sqlx::SqlitePool;
use sqlx::sqlite::SqlitePoolOptions;

static CONFIG_INIT: Once = Once::new();

fn init_config() {
    CONFIG_INIT.call_once(|| {
        let data_dir = std::env::var("JSON_DATA_DIR").expect(
            "JSON_DATA_DIR must point at the international 3.6 runtime excel2json directory",
        );
        config::configs::init(&data_dir).expect("failed to initialize config data");
    });
}

fn context(
    hero_id: i32,
    ex_skill_level: i32,
    destiny_rank: i32,
    facet_id: i32,
    source: HeroSource,
    is_substitute: bool,
) -> HeroBuildContext {
    init_config();
    let character = config::configs::get()
        .character
        .get(hero_id)
        .expect("character fixture should exist");
    HeroBuildContext {
        hero_id,
        skin: character.skin_id,
        rank: 3,
        ex_skill_level,
        destiny: DestinyState {
            rank: destiny_rank,
            level: if destiny_rank == 0 { 0 } else { 1 },
            facet_id,
        },
        is_substitute,
        hero_type: character.hero_type,
        source,
    }
}

fn kit(ctx: HeroBuildContext) -> ResolvedHeroKit {
    resolve_entity_kit(&ctx).expect("entity kit should resolve")
}

fn hero_for_facet(facet_id: i32) -> i32 {
    init_config();
    config::configs::get()
        .character_destiny
        .iter()
        .find(|hero| {
            hero.facets_id
                .split('#')
                .filter_map(|value| value.parse::<i32>().ok())
                .any(|id| id == facet_id)
        })
        .map(|hero| hero.hero_id)
        .unwrap_or_else(|| panic!("missing hero owner for facet {facet_id}"))
}

fn owned_hero(hero_id: i32, ex_skill_level: i32, facet_id: i32, facet_rank: i32) -> HeroData {
    init_config();
    let character = config::configs::get()
        .character
        .get(hero_id)
        .expect("character fixture should exist");
    HeroData {
        record: Hero {
            uid: 42,
            user_id: 7,
            hero_id,
            create_time: 0,
            level: 180,
            exp: 0,
            rank: character.rank,
            breakthrough: 0,
            skin: character.skin_id,
            faith: 0,
            active_skill_level: 1,
            ex_skill_level,
            is_new: false,
            talent: 0,
            default_equip_uid: 0,
            duplicate_count: 0,
            use_talent_template_id: 0,
            talent_style_unlock: 0,
            talent_style_red: 0,
            is_favor: false,
            destiny_rank: facet_rank,
            destiny_level: if facet_rank == 0 { 0 } else { 1 },
            destiny_stone: facet_id,
            red_dot: 0,
            extra_str: String::new(),
            base_hp: 10_000,
            base_attack: 1_000,
            base_defense: 500,
            base_mdefense: 500,
            base_technic: 100,
            base_multi_hp_idx: 0,
            base_multi_hp_num: 0,
            ex_cri: 0,
            ex_recri: 0,
            ex_cri_dmg: 0,
            ex_cri_def: 0,
            ex_add_dmg: 0,
            ex_drop_dmg: 0,
        },
        passive_skill_levels: vec![],
        voices: vec![],
        voices_heard: vec![],
        skin_list: vec![],
        sp_attr: None,
        equip_attrs: vec![],
        item_unlocks: vec![],
        talent_cubes: vec![],
        talent_templates: vec![],
        destiny_stone_unlocks: vec![],
    }
}

async fn owned_hero_pool() -> SqlitePool {
    init_config();
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    database::run_migrations(&pool).await.unwrap();
    sqlx::query("INSERT INTO users (id, username, created_at, updated_at) VALUES (7, 'trial-position-test', 0, 0)")
        .execute(&pool)
        .await
        .unwrap();
    let character = config::configs::get()
        .character
        .get(3025)
        .expect("owned hero fixture should exist");
    sqlx::query(
        "INSERT INTO heroes
         (uid, user_id, hero_id, create_time, level, exp, rank, breakthrough,
          skin, faith, active_skill_level, ex_skill_level, destiny_rank,
          destiny_level, destiny_stone, base_hp, base_attack, base_defense,
          base_mdefense, base_technic)
         VALUES (42, 7, 3025, 0, 180, 0, ?, 0, ?, 0, 1, 1, 0, 0, 0,
                 10000, 1000, 500, 500, 100)",
    )
    .bind(character.rank)
    .bind(character.skin_id)
    .execute(&pool)
    .await
    .unwrap();
    pool
}

#[tokio::test]
async fn live_owned_main_and_substitute_use_the_same_resolved_kit() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    let hero = owned_hero(3110, 5, 311001, 4);
    let expected = kit(context(3110, 5, 4, 311001, HeroSource::Owned, false));
    let main = build_hero_entity(&pool, &hero, 1, 1, false).await.unwrap();
    let substitute = build_hero_entity(&pool, &hero, -1, 1, true).await.unwrap();

    assert_eq!(main.skill_group1, substitute.skill_group1);
    assert_eq!(main.skill_group2, substitute.skill_group2);
    assert_eq!(main.passive_skill, substitute.passive_skill);
    assert_eq!(main.ex_skill, substitute.ex_skill);
    assert_eq!(main.power_infos, substitute.power_infos);
    assert_eq!(main.skill_group1, expected.skill_group_1);
    assert_eq!(main.skill_group2, expected.skill_group_2);
    assert_eq!(main.passive_skill, expected.passives);
    assert_eq!(main.ex_skill, Some(expected.ultimate));
    assert_eq!(main.power_infos, expected.power_infos);
    let attrs = main.attr.as_ref().expect("main entity attrs");
    assert!(attrs.hp.unwrap() > 10_987);
    assert!(attrs.attack.unwrap() > 1_079);
    assert!(attrs.defense.unwrap() > 547);
    assert!(attrs.mdefense.unwrap() > 547);
}

#[tokio::test]
async fn owned_default_skin_is_not_inferred_as_activity_role() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    let hero = owned_hero(3101, 0, 0, 0);

    let entity = build_hero_entity(&pool, &hero, 1, 1, false)
        .await
        .expect("owned hero should use the owned source path");

    assert_eq!(entity.passive_skill, vec![31010141, 31010142]);
}

#[tokio::test]
async fn destiny_percentages_use_pre_equipment_base_attributes() {
    init_config();
    let game = config::configs::get();
    let strengthen = game
        .equip_strengthen
        .iter()
        .find(|row| row.hp != 0 || row.atk != 0 || row.def != 0 || row.mdef != 0)
        .expect("runtime fixture should contain equipment strengthening");
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::query(
        "CREATE TABLE equipment (uid INTEGER PRIMARY KEY, user_id INTEGER NOT NULL, equip_id INTEGER NOT NULL, level INTEGER NOT NULL, exp INTEGER NOT NULL, break_lv INTEGER NOT NULL, count INTEGER NOT NULL, is_lock BOOLEAN NOT NULL, refine_lv INTEGER NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO equipment (uid, user_id, equip_id, level, exp, break_lv, count, is_lock, refine_lv, created_at, updated_at) VALUES (1, 7, ?, 1, 0, 0, 1, 0, 0, 0, 0)",
    )
    .bind(strengthen.strength_type)
    .execute(&pool)
    .await
    .unwrap();

    let mut hero = owned_hero(3098, 5, 309801, 4);
    hero.record.default_equip_uid = 1;
    let entity = build_hero_entity(&pool, &hero, 1, 1, false)
        .await
        .expect("equipped hero should build");

    let base = HeroBaseAttributes {
        hp: ((hero.record.base_hp as f32) * 1.0986541).round() as i32,
        attack: ((hero.record.base_attack as f32) * 1.0786).round() as i32,
        defense: ((hero.record.base_defense as f32) * 1.0942857).round() as i32,
        mdefense: ((hero.record.base_mdefense as f32) * 1.0942857).round() as i32,
    };
    let resolved = resolve_destiny_attributes(
        &config::destiny::DestinyConfigIndex::try_from_game_db(game).unwrap(),
        hero.record.hero_id,
        DestinyState {
            rank: hero.record.destiny_rank,
            level: hero.record.destiny_level,
            facet_id: hero.record.destiny_stone,
        },
        base,
    )
    .unwrap();
    let attrs = entity.attr.as_ref().unwrap();
    assert_eq!(attrs.hp, Some(base.hp + strengthen.hp + resolved.hp));
    assert_eq!(
        attrs.attack,
        Some(base.attack + strengthen.atk + resolved.attack)
    );
}

#[test]
fn hero_3088_keeps_compatibility_passives_once() {
    let resolved = kit(context(3088, 5, 4, 308801, HeroSource::Activity, false));

    assert_eq!(
        resolved.passives,
        vec![
            30880141, 30880151, 308801611, 308801911, 308801921, 308802111
        ]
    );
    for bonus in [308801911, 308801921, 308802111] {
        assert_eq!(
            resolved.passives.iter().filter(|id| **id == bonus).count(),
            1
        );
    }
}

#[test]
fn activity174_kit_uses_role_specific_passives() {
    let resolved = kit(context(3012, 0, 0, 0, HeroSource::Activity, false));

    assert_eq!(resolved.skill_group_1, vec![30120111, 30120112, 30120113]);
    assert_eq!(resolved.skill_group_2, vec![30120121, 30120122, 30120123]);
    assert_eq!(resolved.ultimate, 30120131);
    assert_eq!(resolved.passives, vec![6230812, 30120142]);
}

#[test]
fn activity174_composite_passives_follow_client_number_split() {
    let hero_3101 = kit(context(3101, 0, 0, 0, HeroSource::Activity, false));
    assert_eq!(hero_3101.passives, vec![31010141]);

    let hero_3103 = kit(context(3103, 0, 0, 0, HeroSource::Activity, false));
    assert_eq!(hero_3103.passives, vec![31030141, 31030151]);
}

#[test]
fn hero_3088_foreign_facet_is_inactive_without_compatibility_bonuses() {
    let resolved = kit(context(3088, 5, 4, 311001, HeroSource::Owned, false));
    assert!(!resolved.passives.contains(&308801911));
    assert!(!resolved.passives.contains(&308801921));
    assert!(!resolved.passives.contains(&308802111));
}

#[test]
fn all_43_facets_cover_four_levels() {
    init_config();
    let mut levels_by_facet = BTreeMap::<i32, BTreeSet<i32>>::new();

    for facet in config::configs::get().character_destiny_facets.iter() {
        levels_by_facet
            .entry(facet.facets_id)
            .or_default()
            .insert(facet.level);
    }

    assert_eq!(levels_by_facet.len(), 43);
    let expected_levels = BTreeSet::from([1, 2, 3, 4]);

    for (facet_id, levels) in levels_by_facet {
        assert_eq!(levels, expected_levels, "facet {facet_id}");
    }
}

#[test]
fn all_valid_facet_rank_pairs_build_for_main_and_substitute() {
    init_config();
    let base = HeroBaseAttributes {
        hp: 10_000,
        attack: 1_000,
        defense: 500,
        mdefense: 500,
    };

    for facet in config::configs::get().character_destiny_facets.iter() {
        let hero_id = hero_for_facet(facet.facets_id);
        let main = context(
            hero_id,
            5,
            facet.level,
            facet.facets_id,
            HeroSource::Owned,
            false,
        );
        let substitute = context(
            hero_id,
            5,
            facet.level,
            facet.facets_id,
            HeroSource::Owned,
            true,
        );

        let main_kit = resolve_entity_kit(&main).expect("main facet kit should resolve");
        let substitute_kit =
            resolve_entity_kit(&substitute).expect("substitute facet kit should resolve");
        assert_eq!(
            main_kit, substitute_kit,
            "facet {} level {} hero {}",
            facet.facets_id, facet.level, hero_id
        );

        let main_attr = resolve_entity_destiny_attributes(&main, base).expect("main attrs");
        let substitute_attr =
            resolve_entity_destiny_attributes(&substitute, base).expect("substitute attrs");
        assert_eq!(
            main_attr, substitute_attr,
            "facet {} level {} hero {}",
            facet.facets_id, facet.level, hero_id
        );
    }
}

#[test]
fn all_37_heroes_cover_all_925_nodes() {
    init_config();
    let game = config::configs::get();
    let mut global_nodes = HashSet::new();

    assert_eq!(game.character_destiny.len(), 37);
    assert_eq!(game.character_destiny_slots.len(), 925);

    for hero in game.character_destiny.iter() {
        let nodes: HashSet<_> = game
            .character_destiny_slots
            .iter()
            .filter(|slot| slot.slots_id == hero.slots_id)
            .map(|slot| (slot.stage, slot.node))
            .collect();
        assert_eq!(nodes.len(), 25, "hero {}", hero.hero_id);
        assert!(
            nodes
                .iter()
                .all(|(stage, node)| (1..=4).contains(stage) && (1..=25).contains(node))
        );
        for node in nodes {
            assert!(global_nodes.insert((hero.hero_id, node.0, node.1)));
        }
    }

    assert_eq!(global_nodes.len(), 925);
}

#[test]
fn foreign_and_missing_facets_preserve_node_attributes_without_facet_kit() {
    let base = HeroBaseAttributes {
        hp: 10_000,
        attack: 1_000,
        defense: 500,
        mdefense: 500,
    };
    let no_facet = context(3088, 5, 4, 0, HeroSource::Owned, false);
    let foreign = context(3088, 5, 4, 311001, HeroSource::Owned, false);
    let missing = context(3088, 5, 4, 399999, HeroSource::Owned, false);

    let no_facet_attrs = resolve_entity_destiny_attributes(&no_facet, base.clone()).unwrap();
    let foreign_attrs = resolve_entity_destiny_attributes(&foreign, base.clone()).unwrap();
    let missing_attrs = resolve_entity_destiny_attributes(&missing, base).unwrap();
    let attrs = |value: &gameserver::state::battle::destiny::ResolvedDestinyAttributes| {
        (value.hp, value.attack, value.defense, value.mdefense)
    };

    assert_ne!(attrs(&no_facet_attrs), (0, 0, 0, 0));
    assert_eq!(attrs(&foreign_attrs), attrs(&no_facet_attrs));
    assert_eq!(attrs(&missing_attrs), attrs(&no_facet_attrs));
    assert_eq!(
        kit(foreign).passives,
        kit(no_facet.clone()).passives,
        "foreign facet may disable facet kit but not node passives"
    );
    assert_eq!(kit(missing).passives, kit(no_facet).passives);
}

#[tokio::test]
async fn entity_builder_rejects_partial_or_missing_destiny_state() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();

    for (rank, level) in [(1, 0), (0, 1)] {
        let mut hero = owned_hero(3025, 0, 0, rank);
        hero.record.destiny_level = level;
        let error = build_hero_entity(&pool, &hero, 1, 1, false)
            .await
            .expect_err("partial Destiny state must not be silently ignored");
        assert!(
            error
                .downcast_ref::<DestinyResolveError>()
                .is_some_and(|error| matches!(error, DestinyResolveError::InvalidState(_))),
            "unexpected partial-state error: {error:?}"
        );
    }

    let valid_zero = owned_hero(3025, 0, 0, 0);
    assert!(
        build_hero_entity(&pool, &valid_zero, 1, 1, false)
            .await
            .is_ok(),
        "the exact zero Destiny state remains a valid no-op"
    );

    let facet_without_progress = owned_hero(3025, 0, 302501, 0);
    let error = build_hero_entity(&pool, &facet_without_progress, 1, 1, false)
        .await
        .expect_err("a facet without rank and level must not become a no-op");
    assert!(
        error
            .downcast_ref::<DestinyResolveError>()
            .is_some_and(|error| matches!(error, DestinyResolveError::InvalidState(_))),
        "unexpected facet-only error: {error:?}"
    );

    let hero_without_destiny = owned_hero(3005, 0, 0, 1);
    let error = build_hero_entity(&pool, &hero_without_destiny, 1, 1, false)
        .await
        .expect_err("missing Destiny config must not be silently ignored");
    assert!(
        error
            .downcast_ref::<DestinyResolveError>()
            .is_some_and(|error| matches!(error, DestinyResolveError::InvalidConfig(_))),
        "unexpected missing-config error: {error:?}"
    );
}

#[tokio::test]
async fn live_builder_foreign_facet_does_not_add_destiny_attributes() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    let foreign = owned_hero(3088, 5, 311001, 4);
    let baseline = owned_hero(3088, 5, 0, 4);

    let foreign_entity = build_hero_entity(&pool, &foreign, 1, 1, false)
        .await
        .unwrap();
    let baseline_entity = build_hero_entity(&pool, &baseline, 1, 1, false)
        .await
        .unwrap();

    let foreign_attr = foreign_entity.attr.as_ref().expect("foreign attrs");
    let baseline_attr = baseline_entity.attr.as_ref().expect("baseline attrs");
    assert_eq!(foreign_attr.hp, baseline_attr.hp);
    assert_eq!(foreign_attr.attack, baseline_attr.attack);
    assert_eq!(foreign_attr.defense, baseline_attr.defense);
    assert_eq!(foreign_attr.mdefense, baseline_attr.mdefense);
}

#[tokio::test]
async fn live_builder_propagates_unresolvable_hero_kit() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    let mut hero = owned_hero(3110, 5, 0, 0);
    hero.record.hero_id = 9999;
    let result = build_hero_entity(&pool, &hero, 1, 1, false).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn live_builder_does_not_mask_non_facet_resolver_errors() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    let hero = owned_hero(3110, 6, 311001, 4);

    let result = build_hero_entity(&pool, &hero, 1, 1, false).await;

    assert!(result.is_err());
}

#[test]
fn trial_2241001_uses_explicit_trial_fields() {
    init_config();
    let trial = config::configs::get()
        .hero_trial
        .get(2241001)
        .expect("trial fixture should exist");
    let character = config::configs::get()
        .character
        .get(trial.hero_id)
        .expect("trial character should exist");
    let context = HeroBuildContext {
        hero_id: trial.hero_id,
        skin: trial.skin,
        rank: character.rank,
        ex_skill_level: trial.ex_skill_lv,
        destiny: DestinyState {
            rank: trial.facetslevel,
            level: 1,
            facet_id: trial.facets_id,
        },
        is_substitute: false,
        hero_type: character.hero_type,
        source: HeroSource::Trial,
    };
    let resolved = kit(context.clone());

    assert_eq!(resolved.skill_group_1, vec![30410314, 30410315, 30410316]);
    assert_eq!(resolved.skill_group_2, vec![30410324, 30410325, 30410326]);
    assert_eq!(resolved.ultimate, 30410334);
    assert_eq!(resolved.power_infos.len(), 1);
    assert_eq!(resolved.power_infos[0].power_id, Some(1));
    assert_eq!(resolved.power_infos[0].num, Some(0));
    assert_eq!(resolved.power_infos[0].max, Some(5));

    let level = config::configs::get()
        .character_level
        .iter()
        .find(|row| row.hero_id == trial.hero_id && row.level == trial.level)
        .expect("trial level fixture should exist");
    let attributes = resolve_entity_destiny_attributes(
        &context,
        HeroBaseAttributes {
            hp: level.hp,
            attack: level.atk,
            defense: level.def,
            mdefense: level.mdef,
        },
    )
    .expect("trial Destiny attributes should resolve");
    assert_eq!(
        (
            attributes.hp,
            attributes.attack,
            attributes.defense,
            attributes.mdefense,
        ),
        (495, 91, 36, 42)
    );
}

#[tokio::test]
async fn fight_group_trial_list_builds_trial_once_with_configured_kit() {
    init_config();
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    let group = FightGroup {
        trial_hero_list: vec![TrialHero {
            trial_id: Some(2241001),
            pos: Some(3),
            ..Default::default()
        }],
        ..Default::default()
    };

    let team = build_attacker_team(&pool, 7, &group).await.unwrap();
    assert_eq!(team.entitys.len(), 1);
    assert!(team.sub_entitys.is_empty());
    let trial = &team.entitys[0];
    assert_eq!(trial.uid, Some(-1));
    assert_eq!(trial.trial_id, Some(2241001));
    assert_eq!(trial.model_id, Some(3041));
    assert_eq!(trial.skin, Some(304102));
    assert_eq!(trial.position, Some(3));
    assert_eq!(trial.skill_group1, vec![30410314, 30410315, 30410316]);
    assert_eq!(trial.skill_group2, vec![30410324, 30410325, 30410326]);
    assert_eq!(trial.ex_skill, Some(30410334));
    assert_eq!(trial.power_infos.len(), 1);
    assert_eq!(trial.power_infos[0].power_id, Some(1));
    assert_eq!(trial.power_infos[0].num, Some(0));
    assert_eq!(trial.power_infos[0].max, Some(5));
    assert_eq!(trial.destiny_stone, Some(304101));
    assert_eq!(trial.destiny_rank, Some(4));
    let attr = trial.attr.as_ref().expect("trial attributes should exist");
    assert_eq!(attr.hp, Some(5_939));
    assert_eq!(attr.attack, Some(1_044));
    assert_eq!(attr.defense, Some(475));
    assert_eq!(attr.mdefense, Some(532));
}

#[tokio::test]
async fn trial_list_contributes_destiny_combat_modifiers() {
    init_config();
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    let group = FightGroup {
        trial_hero_list: vec![TrialHero {
            trial_id: Some(2241001),
            pos: Some(3),
            ..Default::default()
        }],
        ..Default::default()
    };

    let (_, modifiers) = build_attacker_team_with_destiny_modifiers(&pool, 7, &group)
        .await
        .unwrap();
    let modifier = modifiers
        .get(&-1)
        .expect("trial UID should retain its Destiny combat modifier");
    assert_eq!(
        (
            modifier.hp,
            modifier.attack,
            modifier.defense,
            modifier.mdefense,
        ),
        (495, 91, 36, 42)
    );
}

#[tokio::test]
async fn trial_list_contributes_cards_with_the_same_ordinal_uid() {
    init_config();
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    let group = FightGroup {
        trial_hero_list: vec![TrialHero {
            trial_id: Some(2241001),
            pos: Some(3),
            ..Default::default()
        }],
        ..Default::default()
    };

    let cards = generate_initial_deck(&pool, 7, &group, 0).await.unwrap();
    assert!(!cards.card_group.is_empty());
    assert!(
        cards
            .card_group
            .iter()
            .all(|card| card.hero_id == Some(3041))
    );
    assert!(
        cards
            .card_group
            .iter()
            .all(|card| card.temp_card == Some(true))
    );
}

#[tokio::test]
async fn legacy_negative_trial_id_remains_reachable_without_explicit_list() {
    init_config();
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    let trial_id = config::configs::get()
        .hero_trial
        .all()
        .first()
        .expect("legacy trial fixture should exist")
        .id;
    let group = FightGroup {
        hero_list: vec![-i64::from(trial_id)],
        ..Default::default()
    };

    let team = build_attacker_team(&pool, 7, &group).await.unwrap();
    assert_eq!(team.entitys.len(), 1);
    assert_eq!(team.entitys[0].uid, Some(-1));
    assert_eq!(team.entitys[0].trial_id, Some(trial_id));

    let cards = generate_initial_deck(&pool, 7, &group, 0).await.unwrap();
    assert!(!cards.card_group.is_empty());
    assert!(
        cards
            .card_group
            .iter()
            .all(|card| card.temp_card == Some(true))
    );
}

#[tokio::test]
async fn unsupported_battle_aid_placeholder_does_not_fail_trial_consumers() {
    init_config();
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    let group = FightGroup {
        hero_list: vec![-1],
        ..Default::default()
    };

    assert!(normalize_trial_requests(&group).unwrap().is_empty());
    let team = build_attacker_team(&pool, 7, &group).await.unwrap();
    assert!(team.entitys.is_empty());
    let cards = generate_initial_deck(&pool, 7, &group, 0).await.unwrap();
    assert!(cards.card_group.is_empty());
}

#[tokio::test]
async fn battle_aid_does_not_inflate_owned_card_source_count() {
    init_config();
    let pool = owned_hero_pool().await;
    let character = config::configs::get()
        .character
        .get(3025)
        .expect("owned hero fixture should exist");
    sqlx::query(
        "INSERT INTO heroes
         (uid, user_id, hero_id, create_time, level, exp, rank, breakthrough,
          skin, faith, active_skill_level, ex_skill_level, destiny_rank,
          destiny_level, destiny_stone, base_hp, base_attack, base_defense,
          base_mdefense, base_technic)
         VALUES (43, 7, 3025, 0, 180, 0, ?, 0, ?, 0, 1, 1, 0, 0, 0,
                 10000, 1000, 500, 500, 100)",
    )
    .bind(character.rank)
    .bind(character.skin_id)
    .execute(&pool)
    .await
    .unwrap();
    let group = FightGroup {
        hero_list: vec![42, 43, -1],
        ..Default::default()
    };

    let trials = normalize_trial_requests(&group).unwrap();
    assert_eq!(
        gameserver::state::battle::trial::active_hero_count(&group, &trials),
        3
    );
    let cards = generate_initial_deck(&pool, 7, &group, 0).await.unwrap();
    assert_eq!(cards.card_group.len(), 6);
}

#[tokio::test]
async fn explicit_trial_and_battle_aid_placeholder_can_coexist() {
    init_config();
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    let group = FightGroup {
        hero_list: vec![-1],
        trial_hero_list: vec![TrialHero {
            trial_id: Some(2_241_001),
            pos: Some(2),
            ..Default::default()
        }],
        ..Default::default()
    };

    let team = build_attacker_team(&pool, 7, &group).await.unwrap();
    assert_eq!(team.entitys.len(), 1);
    assert_eq!(team.entitys[0].trial_id, Some(2_241_001));
    assert_eq!(team.entitys[0].uid, Some(-2));
    assert_ne!(team.entitys[0].uid, Some(-1));
    assert_eq!(team.entitys[0].position, Some(2));
    let cards = generate_initial_deck(&pool, 7, &group, 0).await.unwrap();
    assert!(!cards.card_group.is_empty());
    assert!(
        cards
            .card_group
            .iter()
            .all(|card| card.hero_id == Some(3041))
    );
}

#[tokio::test]
async fn explicit_main_aid_placeholder_consumes_its_wire_slot() {
    init_config();
    let pool = owned_hero_pool().await;
    let group = FightGroup {
        hero_list: vec![-1, 42],
        trial_hero_list: vec![TrialHero {
            trial_id: Some(2_241_001),
            pos: Some(3),
            ..Default::default()
        }],
        ..Default::default()
    };

    let team = build_attacker_team(&pool, 7, &group).await.unwrap();
    assert_eq!(team.entitys.len(), 2);
    let owned = team
        .entitys
        .iter()
        .find(|entity| entity.uid == Some(42))
        .unwrap();
    let trial = team
        .entitys
        .iter()
        .find(|entity| entity.trial_id == Some(2_241_001))
        .unwrap();
    assert_eq!(owned.position, Some(2));
    assert_eq!(trial.position, Some(3));
}

#[tokio::test]
async fn explicit_substitute_aid_placeholder_consumes_its_wire_slot() {
    init_config();
    let pool = owned_hero_pool().await;
    let group = FightGroup {
        sub_hero_list: vec![-1, 42],
        trial_hero_list: vec![TrialHero {
            trial_id: Some(2_241_001),
            pos: Some(-2),
            ..Default::default()
        }],
        ..Default::default()
    };

    let team = build_attacker_team(&pool, 7, &group).await.unwrap();
    assert_eq!(team.sub_entitys.len(), 2);
    assert_eq!(team.sub_entitys[0].trial_id, Some(2_241_001));
    assert_eq!(team.sub_entitys[1].uid, Some(-3));
}

#[tokio::test]
async fn battle_aid_prefix_is_reserved_for_owned_substitute_uid() {
    init_config();
    let pool = owned_hero_pool().await;
    let group = FightGroup {
        hero_list: vec![-4],
        sub_hero_list: vec![42],
        ..Default::default()
    };

    let (team, modifiers) = build_attacker_team_with_destiny_modifiers(&pool, 7, &group)
        .await
        .unwrap();
    assert!(team.entitys.is_empty());
    assert_eq!(team.sub_entitys.len(), 1);
    assert_eq!(team.sub_entitys[0].uid, Some(-5));
    assert!(modifiers.contains_key(&-5));
    assert!(!modifiers.contains_key(&-4));
}

#[tokio::test]
async fn explicit_trial_list_rejects_missing_trial_id() {
    init_config();
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    let group = FightGroup {
        trial_hero_list: vec![TrialHero::default()],
        ..Default::default()
    };

    assert!(build_attacker_team(&pool, 7, &group).await.is_err());
    assert!(generate_initial_deck(&pool, 7, &group, 0).await.is_err());
}

#[tokio::test]
async fn legacy_trial_id_in_main_hero_list_builds_by_real_id() {
    init_config();
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    let group = FightGroup {
        hero_list: vec![-2_241_001],
        ..Default::default()
    };

    let team = build_attacker_team(&pool, 7, &group).await.unwrap();
    assert_eq!(team.entitys.len(), 1);
    assert!(team.sub_entitys.is_empty());
    assert_eq!(team.entitys[0].trial_id, Some(2_241_001));
    assert_eq!(team.entitys[0].uid, Some(-1));
}

#[tokio::test]
async fn legacy_trial_id_in_sub_hero_list_builds_substitute_without_main_card() {
    init_config();
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    let group = FightGroup {
        sub_hero_list: vec![-2_241_001],
        ..Default::default()
    };

    let team = build_attacker_team(&pool, 7, &group).await.unwrap();
    assert!(team.entitys.is_empty());
    assert_eq!(team.sub_entitys.len(), 1);
    assert_eq!(team.sub_entitys[0].trial_id, Some(2_241_001));
    assert_eq!(team.sub_entitys[0].uid, Some(-1));

    let cards = generate_initial_deck(&pool, 7, &group, 0).await.unwrap();
    assert!(cards.card_group.is_empty());
}

#[tokio::test]
async fn explicit_negative_trial_position_builds_substitute_without_main_card() {
    init_config();
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    let group = FightGroup {
        trial_hero_list: vec![TrialHero {
            trial_id: Some(2_241_001),
            pos: Some(-1),
            ..Default::default()
        }],
        ..Default::default()
    };

    let team = build_attacker_team(&pool, 7, &group).await.unwrap();
    assert!(team.entitys.is_empty());
    assert_eq!(team.sub_entitys.len(), 1);
    assert_eq!(team.sub_entitys[0].trial_id, Some(2_241_001));
    assert_eq!(team.sub_entitys[0].uid, Some(-1));

    let cards = generate_initial_deck(&pool, 7, &group, 0).await.unwrap();
    assert!(cards.card_group.is_empty());
}

#[tokio::test]
async fn explicit_trial_without_position_is_rejected() {
    init_config();
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    let trial_id = config::configs::get()
        .hero_trial
        .all()
        .first()
        .expect("trial fixture should exist")
        .id;
    let group = FightGroup {
        trial_hero_list: vec![TrialHero {
            trial_id: Some(trial_id),
            ..Default::default()
        }],
        ..Default::default()
    };

    assert!(build_attacker_team(&pool, 7, &group).await.is_err());
    assert!(generate_initial_deck(&pool, 7, &group, 0).await.is_err());
}

#[tokio::test]
async fn explicit_trial_slot_one_places_compacted_owned_hero_in_slot_two() {
    let pool = owned_hero_pool().await;
    let group = FightGroup {
        hero_list: vec![42],
        trial_hero_list: vec![TrialHero {
            trial_id: Some(2_241_001),
            pos: Some(1),
            ..Default::default()
        }],
        ..Default::default()
    };

    let team = build_attacker_team(&pool, 7, &group).await.unwrap();
    let owned = team
        .entitys
        .iter()
        .find(|entity| entity.uid == Some(42))
        .unwrap();
    let trial = team
        .entitys
        .iter()
        .find(|entity| entity.trial_id == Some(2_241_001))
        .unwrap();
    assert_eq!(trial.position, Some(1));
    assert_eq!(owned.position, Some(2));
    assert_eq!(team.entitys[0].trial_id, Some(2_241_001));
}

#[tokio::test]
async fn explicit_trial_slot_two_keeps_compacted_owned_hero_in_slot_one() {
    let pool = owned_hero_pool().await;
    let group = FightGroup {
        hero_list: vec![42],
        trial_hero_list: vec![TrialHero {
            trial_id: Some(2_241_001),
            pos: Some(2),
            ..Default::default()
        }],
        ..Default::default()
    };

    let team = build_attacker_team(&pool, 7, &group).await.unwrap();
    let owned = team
        .entitys
        .iter()
        .find(|entity| entity.uid == Some(42))
        .unwrap();
    let trial = team
        .entitys
        .iter()
        .find(|entity| entity.trial_id == Some(2_241_001))
        .unwrap();
    assert_eq!(owned.position, Some(1));
    assert_eq!(trial.position, Some(2));
}

#[tokio::test]
async fn explicit_substitute_position_preserves_compacted_slot_order() {
    let pool = owned_hero_pool().await;
    for (trial_position, trial_first) in [(-1, true), (-2, false)] {
        let group = FightGroup {
            sub_hero_list: vec![42],
            trial_hero_list: vec![TrialHero {
                trial_id: Some(2_241_001),
                pos: Some(trial_position),
                ..Default::default()
            }],
            ..Default::default()
        };

        let (team, modifiers) = build_attacker_team_with_destiny_modifiers(&pool, 7, &group)
            .await
            .unwrap();
        assert_eq!(team.sub_entitys.len(), 2);
        assert_eq!(team.sub_entitys[0].trial_id == Some(2_241_001), trial_first);
        let trial = team
            .sub_entitys
            .iter()
            .find(|entity| entity.trial_id == Some(2_241_001))
            .unwrap();
        let owned = team
            .sub_entitys
            .iter()
            .find(|entity| entity.trial_id != Some(2_241_001))
            .unwrap();
        assert_eq!(trial.uid, Some(-1));
        assert_eq!(owned.uid, Some(-2));
        let uids = team
            .sub_entitys
            .iter()
            .filter_map(|entity| entity.uid)
            .collect::<HashSet<_>>();
        assert_eq!(uids, HashSet::from([-1, -2]));
        assert_eq!(modifiers.len(), 2);
        assert!(modifiers.contains_key(&-1));
        assert!(modifiers.contains_key(&-2));
    }
}

#[tokio::test]
async fn legacy_substitute_position_preserves_inline_slot_order() {
    let pool = owned_hero_pool().await;
    for (sub_hero_list, trial_first) in
        [(vec![-2_241_001, 42], true), (vec![42, -2_241_001], false)]
    {
        let group = FightGroup {
            sub_hero_list,
            ..Default::default()
        };

        let (team, modifiers) = build_attacker_team_with_destiny_modifiers(&pool, 7, &group)
            .await
            .unwrap();
        assert_eq!(team.sub_entitys.len(), 2);
        assert_eq!(team.sub_entitys[0].trial_id == Some(2_241_001), trial_first);
        let trial = team
            .sub_entitys
            .iter()
            .find(|entity| entity.trial_id == Some(2_241_001))
            .unwrap();
        let owned = team
            .sub_entitys
            .iter()
            .find(|entity| entity.trial_id != Some(2_241_001))
            .unwrap();
        assert_eq!(trial.uid, Some(-1));
        assert_eq!(owned.uid, Some(-2));
        let uids = team
            .sub_entitys
            .iter()
            .filter_map(|entity| entity.uid)
            .collect::<HashSet<_>>();
        assert_eq!(uids, HashSet::from([-1, -2]));
        assert_eq!(modifiers.len(), 2);
        assert!(modifiers.contains_key(&-1));
        assert!(modifiers.contains_key(&-2));
    }
}

#[tokio::test]
async fn legacy_main_trial_position_preserves_inline_slot_order() {
    let pool = owned_hero_pool().await;
    for (hero_list, trial_first) in [(vec![-2_241_001, 42], true), (vec![42, -2_241_001], false)] {
        let group = FightGroup {
            hero_list,
            ..Default::default()
        };

        let team = build_attacker_team(&pool, 7, &group).await.unwrap();
        assert_eq!(team.entitys.len(), 2);
        assert_eq!(team.entitys[0].trial_id == Some(2_241_001), trial_first);
    }
}

#[tokio::test]
async fn explicit_and_legacy_trial_mix_is_rejected_by_builder_and_deck() {
    init_config();
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    let group = FightGroup {
        hero_list: vec![-2_241_001],
        trial_hero_list: vec![TrialHero {
            trial_id: Some(2_241_001),
            pos: Some(1),
            ..Default::default()
        }],
        ..Default::default()
    };

    assert!(build_attacker_team(&pool, 7, &group).await.is_err());
    assert!(generate_initial_deck(&pool, 7, &group, 0).await.is_err());
}

#[tokio::test]
async fn legacy_aid_and_zero_placeholders_preserve_main_and_substitute_slots() {
    let pool = owned_hero_pool().await;

    for (hero_list, trial_position, owned_position, trial_first) in [
        (vec![-4, 0, -2_241_001, 42], 3, 4, true),
        (vec![-4, 0, 42, -2_241_001], 4, 3, false),
    ] {
        let group = FightGroup {
            hero_list,
            ..Default::default()
        };
        let team = build_attacker_team(&pool, 7, &group).await.unwrap();
        let trial = team
            .entitys
            .iter()
            .find(|entity| entity.trial_id == Some(2_241_001))
            .unwrap();
        let owned = team
            .entitys
            .iter()
            .find(|entity| entity.uid == Some(42))
            .unwrap();
        assert_eq!(team.entitys[0].trial_id == Some(2_241_001), trial_first);
        assert_eq!(trial.uid, Some(-5));
        assert_eq!(trial.position, Some(trial_position));
        assert_eq!(owned.position, Some(owned_position));
    }

    for (sub_hero_list, trial_first) in [
        (vec![-4, 0, -2_241_001, 42], true),
        (vec![-4, 0, 42, -2_241_001], false),
    ] {
        let group = FightGroup {
            sub_hero_list,
            ..Default::default()
        };
        let (team, modifiers) = build_attacker_team_with_destiny_modifiers(&pool, 7, &group)
            .await
            .unwrap();
        assert_eq!(team.sub_entitys.len(), 2);
        assert_eq!(team.sub_entitys[0].trial_id == Some(2_241_001), trial_first);
        let trial = team
            .sub_entitys
            .iter()
            .find(|entity| entity.trial_id == Some(2_241_001))
            .unwrap();
        let owned = team
            .sub_entitys
            .iter()
            .find(|entity| entity.trial_id != Some(2_241_001))
            .unwrap();
        assert_eq!(trial.uid, Some(-5));
        assert_eq!(owned.uid, Some(-6));
        assert!(modifiers.contains_key(&-5));
        assert!(modifiers.contains_key(&-6));
    }
}

#[tokio::test]
async fn duplicate_explicit_signed_positions_are_rejected_everywhere() {
    init_config();
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    let trial_ids = config::configs::get()
        .hero_trial
        .all()
        .iter()
        .map(|trial| trial.id)
        .take(2)
        .collect::<Vec<_>>();
    assert_eq!(trial_ids.len(), 2);

    for position in [1, -1] {
        let group = FightGroup {
            trial_hero_list: trial_ids
                .iter()
                .map(|trial_id| TrialHero {
                    trial_id: Some(*trial_id),
                    pos: Some(position),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };

        assert!(build_attacker_team(&pool, 7, &group).await.is_err());
        assert!(generate_initial_deck(&pool, 7, &group, 0).await.is_err());
    }
}

#[tokio::test]
async fn explicit_zero_placeholders_preserve_main_and_substitute_slots() {
    let pool = owned_hero_pool().await;
    let main_group = FightGroup {
        hero_list: vec![0, 42],
        trial_hero_list: vec![TrialHero {
            trial_id: Some(2_241_001),
            pos: Some(1),
            ..Default::default()
        }],
        ..Default::default()
    };
    let main_team = build_attacker_team(&pool, 7, &main_group).await.unwrap();
    let owned = main_team
        .entitys
        .iter()
        .find(|entity| entity.uid == Some(42))
        .unwrap();
    assert_eq!(owned.position, Some(3));

    let sub_group = FightGroup {
        sub_hero_list: vec![0, 42],
        trial_hero_list: vec![TrialHero {
            trial_id: Some(2_241_001),
            pos: Some(-2),
            ..Default::default()
        }],
        ..Default::default()
    };
    let sub_team = build_attacker_team(&pool, 7, &sub_group).await.unwrap();
    assert_eq!(sub_team.sub_entitys.len(), 2);
    assert_eq!(sub_team.sub_entitys[0].trial_id, Some(2_241_001));
    assert_eq!(sub_team.sub_entitys[1].trial_id, Some(0));
    assert_eq!(sub_team.sub_entitys[1].model_id, Some(3025));
}

#[tokio::test]
async fn minimum_signed_trial_position_is_rejected_everywhere() {
    init_config();
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    let group = FightGroup {
        trial_hero_list: vec![TrialHero {
            trial_id: Some(2_241_001),
            pos: Some(i32::MIN),
            ..Default::default()
        }],
        ..Default::default()
    };

    assert!(normalize_trial_requests(&group).is_err());
    assert!(generate_initial_deck(&pool, 7, &group, 0).await.is_err());
    assert!(build_attacker_team(&pool, 7, &group).await.is_err());
}
