use sonettobuf::{
    ActEffect, Fight, FightExPointInfo, FightHeroSpAttributeInfo, FightStep, HeroSpAttribute,
    PlayerSkillInfo, PowerInfo,
};
use std::sync::Arc;

use crate::state::battle::{
    destiny::{DestinyModifierMap, DestinyResolveError, ResolvedDestinyAttributes},
    effects::effect_types::EffectType,
    manager::{
        buff_mgr::BuffMgr,
        entity_mgr::{FightEntityDataMgr, get_entity_mut_by_location},
    },
    mechanics::bloodtithe::BloodtitheState,
};

/// Apply the absolute PowerInfo payload emitted by effect type 295.
///
/// The client upserts by power id and clamps the current value to the
/// declared maximum. The server accepts only complete absolute payloads;
/// partial-field client default/removal semantics are not assumed.
pub fn apply_power_info_change(
    entity: &mut sonettobuf::FightEntityInfo,
    incoming: &PowerInfo,
) -> Result<(), DestinyResolveError> {
    let power_id = incoming.power_id.ok_or_else(|| {
        DestinyResolveError::InvalidConfig("PowerInfoChange missing power_id".to_string())
    })?;
    // The wire payload is an absolute replacement. Rejecting an omitted
    // scalar is a server-side integrity guard; client default/removal
    // semantics are not assumed without a matching protocol fixture.
    let incoming_num = incoming.num.ok_or_else(|| {
        DestinyResolveError::InvalidConfig(format!(
            "PowerInfoChange missing num for power {power_id}"
        ))
    })?;
    let max = incoming.max.ok_or_else(|| {
        DestinyResolveError::InvalidConfig(format!(
            "PowerInfoChange missing max for power {power_id}"
        ))
    })?;
    if max < 0 {
        return Err(DestinyResolveError::InvalidConfig(format!(
            "PowerInfoChange max {max} is negative for power {power_id}"
        )));
    }
    let normalized = PowerInfo {
        power_id: Some(power_id),
        num: Some(incoming_num.clamp(0, max)),
        max: Some(max),
    };
    if let Some(slot) = entity
        .power_infos
        .iter_mut()
        .find(|power| power.power_id == Some(power_id))
    {
        *slot = normalized;
    } else {
        entity.power_infos.push(normalized);
    }
    Ok(())
}

fn raw_modifier(resolved: &ResolvedDestinyAttributes, id: i32) -> i32 {
    resolved
        .raw_tenths
        .get(&id)
        .copied()
        .and_then(|value| i32::try_from(value).ok())
        .unwrap_or(0)
}

pub fn hero_sp_attribute_from_destiny(
    resolved: Option<&ResolvedDestinyAttributes>,
) -> HeroSpAttribute {
    let empty = ResolvedDestinyAttributes::default();
    let resolved = resolved.unwrap_or(&empty);
    let mut attr = resolved.sp_attr;
    attr.revive.get_or_insert(0);
    attr.heal.get_or_insert(raw_modifier(resolved, 212));
    attr.absorb.get_or_insert(0);
    attr.defense_ignore.get_or_insert(0);
    attr.clutch.get_or_insert(raw_modifier(resolved, 211));
    attr.final_add_dmg.get_or_insert(0);
    attr.final_drop_dmg.get_or_insert(0);
    attr.normal_skill_rate
        .get_or_insert(raw_modifier(resolved, 214));
    attr.play_add_rate.get_or_insert(0);
    attr.play_drop_rate.get_or_insert(0);
    attr.dizzy_resistances.get_or_insert(0);
    attr.sleep_resistances.get_or_insert(0);
    attr.petrified_resistances.get_or_insert(0);
    attr.frozen_resistances.get_or_insert(0);
    attr.disarm_resistances.get_or_insert(0);
    attr.forbid_resistances.get_or_insert(0);
    attr.seal_resistances.get_or_insert(0);
    attr.cant_get_exskill_resistances.get_or_insert(0);
    attr.del_ex_point_resistances.get_or_insert(0);
    attr.stress_up_resistances.get_or_insert(0);
    attr.control_resilience.get_or_insert(0);
    attr.del_ex_point_resilience.get_or_insert(0);
    attr.stress_up_resilience.get_or_insert(0);
    attr.charm_resistances.get_or_insert(0);
    attr.rebound_dmg.get_or_insert(raw_modifier(resolved, 218));
    attr.extra_dmg.get_or_insert(raw_modifier(resolved, 219));
    attr.reuse_dmg.get_or_insert(raw_modifier(resolved, 220));
    attr.big_skill_rate.get_or_insert(0);
    attr.clutch_dmg.get_or_insert(0);
    attr
}

#[derive(Default, Debug, Clone)]
pub struct FightCalculateDataMgr {
    fight: Arc<Fight>,
    destiny_modifiers: Arc<DestinyModifierMap>,
    entity_mgr: FightEntityDataMgr,
    buff_mgr: BuffMgr,
}

impl FightCalculateDataMgr {
    pub fn new(fight: Arc<Fight>) -> Self {
        Self::new_with_destiny_modifiers(fight, Arc::new(DestinyModifierMap::new()))
    }

    pub fn new_with_destiny_modifiers(
        fight: Arc<Fight>,
        destiny_modifiers: Arc<DestinyModifierMap>,
    ) -> Self {
        Self {
            fight: fight.clone(),
            destiny_modifiers,
            entity_mgr: FightEntityDataMgr::new(fight.clone()),
            buff_mgr: BuffMgr::new(),
        }
    }

    pub fn play_step_data(
        &mut self,
        step: &mut FightStep,
        fight: &mut Fight,
        bloodtithe: &mut BloodtitheState,
        buff_mgr: &mut BuffMgr,
    ) -> Result<(), String> {
        for effect in &mut step.act_effect {
            self.play_act_effect_data(effect, fight, bloodtithe, buff_mgr)?;
        }
        Ok(())
    }

    pub fn play_step_data_list(
        &mut self,
        steps: &mut [FightStep],
        fight: &mut Fight,
        bloodtithe: &mut BloodtitheState,
        buff_mgr: &mut BuffMgr,
    ) -> Result<(), String> {
        for step in steps {
            self.play_step_data(step, fight, bloodtithe, buff_mgr)?;
        }
        Ok(())
    }

    pub fn play_act_effect_data(
        &mut self,
        effect: &mut ActEffect,
        fight: &mut Fight,
        bloodtithe: &mut BloodtitheState,
        buff_mgr: &mut BuffMgr,
    ) -> Result<(), String> {
        if let Some(ref mut nested) = effect.fight_step {
            return self.play_step_data(nested, fight, bloodtithe, buff_mgr);
        }

        let effect_type = EffectType::from(effect.effect_type.unwrap_or(0));

        match effect_type {
            // Just ignore
            EffectType::None | EffectType::FightStep | EffectType::MasterHalo => Ok(()),

            EffectType::Damage
            | EffectType::Crit
            | EffectType::DamageExtra
            | EffectType::OriginDamage
            | EffectType::OriginCrit
            | EffectType::DamageFromAbsorb
            | EffectType::DamageFromLostHp
            | EffectType::EnchantBurnDamage
            | EffectType::DamageShareHp
            | EffectType::DeadlyPoisonOriginDamage
            | EffectType::DeadlyPoisonOriginCrit
            | EffectType::AdditionalDamage
            | EffectType::AdditionalDamageCrit
            | EffectType::ShareHurt
            | EffectType::EnchantDepresseDamage => {
                self.play_effect_damage(effect, fight, bloodtithe)
            }

            EffectType::Heal
            | EffectType::Bloodlust
            | EffectType::InjuryBankHeal
            | EffectType::SubHeroLifeChange => self.play_effect_heal(effect, fight),

            EffectType::BuffAdd => self.play_effect_add_buff(effect),

            EffectType::Dead => self.play_effect_death(effect, fight),
            EffectType::Kill => self.play_effect_kill(effect, fight),

            EffectType::Shield => self.play_effect_shield(effect, fight),

            EffectType::AverageLife => self.play_effect_set_hp(effect, fight),
            EffectType::MaxHpChange => self.play_effect_set_max_hp(effect, fight),
            EffectType::CurrentHpChange => self.play_effect_set_current_hp(effect, fight),

            EffectType::AddExPoint | EffectType::ExPointChange => {
                self.play_effect_add_ex_point(effect, fight)
            }

            EffectType::PowerInfoChange => self.play_effect_power_info_change(effect, fight),

            EffectType::BloodPoolMaxCreate => self.play_effect_bloodtithe_enable(effect),
            EffectType::BloodPoolMaxChange => self.play_effect_bloodtithe_max(effect),
            EffectType::BloodPoolValueChange => self.play_effect_bloodtithe_value(effect),

            EffectType::FightHurtDetail => self.play_effect_fight_hurt_detail(effect, fight),

            // TODO
            EffectType::EnterFightDeal
            | EffectType::Attr
            | EffectType::TeammateInjuryCount
            | EffectType::ExPointOverflowBank
            | EffectType::Cure
            | EffectType::CardsPush
            | EffectType::CardDeckNum => Ok(()),

            other => {
                tracing::warn!("Unhandled effect type: {:?}", other);
                Ok(())
            }
        }
    }

    fn play_effect_damage(
        &mut self,
        effect: &ActEffect,
        fight: &mut Fight,
        bloodtithe: &mut BloodtitheState,
    ) -> Result<(), String> {
        let target_id = effect.target_id.ok_or("No target ID")?;
        let damage = effect.effect_num.ok_or("No damage amount")?;

        let location = self
            .entity_mgr
            .get_location(target_id)
            .ok_or_else(|| format!("Entity {} not found", target_id))?;

        let entity = get_entity_mut_by_location(fight, location)
            .ok_or_else(|| format!("Failed to get entity {} mutably", target_id))?;

        let current_hp = entity.current_hp.unwrap_or(0);
        entity.current_hp = Some((current_hp - damage).max(0));

        if let Some(team_type) = entity.team_type
            && damage > 0
        {
            bloodtithe.on_hp_lost(target_id, team_type, damage);
        }

        tracing::trace!("Damage applied: target={}, damage={}", target_id, damage);
        Ok(())
    }

    fn play_effect_heal(&mut self, effect: &ActEffect, fight: &mut Fight) -> Result<(), String> {
        let target_id = effect.target_id.ok_or("No target ID")?;
        let heal = effect.effect_num.ok_or("No heal amount")?;

        let location = self
            .entity_mgr
            .get_location(target_id)
            .ok_or_else(|| format!("Entity {} not found", target_id))?;

        let entity = get_entity_mut_by_location(fight, location)
            .ok_or_else(|| format!("Failed to get entity {} mutably", target_id))?;

        let current_hp = entity.current_hp.unwrap_or(0);
        let max_hp = entity
            .attr
            .as_ref()
            .and_then(|a| a.hp)
            .unwrap_or(current_hp);
        entity.current_hp = Some((current_hp + heal).min(max_hp));

        tracing::trace!("Heal applied: target={}, heal={}", target_id, heal);
        Ok(())
    }

    fn play_effect_add_buff(&mut self, effect: &ActEffect) -> Result<(), String> {
        let target_id = effect.target_id.ok_or("No target ID")?;
        let buff_id = effect.effect_num.ok_or("No buff ID")?;

        let from_uid = effect.buff.as_ref().and_then(|b| b.from_uid).unwrap_or(0);

        self.buff_mgr.add_buff(target_id, buff_id, from_uid);

        Ok(())
    }

    fn play_effect_death(&mut self, effect: &ActEffect, fight: &mut Fight) -> Result<(), String> {
        let target_id = effect.target_id.ok_or("No target ID")?;

        let location = self
            .entity_mgr
            .get_location(target_id)
            .ok_or_else(|| format!("Entity {} not found", target_id))?;

        let entity = get_entity_mut_by_location(fight, location)
            .ok_or_else(|| format!("Failed to get entity {} mutably", target_id))?;

        entity.current_hp = Some(0);
        self.buff_mgr.clear_dead(target_id);

        tracing::trace!("Entity died: target={}", target_id);
        Ok(())
    }

    fn play_effect_kill(&mut self, effect: &ActEffect, fight: &mut Fight) -> Result<(), String> {
        let target_id = effect.target_id.ok_or("No target ID")?;

        let location = self
            .entity_mgr
            .get_location(target_id)
            .ok_or_else(|| format!("Entity {} not found", target_id))?;

        let entity = get_entity_mut_by_location(fight, location)
            .ok_or_else(|| format!("Failed to get entity {} mutably", target_id))?;

        entity.current_hp = Some(0);
        self.buff_mgr.clear_dead(target_id);

        tracing::trace!("Entity killed: target={}", target_id);
        Ok(())
    }

    fn play_effect_shield(&mut self, effect: &ActEffect, _fight: &mut Fight) -> Result<(), String> {
        let target_id = effect.target_id.ok_or("No target ID")?;
        let shield = effect.effect_num.ok_or("No shield amount")?;

        tracing::trace!("Shield applied: target={}, shield={}", target_id, shield);
        // TODO: Add shield to entity
        Ok(())
    }

    fn play_effect_set_hp(&mut self, effect: &ActEffect, fight: &mut Fight) -> Result<(), String> {
        let target_id = effect.target_id.ok_or("No target ID")?;
        let hp = effect.effect_num.ok_or("No HP amount")?;

        let location = self
            .entity_mgr
            .get_location(target_id)
            .ok_or_else(|| format!("Entity {} not found", target_id))?;

        let entity = get_entity_mut_by_location(fight, location)
            .ok_or_else(|| format!("Failed to get entity {} mutably", target_id))?;

        entity.current_hp = Some(hp);
        tracing::trace!("HP set: target={}, hp={}", target_id, hp);
        Ok(())
    }

    fn play_effect_set_max_hp(
        &mut self,
        effect: &ActEffect,
        fight: &mut Fight,
    ) -> Result<(), String> {
        let target_id = effect.target_id.ok_or("No target ID")?;
        let max_hp = effect.effect_num.ok_or("No max HP amount")?;

        let location = self
            .entity_mgr
            .get_location(target_id)
            .ok_or_else(|| format!("Entity {} not found", target_id))?;

        let entity = get_entity_mut_by_location(fight, location)
            .ok_or_else(|| format!("Failed to get entity {} mutably", target_id))?;

        if let Some(attr) = entity.attr.as_mut() {
            attr.hp = Some(max_hp);
        }

        if let Some(base) = entity.base_attr.as_mut() {
            base.hp = Some(max_hp);
        }

        tracing::trace!("Max HP set: target={}, max_hp={}", target_id, max_hp);
        Ok(())
    }

    fn play_effect_set_current_hp(
        &mut self,
        effect: &ActEffect,
        fight: &mut Fight,
    ) -> Result<(), String> {
        self.play_effect_set_hp(effect, fight)
    }

    fn play_effect_add_ex_point(
        &mut self,
        effect: &ActEffect,
        _fight: &mut Fight,
    ) -> Result<(), String> {
        let target_id = effect.target_id.ok_or("No target ID")?;
        let ex_point = effect.effect_num.unwrap_or(0);

        tracing::trace!("EX point added: target={}, amount={}", target_id, ex_point);
        // TODO: Add EX points to entity
        Ok(())
    }

    fn play_effect_power_info_change(
        &mut self,
        effect: &mut ActEffect,
        fight: &mut Fight,
    ) -> Result<(), String> {
        let target_id = effect.target_id.ok_or("PowerInfoChange missing target ID")?;
        let incoming = effect
            .power_info
            .as_ref()
            .ok_or("PowerInfoChange missing PowerInfo")?;
        let location = self
            .entity_mgr
            .get_location(target_id)
            .ok_or_else(|| format!("Entity {target_id} not found"))?;
        let entity = get_entity_mut_by_location(fight, location)
            .ok_or_else(|| format!("Failed to get entity {target_id} mutably"))?;
        let power_id = incoming.power_id;
        apply_power_info_change(entity, incoming).map_err(|error| error.to_string())?;
        effect.power_info = entity
            .power_infos
            .iter()
            .find(|power| power.power_id == power_id)
            .copied();
        Ok(())
    }

    fn play_effect_fight_hurt_detail(
        &mut self,
        effect: &ActEffect,
        _fight: &mut Fight,
    ) -> Result<(), String> {
        let target_id = effect.target_id.ok_or("No target ID")?;
        let hurt = effect.effect_num.unwrap_or(0);

        tracing::trace!("Hurt detail: target={}, amount={}", target_id, hurt);

        Ok(())
    }

    fn play_effect_bloodtithe_enable(&mut self, effect: &ActEffect) -> Result<(), String> {
        let team_type = effect.team_type.unwrap_or(1);
        tracing::trace!("Bloodtithe enabled: team={}", team_type);
        Ok(())
    }

    fn play_effect_bloodtithe_max(&mut self, effect: &ActEffect) -> Result<(), String> {
        let team_type = effect.team_type.unwrap_or(1);
        let max = effect.effect_num1.unwrap_or(0);
        tracing::trace!("Bloodtithe max: team={}, max={}", team_type, max);
        Ok(())
    }

    fn play_effect_bloodtithe_value(&mut self, effect: &ActEffect) -> Result<(), String> {
        let target_id = effect.target_id.unwrap_or(0);
        let value = effect.effect_num.unwrap_or(0);
        tracing::trace!("Bloodtithe value: target={}, value={}", target_id, value);
        Ok(())
    }

    pub fn update_fight(&mut self, fight: Arc<Fight>) {
        self.fight = fight.clone();
        self.entity_mgr.update_fight(fight);
    }
}

impl FightCalculateDataMgr {
    pub fn build_ex_point_info(&mut self, fight: &Fight) -> Vec<FightExPointInfo> {
        let mut info = Vec::new();

        if let Some(ref attacker) = fight.attacker {
            for entity in &attacker.entitys {
                info.push(FightExPointInfo {
                    uid: entity.uid,
                    ex_point: entity.ex_point,
                    power_infos: entity.power_infos.clone(),
                    current_hp: entity.current_hp,
                    ex_point_type: if entity.model_id == Some(3120) {
                        Some(1)
                    } else {
                        Some(0)
                    },
                });
            }

            for entity in &attacker.sub_entitys {
                info.push(FightExPointInfo {
                    uid: entity.uid,
                    ex_point: entity.ex_point,
                    power_infos: entity.power_infos.clone(),
                    current_hp: entity.current_hp,
                    ex_point_type: Some(0),
                });
            }
        }

        if let Some(ref defender) = fight.defender {
            for entity in &defender.entitys {
                info.push(FightExPointInfo {
                    uid: entity.uid,
                    ex_point: entity.ex_point,
                    power_infos: entity.power_infos.clone(),
                    current_hp: entity.current_hp,
                    ex_point_type: Some(0),
                });
            }
        }

        info
    }

    pub fn build_hero_sp_attributes(&mut self, fight: &Fight) -> Vec<FightHeroSpAttributeInfo> {
        let mut attrs = Vec::new();
        let mut append = |entity: &sonettobuf::FightEntityInfo| {
            attrs.push(FightHeroSpAttributeInfo {
                uid: entity.uid,
                attribute: Some(hero_sp_attribute_from_destiny(
                    entity.uid.and_then(|uid| self.destiny_modifiers.get(&uid)),
                )),
            });
        };
        if let Some(attacker) = &fight.attacker {
            for entity in attacker.entitys.iter().chain(attacker.sub_entitys.iter()) {
                append(entity);
            }
        }
        if let Some(defender) = &fight.defender {
            for entity in &defender.entitys {
                append(entity);
            }
        }
        attrs
    }

    pub fn build_player_skills(&mut self) -> Vec<PlayerSkillInfo> {
        vec![
            PlayerSkillInfo {
                skill_id: Some(30010201),
                cd: Some(0),
                need_power: Some(40),
                r#type: Some(0),
            },
            PlayerSkillInfo {
                skill_id: Some(30010202),
                cd: Some(0),
                need_power: Some(25),
                r#type: Some(0),
            },
        ]
    }
}

impl FightCalculateDataMgr {
    pub fn on_round_end(&mut self) {
        self.buff_mgr.on_round_end();
    }
}
