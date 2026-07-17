use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use config::destiny::DestinyConfigIndex;

fn runtime_data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../sonetto-data/versions/international-3.6-destiny-runtime/excel2json")
}

struct TempRuntime(PathBuf);

impl TempRuntime {
    fn copy_without(source: &Path, missing_name: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("sonetto-config-{}-{unique}", std::process::id()));
        fs::create_dir(&root).unwrap();
        for table in include_str!("../configs/mod.rs")
            .lines()
            .filter_map(|line| {
                line.strip_prefix("pub mod ")
                    .and_then(|value| value.strip_suffix(';'))
            })
        {
            let file_name = format!("{table}.json");
            if file_name == missing_name {
                continue;
            }
            fs::copy(source.join(&file_name), root.join(file_name)).unwrap();
        }
        Self(root)
    }

    fn copy_all(source: &Path) -> Self {
        Self::copy_without(source, "")
    }
}

impl Drop for TempRuntime {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn loads_complete_international_36_destiny_tables() {
    let path = runtime_data_dir();
    let db = config::GameDB::load(path.to_str().unwrap()).unwrap();

    assert_eq!(db.character_destiny.len(), 37);
    assert_eq!(db.character_destiny_slots.len(), 925);
    assert_eq!(db.character_destiny_facets.len(), 172);
    assert_eq!(db.character_destiny_facets_consume.len(), 44);
    assert_eq!(db.destiny_facets_ex_level.len(), 15);
    assert_eq!(db.skill_ex_level_destiny_facets.len(), 5);
    assert_eq!(db.character_attribute.len(), 61);
}

#[test]
fn each_new_required_table_missing_does_not_fall_back_to_generated_data() {
    let source = runtime_data_dir();
    for missing in [
        "character_attribute.json",
        "character_destiny_facets_consume.json",
        "character_destiny_slots.json",
        "destiny_facets_ex_level.json",
        "skill_ex_level_destiny_facets.json",
    ] {
        let copy = TempRuntime::copy_without(&source, missing);
        let error = match config::GameDB::load(copy.0.to_str().unwrap()) {
            Ok(_) => panic!("GameDB::load unexpectedly accepted missing {missing}"),
            Err(error) => error,
        };

        assert!(
            error.to_string().contains(missing),
            "unexpected load error for {missing}: {error:#}"
        );
    }
}

#[test]
fn indexes_destiny_configuration_by_owned_composite_keys() {
    let path = runtime_data_dir();
    let db = config::GameDB::load(path.to_str().unwrap()).unwrap();
    let index = DestinyConfigIndex::try_from_game_db(&db).unwrap();

    let hero = index.hero(3073).unwrap();
    assert_eq!(hero.hero_id, 3073);
    assert_eq!(hero.slots_id, 3073);
    assert_eq!(hero.facet_ids, vec![307301]);
    assert!(
        hero.facet_ids
            .iter()
            .all(|facets_id| index.facet(*facets_id, 1).is_some())
    );

    let slot = index.slot(3073, 1, 1).unwrap();
    assert_eq!(slot.effect, "601#100|602#20|603#10|604#10|211#100");

    let facet = index.facet(308101, 4).unwrap();
    assert!(facet.ex_level_exchange);
    assert_eq!(facet.desc, "language_10100268");

    let consume = index.facet_consume(308101).unwrap();
    assert_eq!(consume.tend, 5);

    let reshape = index.reshape(308101, 1).unwrap();
    let skill_facet = index.skill_destiny_facet(308101, 1).unwrap();
    assert_eq!(reshape.skill_ex, 30810411);
    assert_eq!(skill_facet.skill_ex, 30810132);
    assert_ne!(reshape.desc, skill_facet.desc);

    let attribute = index.attribute(605).unwrap();
    assert_eq!(attribute.attr_type, "hp_percent");
    assert_eq!(attribute.show_type, 1);
}

#[test]
fn rejects_duplicate_composite_keys_instead_of_overwriting() {
    let source = runtime_data_dir();
    let copy = TempRuntime::copy_all(&source);
    let slots_path = copy.0.join("character_destiny_slots.json");
    let mut value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&slots_path).unwrap()).unwrap();
    let rows = value[1].as_array_mut().unwrap();
    rows.push(rows[0].clone());
    fs::write(&slots_path, serde_json::to_vec(&value).unwrap()).unwrap();
    let db = config::GameDB::load(copy.0.to_str().unwrap()).unwrap();

    let error = match DestinyConfigIndex::try_from_game_db(&db) {
        Ok(_) => panic!("DestinyConfigIndex accepted a duplicate slot key"),
        Err(error) => error,
    };

    assert!(
        error
            .to_string()
            .contains("duplicate character_destiny_slots key (3052, 1, 1)"),
        "unexpected index error: {error:#}"
    );
}
