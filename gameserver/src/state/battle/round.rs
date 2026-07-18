use anyhow::Result;
use sonettobuf::{ActEffect, CardInfo, Fight, FightEntityInfo};
use std::collections::HashMap;

use crate::state::battle::manager::buff_mgr::BuffMgr;
use crate::state::battle::destiny::DestinyModifierMap;
use crate::state::battle::manager::calculate_mgr::hero_sp_attribute_from_destiny;

#[allow(dead_code)]
pub struct RoundState {
    pub entities: HashMap<i64, FightEntityInfo>,
    pub buff_mgr: BuffMgr,
    pub act_point: i32,
    pub power: i32,
    pub player_deck: Vec<CardInfo>,
    pub ai_cards: Vec<CardInfo>,
    pub used_cards: Vec<i32>,
    pub round_num: i32,
    pub move_num: i32,
    pub is_finish: bool,
    pub pending_effects: Vec<ActEffect>,
    pub destiny_modifiers: DestinyModifierMap,
}

#[allow(dead_code)]
impl RoundState {
    pub fn new(fight: &Fight) -> Result<Self> {
        Self::new_with_destiny_modifiers(fight, DestinyModifierMap::new())
    }

    pub fn new_with_destiny_modifiers(
        fight: &Fight,
        destiny_modifiers: DestinyModifierMap,
    ) -> Result<Self> {
        let mut entities = HashMap::new();

        if let Some(attacker) = &fight.attacker {
            for e in attacker.entitys.iter().chain(attacker.sub_entitys.iter()) {
                if let Some(uid) = e.uid {
                    entities.insert(uid, e.clone());
                }
            }
        }

        if let Some(defender) = &fight.defender {
            for e in &defender.entitys {
                if let Some(uid) = e.uid {
                    entities.insert(uid, e.clone());
                }
            }
        }

        Ok(Self {
            entities,
            buff_mgr: BuffMgr::new(),
            act_point: 4,
            power: 15,
            player_deck: vec![],
            ai_cards: vec![],
            used_cards: vec![],
            round_num: fight.cur_round.unwrap_or(1),
            move_num: 0,
            is_finish: fight.is_finish.unwrap_or(false),
            pending_effects: vec![],
            destiny_modifiers,
        })
    }

    pub fn get_entity(&self, uid: i64) -> Option<&FightEntityInfo> {
        self.entities.get(&uid)
    }

    pub fn get_entity_mut(&mut self, uid: i64) -> Option<&mut FightEntityInfo> {
        self.entities.get_mut(&uid)
    }

    pub fn iter_entities(&self) -> impl Iterator<Item = &FightEntityInfo> {
        self.entities.values()
    }

    pub fn iter_entities_mut(&mut self) -> impl Iterator<Item = &mut FightEntityInfo> {
        self.entities.values_mut()
    }

    pub fn snapshot_entities_map(&self) -> HashMap<i64, FightEntityInfo> {
        self.entities.clone()
    }

    pub fn build_ex_point_info(&self) -> Vec<sonettobuf::FightExPointInfo> {
        self.iter_entities()
            .map(|entity| {
                let ex_point_type = if entity.model_id == Some(3120) {
                    Some(1)
                } else {
                    entity.ex_point_type.or(Some(0))
                };

                sonettobuf::FightExPointInfo {
                    uid: entity.uid,
                    ex_point: entity.ex_point,
                    power_infos: entity.power_infos.clone(),
                    current_hp: entity.current_hp,
                    ex_point_type,
                }
            })
            .collect()
    }

    fn build_sp_attributes(&self) -> Vec<sonettobuf::FightHeroSpAttributeInfo> {
        self.iter_entities()
            .map(|e| sonettobuf::FightHeroSpAttributeInfo {
                uid: e.uid,
                attribute: Some(hero_sp_attribute_from_destiny(
                    e.uid.and_then(|uid| self.destiny_modifiers.get(&uid)),
                )),
            })
            .collect()
    }
}

pub struct RoundSnapshot {
    pub act_point: i32,
    pub power: i32,
    pub player_deck: Vec<CardInfo>,
    pub ai_cards: Vec<CardInfo>,
    pub used_cards: Vec<i32>,
    pub round_num: i32,
    pub move_num: i32,
    pub is_finish: bool,
    pub ex_point_info: Vec<sonettobuf::FightExPointInfo>,
    pub hero_sp_attributes: Vec<sonettobuf::FightHeroSpAttributeInfo>,
}

impl RoundState {
    pub fn export_snapshot(&self) -> RoundSnapshot {
        RoundSnapshot {
            act_point: self.act_point,
            power: self.power,
            player_deck: self.player_deck.clone(),
            ai_cards: self.ai_cards.clone(),
            used_cards: self.used_cards.clone(),
            round_num: self.round_num,
            move_num: self.move_num,
            is_finish: self.is_finish,
            ex_point_info: self.build_ex_point_info(),
            hero_sp_attributes: self.build_sp_attributes(),
        }
    }
}
