use std::sync::Once;

use gameserver::state::battle::destiny::{
    DestinyState, HeroBuildContext, HeroSource, SkillExchange, parse_skill_exchanges,
    resolve_hero_kit,
};

static CONFIG_INIT: Once = Once::new();

fn init_config() {
    CONFIG_INIT.call_once(|| {
        let data_dir = std::env::var("JSON_DATA_DIR").expect(
            "JSON_DATA_DIR must point at the international 3.6 runtime excel2json directory",
        );
        config::configs::init(&data_dir).expect("failed to initialize config data");
    });
}

fn resolve(
    hero_id: i32,
    ex_skill_level: i32,
    destiny_rank: i32,
    facet_id: i32,
) -> gameserver::state::battle::destiny::ResolvedHeroKit {
    init_config();
    let game = config::configs::get();
    let index = config::destiny::DestinyConfigIndex::try_from_game_db(game).unwrap();
    let character = game.character.get(hero_id).unwrap();
    resolve_with_rank_skin(
        &index,
        game,
        character.skin_id,
        3,
        hero_id,
        ex_skill_level,
        destiny_rank,
        facet_id,
    )
}

fn resolve_with_rank_skin(
    index: &config::destiny::DestinyConfigIndex,
    game: &config::GameDB,
    skin: i32,
    rank: i32,
    hero_id: i32,
    ex_skill_level: i32,
    destiny_rank: i32,
    facet_id: i32,
) -> gameserver::state::battle::destiny::ResolvedHeroKit {
    let character = game.character.get(hero_id).unwrap();
    resolve_hero_kit(
        index,
        game,
        &HeroBuildContext {
            hero_id,
            skin,
            rank,
            ex_skill_level,
            destiny: DestinyState {
                rank: destiny_rank,
                level: if destiny_rank == 0 { 0 } else { 1 },
                facet_id,
            },
            is_substitute: false,
            hero_type: character.hero_type,
            source: HeroSource::Owned,
        },
    )
    .unwrap()
}

#[test]
fn normal_sparse_ex_rows_use_last_nonempty_value() {
    let kit = resolve(3003, 5, 0, 0);
    assert_eq!(kit.skill_group_1, vec![30030114, 30030115, 30030116]);
    assert_eq!(kit.skill_group_2, vec![30030127, 30030128, 30030129]);
}

#[test]
fn ultimate_uses_latest_nonzero_row_not_level_one() {
    let kit = resolve(3098, 5, 0, 0);
    assert_eq!(kit.ultimate, 30980133);
}

#[test]
fn rank_replacement_uses_skin_rank_and_replacement_hero_type() {
    let kit = resolve(3120, 0, 0, 0);
    assert_eq!(kit.skill_group_1, vec![31200111, 31200112, 31200113]);
    assert_eq!(kit.skill_group_2, vec![31200151, 31200152, 31200153]);
    assert_eq!(kit.ultimate, 31200131);

    let ex_level_one = resolve(3120, 1, 0, 0);
    assert_eq!(ex_level_one.skill_group_1, vec![31200114, 31200115, 31200116]);
    assert_eq!(ex_level_one.skill_group_2, vec![31200154, 31200155, 31200156]);
    assert_eq!(ex_level_one.ultimate, 31200131);
}

#[test]
fn rank_replacement_requires_threshold_rank_and_matching_skin() {
    init_config();
    let game = config::configs::get();
    let index = config::destiny::DestinyConfigIndex::try_from_game_db(game).unwrap();

    let low_rank = resolve_with_rank_skin(&index, game, 312001, 2, 3120, 0, 0, 0);
    assert_eq!(low_rank.skill_group_1, vec![31200201, 31200202, 31200203]);
    assert_eq!(low_rank.skill_group_2, vec![31200211, 31200212, 31200213]);
    assert_eq!(low_rank.ultimate, 0);

    let wrong_skin = resolve_with_rank_skin(&index, game, 312099, 3, 3120, 0, 0, 0);
    assert_eq!(wrong_skin.skill_group_1, low_rank.skill_group_1);
    assert_eq!(wrong_skin.skill_group_2, low_rank.skill_group_2);
    assert_eq!(wrong_skin.ultimate, low_rank.ultimate);
}

#[test]
fn reshape_replaces_normal_ex_source_only_when_active() {
    let inactive = resolve(3007, 5, 3, 300701);
    assert_eq!(inactive.skill_group_1, vec![30070214, 30070215, 30070216]);
    assert_eq!(inactive.ultimate, 30070234);

    let active = resolve(3007, 5, 4, 300701);
    assert_eq!(active.skill_group_1, vec![30072211, 30072212, 30072213]);
    assert_eq!(active.skill_group_2, vec![30074221, 30074222, 30074223]);
    assert_eq!(active.ultimate, 30075231);
}

#[test]
fn facet_exchange_preserves_declared_chain_order() {
    let exchanges = parse_skill_exchanges("1#2|2#3|1#4").unwrap();
    assert_eq!(
        exchanges,
        vec![
            SkillExchange {
                source: 1,
                target: 2
            },
            SkillExchange {
                source: 2,
                target: 3
            },
            SkillExchange {
                source: 1,
                target: 4
            },
        ]
    );
}

#[test]
fn exchange_parser_ignores_trailing_empty_segment() {
    assert_eq!(
        parse_skill_exchanges("1#2|").unwrap(),
        vec![SkillExchange {
            source: 1,
            target: 2,
        }]
    );
}

#[test]
fn exchange_parser_rejects_unconfirmed_empty_segments() {
    let malformed = ["|1#2", "1#2||3#4", "1#2||"];
    let accepted = malformed
        .into_iter()
        .filter(|value| parse_skill_exchanges(value).is_ok())
        .collect::<Vec<_>>();
    assert!(accepted.is_empty(), "unexpectedly accepted {accepted:?}");
}

#[test]
fn exchange_parser_rejects_non_positive_skill_ids() {
    let invalid = ["-1#5", "0#5", "1#0", "1#-5"];
    let accepted = invalid
        .into_iter()
        .filter(|value| parse_skill_exchanges(value).is_ok())
        .collect::<Vec<_>>();
    assert!(accepted.is_empty(), "unexpectedly accepted {accepted:?}");
}

#[test]
fn duplicate_facet_sources_use_each_declared_match_without_cascading() {
    let kit = resolve(3087, 0, 4, 308701);
    assert_eq!(kit.passives, vec![30870341, 30870151, 30870161]);
    assert_eq!(
        kit.trace
            .iter()
            .filter(|entry| entry.detail == "passive 30870141 -> 30870341")
            .count(),
        2
    );
}

#[test]
fn zero_facet_is_inactive_and_traced() {
    let kit = resolve(3003, 0, 0, 0);
    let trace = kit
        .trace
        .iter()
        .map(|entry| (entry.source_key.as_str(), entry.detail.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        trace,
        vec![("character_destiny_facets:0:0", "facet 0 inactive")]
    );
}

#[test]
fn passives_apply_normal_then_facet_then_reshape_order() {
    let kit = resolve(3053, 4, 4, 305301);
    assert_eq!(kit.passives, vec![30530156, 30530142, 30530157]);
    let transitions = kit
        .trace
        .iter()
        .filter(|entry| entry.detail.contains("passive"))
        .map(|entry| entry.detail.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        transitions,
        vec![
            "passive 30530141 -> 30530144",
            "passive 30530144 -> 30530190",
            "passive 30530143 -> 30530151",
            "passive 30530190 -> 30530152",
            "passive 30530152 -> 30530154",
            "passive 30530154 -> 30530156",
            "passive 30530151 -> 30530157",
        ]
    );
}

#[test]
fn foreign_facet_is_inactive_without_error() {
    let base = resolve(3003, 5, 0, 0);
    let foreign = resolve(3003, 5, 4, 300701);
    assert_eq!(foreign.skill_group_1, base.skill_group_1);
    assert_eq!(foreign.skill_group_2, base.skill_group_2);
    assert_eq!(foreign.ultimate, base.ultimate);
    assert_eq!(foreign.passives, base.passives);
    assert!(
        foreign
            .trace
            .iter()
            .any(|entry| entry.detail.contains("foreign facet 300701 inactive"))
    );
}

#[test]
fn kit_trace_records_each_source_to_target_transition() {
    let kit = resolve(3081, 5, 4, 308101);
    assert_eq!(kit.skill_group_1, vec![30810436, 30810437, 30810438]);
    assert_eq!(kit.skill_group_2, vec![30810381, 30810382, 30810383]);
    assert_eq!(kit.ultimate, 30810413);
    let transitions = kit
        .trace
        .iter()
        .map(|entry| (entry.source_key.as_str(), entry.detail.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        transitions,
        vec![
            (
                "character_destiny_facets:308101:4",
                "facet 308101 active at rank 4",
            ),
            (
                "destiny_facets_ex_level:308101:5",
                "skill_group_1 30810111 -> 30810436",
            ),
            (
                "destiny_facets_ex_level:308101:5",
                "skill_group_1 30810112 -> 30810437",
            ),
            (
                "destiny_facets_ex_level:308101:5",
                "skill_group_1 30810113 -> 30810438",
            ),
            (
                "destiny_facets_ex_level:308101:5",
                "skill_group_2 30810121 -> 30810381",
            ),
            (
                "destiny_facets_ex_level:308101:5",
                "skill_group_2 30810122 -> 30810382",
            ),
            (
                "destiny_facets_ex_level:308101:5",
                "skill_group_2 30810123 -> 30810383",
            ),
            (
                "destiny_facets_ex_level:308101:5",
                "ultimate 30810131 -> 30810413",
            ),
            ("skill_ex_level:3081:5", "passive 30810141 -> 30810145",),
            ("skill_ex_level:3081:5", "passive 30810161 -> 30810165",),
            (
                "character_destiny_facets:308101:4",
                "passive 30810145 -> 30810315",
            ),
            (
                "character_destiny_facets:308101:4",
                "passive 30810165 -> 30810325",
            ),
            (
                "destiny_facets_ex_level:308101:5",
                "passive 30810315 -> 30810356",
            ),
            (
                "destiny_facets_ex_level:308101:5",
                "passive 30810325 -> 30810366",
            ),
        ]
    );
}
