use std::sync::Once;

use gameserver::state::battle::destiny::{
    DestinyState, FixedTenths, HeroBaseAttributes, resolve_destiny_attributes,
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

fn index() -> config::destiny::DestinyConfigIndex {
    init_config();
    config::destiny::DestinyConfigIndex::try_from_game_db(config::configs::get())
        .expect("Destiny config index should build")
}

fn base() -> HeroBaseAttributes {
    HeroBaseAttributes {
        hp: 10_000,
        attack: 1_000,
        defense: 500,
        mdefense: 400,
    }
}

#[test]
fn fixed_tenths_floor_percent_smoke() {
    let _state = DestinyState {
        rank: 1,
        level: 1,
        facet_id: 0,
    };
    let _base = base();

    assert_eq!(FixedTenths(45).floor_percent_of(1000).unwrap(), 45);
}

#[test]
fn zero_state_adds_no_destiny_attributes() {
    let resolved = resolve_destiny_attributes(
        &index(),
        3098,
        DestinyState {
            rank: 0,
            level: 0,
            facet_id: 0,
        },
        base(),
    )
    .unwrap();

    assert_eq!(resolved.hp, 0);
    assert_eq!(resolved.attack, 0);
    assert_eq!(resolved.defense, 0);
    assert_eq!(resolved.mdefense, 0);
    assert!(resolved.raw_tenths.is_empty());
    assert!(resolved.trace.is_empty());
}

#[test]
fn includes_prior_stages_and_current_prefix_only() {
    let resolved = resolve_destiny_attributes(
        &index(),
        3098,
        DestinyState {
            rank: 2,
            level: 2,
            facet_id: 0,
        },
        base(),
    )
    .unwrap();

    assert_eq!(resolved.hp, 270);
    assert_eq!(resolved.attack, 170);
    assert_eq!(resolved.defense, 17);
    assert_eq!(resolved.mdefense, 25);
    assert_eq!(resolved.raw_tenths.get(&606), Some(&135));
    assert_eq!(resolved.trace.len(), 7);
    assert!(
        resolved
            .trace
            .iter()
            .any(|entry| entry.source_key == "character_destiny_slots:3098:2:2")
    );
    assert!(
        !resolved
            .trace
            .iter()
            .any(|entry| entry.source_key == "character_destiny_slots:3098:2:3")
    );
}

#[test]
fn repeated_direct_and_percent_effects_aggregate_before_floor() {
    let resolved = resolve_destiny_attributes(
        &index(),
        3098,
        DestinyState {
            rank: 2,
            level: 1,
            facet_id: 0,
        },
        HeroBaseAttributes {
            hp: 1000,
            attack: 999,
            defense: 1000,
            mdefense: 1000,
        },
    )
    .unwrap();

    assert_eq!(resolved.raw_tenths.get(&606), Some(&135));
    assert_eq!(resolved.attack, 35 + 134);
}

#[test]
fn retains_all_special_combat_attributes() {
    let resolved = resolve_destiny_attributes(
        &index(),
        3051,
        DestinyState {
            rank: 2,
            level: 1,
            facet_id: 0,
        },
        base(),
    )
    .unwrap();

    assert_eq!(resolved.raw_tenths.get(&217), Some(&75));
    assert!(resolved.raw_tenths.contains_key(&601));
    assert!(resolved.raw_tenths.contains_key(&604));
}

#[test]
fn all_referenced_attribute_ids_have_runtime_adapters() {
    let allowed = [
        201, 203, 205, 211, 212, 214, 217, 218, 219, 220, 301, 601, 602, 603, 604, 605, 606, 607,
        608,
    ];
    for id in allowed {
        assert!(gameserver::state::battle::destiny::is_supported_destiny_attribute(id));
    }
}

#[test]
fn show_type_one_scales_raw_value_by_one_tenth() {
    assert_eq!(FixedTenths(45).floor_percent_of(999).unwrap(), 44);
}

#[test]
fn all_actual_node_attribute_ids_have_runtime_adapters() {
    init_config();
    let mut seen = std::collections::BTreeSet::new();
    for slot in config::configs::get().character_destiny_slots.iter() {
        for effect in slot.effect.split('|').filter(|effect| !effect.is_empty()) {
            let id = effect.split('#').next().unwrap().parse::<i32>().unwrap();
            seen.insert(id);
        }
    }

    for id in seen {
        assert!(
            gameserver::state::battle::destiny::is_supported_destiny_attribute(id),
            "missing Destiny runtime adapter for attribute {id}"
        );
    }
}
