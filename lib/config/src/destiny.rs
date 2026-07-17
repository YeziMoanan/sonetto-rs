use std::{
    collections::{HashMap, hash_map::Entry},
    fmt::Debug,
    hash::Hash,
};

use anyhow::{Context, Result, bail};

use crate::{
    GameDB, character_attribute::CharacterAttribute,
    character_destiny_facets::CharacterDestinyFacets,
    character_destiny_facets_consume::CharacterDestinyFacetsConsume,
    character_destiny_slots::CharacterDestinySlots, destiny_facets_ex_level::DestinyFacetsExLevel,
    skill_ex_level_destiny_facets::SkillExLevelDestinyFacets,
};

pub struct DestinyConfigIndex {
    heroes: HashMap<i32, HeroDestinyConfig>,
    slots: HashMap<(i32, i32, i32), CharacterDestinySlots>,
    facets: HashMap<(i32, i32), CharacterDestinyFacets>,
    consumes: HashMap<i32, CharacterDestinyFacetsConsume>,
    reshape: HashMap<(i32, i32), DestinyFacetsExLevel>,
    facet_skills: HashMap<(i32, i32), SkillExLevelDestinyFacets>,
    attributes: HashMap<i32, CharacterAttribute>,
}

pub struct HeroDestinyConfig {
    pub hero_id: i32,
    pub slots_id: i32,
    pub facet_ids: Vec<i32>,
}

impl DestinyConfigIndex {
    pub fn try_from_game_db(db: &GameDB) -> Result<Self> {
        let mut heroes = HashMap::with_capacity(db.character_destiny.len());
        for record in db.character_destiny.iter() {
            let facet_ids = record
                .facets_id
                .split('#')
                .map(|value| {
                    value.parse::<i32>().with_context(|| {
                        format!(
                            "invalid character_destiny facets_id {:?} for hero {}",
                            record.facets_id, record.hero_id
                        )
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            insert_unique(
                &mut heroes,
                record.hero_id,
                HeroDestinyConfig {
                    hero_id: record.hero_id,
                    slots_id: record.slots_id,
                    facet_ids,
                },
                "character_destiny",
            )?;
        }

        let mut slots = HashMap::with_capacity(db.character_destiny_slots.len());
        for record in db.character_destiny_slots.iter() {
            insert_unique(
                &mut slots,
                (record.slots_id, record.stage, record.node),
                record.clone(),
                "character_destiny_slots",
            )?;
        }

        let mut facets = HashMap::with_capacity(db.character_destiny_facets.len());
        for record in db.character_destiny_facets.iter() {
            insert_unique(
                &mut facets,
                (record.facets_id, record.level),
                record.clone(),
                "character_destiny_facets",
            )?;
        }

        let mut consumes = HashMap::with_capacity(db.character_destiny_facets_consume.len());
        for record in db.character_destiny_facets_consume.iter() {
            insert_unique(
                &mut consumes,
                record.facets_id,
                record.clone(),
                "character_destiny_facets_consume",
            )?;
        }

        let mut reshape = HashMap::with_capacity(db.destiny_facets_ex_level.len());
        for record in db.destiny_facets_ex_level.iter() {
            insert_unique(
                &mut reshape,
                (record.hero_id, record.skill_level),
                record.clone(),
                "destiny_facets_ex_level",
            )?;
        }

        let mut facet_skills = HashMap::with_capacity(db.skill_ex_level_destiny_facets.len());
        for record in db.skill_ex_level_destiny_facets.iter() {
            insert_unique(
                &mut facet_skills,
                (record.facets_id, record.skill_level),
                record.clone(),
                "skill_ex_level_destiny_facets",
            )?;
        }

        let mut attributes = HashMap::with_capacity(db.character_attribute.len());
        for record in db.character_attribute.iter() {
            insert_unique(
                &mut attributes,
                record.id,
                record.clone(),
                "character_attribute",
            )?;
        }

        Ok(Self {
            heroes,
            slots,
            facets,
            consumes,
            reshape,
            facet_skills,
            attributes,
        })
    }

    pub fn hero(&self, hero_id: i32) -> Option<&HeroDestinyConfig> {
        self.heroes.get(&hero_id)
    }

    pub fn slot(&self, slots_id: i32, stage: i32, node: i32) -> Option<&CharacterDestinySlots> {
        self.slots.get(&(slots_id, stage, node))
    }

    pub fn facet(&self, facets_id: i32, level: i32) -> Option<&CharacterDestinyFacets> {
        self.facets.get(&(facets_id, level))
    }

    pub fn facet_consume(&self, facets_id: i32) -> Option<&CharacterDestinyFacetsConsume> {
        self.consumes.get(&facets_id)
    }

    pub fn reshape(&self, facets_id: i32, skill_level: i32) -> Option<&DestinyFacetsExLevel> {
        self.reshape.get(&(facets_id, skill_level))
    }

    pub fn skill_destiny_facet(
        &self,
        facets_id: i32,
        skill_level: i32,
    ) -> Option<&SkillExLevelDestinyFacets> {
        self.facet_skills.get(&(facets_id, skill_level))
    }

    pub fn attribute(&self, id: i32) -> Option<&CharacterAttribute> {
        self.attributes.get(&id)
    }
}

fn insert_unique<K, V>(map: &mut HashMap<K, V>, key: K, value: V, table: &str) -> Result<()>
where
    K: Debug + Eq + Hash,
{
    match map.entry(key) {
        Entry::Vacant(entry) => {
            entry.insert(value);
            Ok(())
        }
        Entry::Occupied(entry) => {
            bail!("duplicate {table} key {:?}", entry.key())
        }
    }
}
