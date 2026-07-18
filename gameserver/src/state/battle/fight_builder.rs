use super::BattleContext;
use super::destiny::{
    DestinyModifierMap, DestinyState, HeroBaseAttributes, HeroBuildContext, HeroSource,
    ResolvedDestinyAttributes,
};
use super::entity_builder;
use super::trial::{
    MAX_TRIAL_UID_ORDINAL, normalize_trial_requests, reserved_attacker_uid_ordinal,
};
use anyhow::Result;

use database::models::game::heros::{HeroModel, UserHeroModel};

use sonettobuf::{EquipRecord, Fight, FightEntityInfo, FightTeam, HeroAttribute};
use sqlx::SqlitePool;
use std::collections::HashSet;

pub async fn build_fight(
    pool: &SqlitePool,
    ctx: &BattleContext,
    fight_group: &sonettobuf::FightGroup,
) -> Result<Fight> {
    Ok(build_fight_with_destiny_modifiers(pool, ctx, fight_group)
        .await?
        .0)
}

pub async fn build_fight_with_destiny_modifiers(
    pool: &SqlitePool,
    ctx: &BattleContext,
    fight_group: &sonettobuf::FightGroup,
) -> Result<(Fight, DestinyModifierMap)> {
    // Build attacker team (player)
    let (attacker, modifiers) =
        build_attacker_team_with_destiny_modifiers(pool, ctx.player_id, fight_group).await?;

    // Build defender team (enemies from episode config)
    let defender = build_defender_team(ctx.episode_id).await?;

    let fight = Fight {
        attacker: Some(attacker),
        defender: Some(defender.team),
        cur_round: Some(1),
        max_round: Some(defender.max_round),
        is_finish: Some(false), // determines if fight is over
        cur_wave: Some(1),
        battle_id: Some(ctx.battle_id),
        magic_circle: None,
        version: Some(5),
        is_record: Some(false), // enables sweep feature
        episode_id: Some(ctx.episode_id),
        fight_act_type: Some(sonettobuf::fight::FightActType::Normal.into()),
        last_change_hero_uid: Some(0),
        progress: Some(0),
        progress_max: Some(0),
        param: vec![],
        custom_data: vec![],
        fight_task_box: Some(sonettobuf::FightTaskBox { tasks: vec![] }),
        progress_list: vec![],
    };
    Ok((fight, modifiers))
}

pub async fn build_attacker_team(
    pool: &SqlitePool,
    user_id: i64,
    fight_group: &sonettobuf::FightGroup,
) -> Result<FightTeam> {
    Ok(
        build_attacker_team_with_destiny_modifiers(pool, user_id, fight_group)
            .await?
            .0,
    )
}

pub async fn build_attacker_team_with_destiny_modifiers(
    pool: &SqlitePool,
    user_id: i64,
    fight_group: &sonettobuf::FightGroup,
) -> Result<(FightTeam, DestinyModifierMap)> {
    let mut entitys = Vec::new();
    let mut ordered_sub_entitys = Vec::new();
    let mut modifiers = DestinyModifierMap::new();
    let trials = normalize_trial_requests(fight_group)?;
    let trial_uid_count = reserved_attacker_uid_ordinal(fight_group, &trials);
    let hero = UserHeroModel::new(user_id, pool.clone());
    let mut occupied_positions = HashSet::new();
    let mut occupied_sub_positions = HashSet::new();

    for position in trials
        .iter()
        .filter(|trial| !trial.is_substitute)
        .filter_map(|trial| trial.position)
    {
        if !occupied_positions.insert(position) {
            return Err(anyhow::anyhow!("duplicate attacker position {position}"));
        }
    }
    for position in trials
        .iter()
        .filter(|trial| trial.is_substitute)
        .filter_map(|trial| trial.position)
    {
        let position = position
            .checked_neg()
            .ok_or_else(|| anyhow::anyhow!("invalid substitute trial position {position}"))?;
        if !occupied_sub_positions.insert(position) {
            return Err(anyhow::anyhow!("duplicate substitute position {position}"));
        }
    }

    // Main heroes
    let explicit_trials = !fight_group.trial_hero_list.is_empty();
    let mut compacted_position = 1;
    for (wire_position, hero_uid) in fight_group.hero_list.iter().enumerate() {
        if *hero_uid < 0 && !explicit_trials {
            continue;
        }
        let position = if explicit_trials {
            claim_next_attacker_position(&mut occupied_positions, &mut compacted_position)?
        } else {
            let position = i32::try_from(wire_position + 1)
                .map_err(|_| anyhow::anyhow!("attacker hero position is outside i32 range"))?;
            if !occupied_positions.insert(position) {
                return Err(anyhow::anyhow!("duplicate attacker position {position}"));
            }
            position
        };
        if *hero_uid <= 0 {
            continue;
        }
        let hero_data = hero.get_uid(*hero_uid as i32).await?;
        let entity =
            entity_builder::build_hero_entity(pool, &hero_data, position, 1, false).await?;
        collect_destiny_modifiers(&mut modifiers, entity.uid, &hero_data, false);
        entitys.push(entity);
    }

    // Sub heroes
    let mut owned_substitute_index = 0;
    let mut compacted_sub_position = 1;
    for (wire_position, hero_uid) in fight_group.sub_hero_list.iter().enumerate() {
        if *hero_uid < 0 && !explicit_trials {
            continue;
        }
        let sub_position = if explicit_trials {
            claim_next_attacker_position(&mut occupied_sub_positions, &mut compacted_sub_position)?
        } else {
            let position = i32::try_from(wire_position + 1).map_err(|_| {
                anyhow::anyhow!("attacker substitute position is outside i32 range")
            })?;
            if !occupied_sub_positions.insert(position) {
                return Err(anyhow::anyhow!("duplicate substitute position {position}"));
            }
            position
        };
        if *hero_uid <= 0 {
            continue;
        }
        let uid = try_attacker_substitute_uid(trial_uid_count, owned_substitute_index)?;
        owned_substitute_index += 1;
        let hero_data = hero.get_uid(*hero_uid as i32).await?;
        let mut entity = entity_builder::build_hero_entity(pool, &hero_data, -1, 1, true).await?;
        entity.uid = Some(uid);
        if let Some(enhance_info) = entity.enhance_info_box.as_mut() {
            enhance_info.uid = Some(uid);
        }
        collect_destiny_modifiers(&mut modifiers, entity.uid, &hero_data, true);
        ordered_sub_entitys.push((sub_position, entity));
    }

    for trial in trials {
        let (position, substitute_position) = if trial.is_substitute {
            let position = trial.position.and_then(i32::checked_neg).ok_or_else(|| {
                anyhow::anyhow!(
                    "substitute trial {} has no normalized position",
                    trial.trial_id
                )
            })?;
            (-1, Some(position))
        } else {
            (
                trial.position.ok_or_else(|| {
                    anyhow::anyhow!("active trial {} has no normalized position", trial.trial_id)
                })?,
                None,
            )
        };
        let (entity, modifier) =
            build_trial_hero_entity(trial.uid, trial.trial_id, position, 1, trial.is_substitute)?;
        modifiers.insert(trial.uid, modifier);
        if let Some(substitute_position) = substitute_position {
            ordered_sub_entitys.push((substitute_position, entity));
        } else {
            entitys.push(entity);
        }
    }

    entitys.sort_by_key(|entity| entity.position.unwrap_or(i32::MAX));
    ordered_sub_entitys.sort_by_key(|(position, _)| *position);
    let sub_entitys = ordered_sub_entitys
        .into_iter()
        .map(|(_, entity)| entity)
        .collect();

    let player_entity = entity_builder::build_player_entity(user_id, 1);

    let team = build_fight_team(
        entitys,
        sub_entitys,
        player_entity,
        Some(15),
        fight_group.cloth_id,
        build_player_skills(fight_group.cloth_id),
    );
    Ok((team, modifiers))
}

fn claim_next_attacker_position(occupied: &mut HashSet<i32>, next: &mut i32) -> Result<i32> {
    while occupied.contains(next) {
        *next = next
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("attacker position overflow"))?;
    }
    let position = *next;
    occupied.insert(position);
    *next = next
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("attacker position overflow"))?;
    Ok(position)
}

fn collect_destiny_modifiers(
    modifiers: &mut DestinyModifierMap,
    uid: Option<i64>,
    hero_data: &database::models::game::heros::HeroData,
    is_substitute: bool,
) {
    let Some(uid) = uid else { return };
    match entity_builder::resolve_hero_destiny_attributes(hero_data, is_substitute) {
        Ok(resolved) => {
            modifiers.insert(
                uid,
                entity_builder::merge_hero_combat_attributes(resolved, hero_data),
            );
        }
        Err(error) => tracing::warn!(
            uid,
            hero_id = hero_data.record.hero_id,
            error = %error,
            "Destiny combat modifiers unavailable; preserving base battle behavior"
        ),
    }
}

fn build_trial_hero_entity(
    hero_uid: i64,
    trial_id: i32,
    position: i32,
    team_type: i32,
    is_substitute: bool,
) -> Result<(FightEntityInfo, ResolvedDestinyAttributes)> {
    use config::configs;

    let game_data = configs::get();
    let trial_data = game_data
        .hero_trial
        .get(trial_id)
        .ok_or_else(|| anyhow::anyhow!("Trial data not found for ID {}", trial_id))?;

    tracing::info!(
        "Building trial hero entity: UID {} -> trial_id {} -> hero_id {}, level {}",
        hero_uid,
        trial_id,
        trial_data.hero_id,
        trial_data.level
    );

    // Get hero config for skills and career
    let hero_config = game_data
        .character
        .iter()
        .find(|h| h.id == trial_data.hero_id)
        .ok_or_else(|| {
            anyhow::anyhow!("Hero config not found for hero_id {}", trial_data.hero_id)
        })?;

    // Try to find exact level first
    let char_level_opt = game_data
        .character_level
        .iter()
        .find(|c| c.hero_id == trial_data.hero_id && c.level == trial_data.level);

    let (hp, attack, defense, mdefense, technic) = if let Some(char_level) = char_level_opt {
        // Found exact level
        (
            char_level.hp,
            char_level.atk,
            char_level.def,
            char_level.mdef,
            char_level.technic,
        )
    } else {
        // Level not found - try level 1 as base
        let base_level = game_data
            .character_level
            .iter()
            .find(|c| c.hero_id == trial_data.hero_id && c.level == 1)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "No level data found for hero_id {} (tried level {} and level 1)",
                    trial_data.hero_id,
                    trial_data.level
                )
            })?;

        tracing::warn!(
            "Level {} not found for hero {}, using level 1 base stats",
            trial_data.level,
            trial_data.hero_id
        );

        // Use level 1 stats as base (you could calculate growth here if needed)
        (
            base_level.hp,
            base_level.atk,
            base_level.def,
            base_level.mdef,
            base_level.technic,
        )
    };

    let has_destiny = trial_data.facetslevel > 0 && trial_data.facets_id > 0;
    let context = HeroBuildContext {
        hero_id: trial_data.hero_id,
        skin: trial_data.skin,
        rank: hero_config.rank,
        ex_skill_level: trial_data.ex_skill_lv,
        destiny: DestinyState {
            rank: trial_data.facetslevel,
            // FightEntityMO refreshes trial Destiny with the configured rank
            // and facet at battle node level one.
            level: i32::from(has_destiny),
            facet_id: trial_data.facets_id,
        },
        is_substitute,
        hero_type: hero_config.hero_type,
        source: HeroSource::Trial,
    };
    let kit = entity_builder::resolve_entity_kit(&context).map_err(|error| {
        anyhow::anyhow!(
            "failed to resolve trial {} hero {} kit: {}",
            trial_id,
            trial_data.hero_id,
            error
        )
    })?;
    let base = HeroBaseAttributes {
        hp,
        attack,
        defense,
        mdefense,
    };
    let modifier =
        entity_builder::resolve_entity_destiny_attributes(&context, base).map_err(|error| {
            anyhow::anyhow!(
                "failed to resolve trial {} hero {} attributes: {}",
                trial_id,
                trial_data.hero_id,
                error
            )
        })?;
    let attr = HeroAttribute {
        hp: Some(hp.saturating_add(modifier.hp)),
        attack: Some(attack.saturating_add(modifier.attack)),
        defense: Some(defense.saturating_add(modifier.defense)),
        mdefense: Some(mdefense.saturating_add(modifier.mdefense)),
        technic: Some(technic),
        multi_hp_idx: Some(0),
        multi_hp_num: Some(0),
    };
    let current_hp = attr.hp;

    let entity = FightEntityInfo {
        uid: Some(hero_uid),
        model_id: Some(trial_data.hero_id),
        skin: Some(trial_data.skin),
        position: Some(position),
        entity_type: Some(1), // 1 = Hero
        user_id: Some(0),     // Trial heroes have no owner
        ex_point: Some(0),
        level: Some(trial_data.level),
        current_hp,
        attr: Some(attr.clone()),
        buffs: vec![],
        skill_group1: kit.skill_group_1,
        skill_group2: kit.skill_group_2,
        passive_skill: kit.passives,
        ex_skill: Some(kit.ultimate),
        shield_value: Some(0),
        no_effect_buffs: vec![],
        expoint_max_add: Some(0),
        buff_harm_statistic: Some(0),
        equip_uid: Some(0),
        trial_equip: Some(EquipRecord {
            equip_uid: Some(0),
            equip_id: Some(trial_data.equip_id),
            equip_lv: Some(trial_data.equip_lv),
            refine_lv: Some(trial_data.equip_refine),
        }),
        ex_skill_level: Some(trial_data.ex_skill_lv),
        power_infos: kit.power_infos,
        act104_equip_uids: vec![],
        trial_act104_equips: vec![],
        summoned_list: vec![],
        base_attr: Some(attr),
        ex_skill_point_change: Some(0),
        team_type: Some(team_type),
        enhance_info_box: Some(sonettobuf::EnhanceInfoBox {
            uid: Some(hero_uid),
            can_upgrade_ids: vec![],
            upgraded_options: vec![],
        }),
        trial_id: Some(trial_data.id),
        career: Some(hero_config.career),
        status: Some(0),
        guard: Some(-1),
        sub_cd: Some(0),
        ex_point_type: Some(0),
        equips: vec![],
        destiny_stone: Some(trial_data.facets_id),
        destiny_rank: Some(trial_data.facetslevel),
        custom_unit_id: Some(0),
    };
    Ok((entity, modifier))
}

pub struct BattleSetup {
    pub max_round: i32,
    pub team: FightTeam,
}

async fn build_defender_team(episode_id: i32) -> Result<BattleSetup> {
    use config::configs;
    let game_data = configs::get();

    let episode = game_data
        .episode
        .iter()
        .find(|e| e.id == episode_id)
        .ok_or_else(|| anyhow::anyhow!("Episode {} not found", episode_id))?;

    let battle = game_data
        .battle
        .iter()
        .find(|b| b.id == episode.battle_id)
        .ok_or_else(|| anyhow::anyhow!("Battle {} not found", episode.battle_id))?;

    let max_round = battle.max_round;

    tracing::info!(
        "Loading battle {}: monsterGroupIds={}, maxRound={}",
        episode.battle_id,
        battle.monster_group_ids,
        max_round
    );

    let monster_ids: Vec<i32> = battle
        .monster_group_ids
        .split('#')
        .filter_map(|s| s.parse::<i32>().ok())
        .collect();

    let mut entitys = Vec::new();
    for (idx, monster_id) in monster_ids.iter().enumerate() {
        let entity = build_enemy_entity(*monster_id, idx, (idx + 1) as i32, 2)?;

        tracing::info!(
            "Enemy entity: monster_id={}, position={}, uid={:?}",
            monster_id,
            idx + 1,
            entity.uid
        );

        entitys.push(entity);
    }

    tracing::info!("Built {} enemy entities", entitys.len());

    let player_entity = entity_builder::build_player_entity(0, 2);

    let fight_team = build_fight_team(entitys, vec![], player_entity, Some(0), Some(0), vec![]);

    Ok(BattleSetup {
        max_round,
        team: fight_team,
    })
}

fn build_fight_team(
    entitys: Vec<sonettobuf::FightEntityInfo>,
    sub_entitys: Vec<sonettobuf::FightEntityInfo>,
    player_entity: sonettobuf::FightEntityInfo,
    power: Option<i32>,
    cloth_id: Option<i32>,
    skill_infos: Vec<sonettobuf::PlayerSkillInfo>,
) -> FightTeam {
    FightTeam {
        entitys,
        sub_entitys,
        power,
        cloth_id,
        skill_infos,
        sp_entitys: vec![],
        indicators: vec![],
        ex_team_str: Some(String::new()),
        assist_boss: None,
        assist_boss_info: None,
        emitter: None,
        emitter_info: None,
        player_entity: Some(player_entity),
        player_finisher_info: None,
        energy: Some(0),
        card_heat: Some(sonettobuf::CardHeatInfo { values: vec![] }),
        card_deck_size: Some(0),
        blood_pool: None,
        vorpalith: None,
        item_skill_group: None,
        sp_fight_entities: vec![],
    }
}

fn build_enemy_entity(
    monster_id: i32,
    idx: usize,
    position: i32,
    team_type: i32,
) -> Result<sonettobuf::FightEntityInfo> {
    use config::configs;
    use sonettobuf::{EquipRecord, FightEntityInfo, HeroAttribute};

    let game_data = configs::get();

    let monster = game_data
        .monster
        .iter()
        .find(|m| m.id == monster_id)
        .ok_or_else(|| anyhow::anyhow!("Monster {} not found", monster_id))?;

    let template_id = if monster.template != 0 {
        monster.template
    } else {
        monster.skill_template
    };

    let template = game_data
        .monster_template
        .iter()
        .find(|t| t.template == template_id)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Monster template {} not found (monster {})",
                template_id,
                monster_id
            )
        })?;

    let skill_template = game_data
        .monster_skill_template
        .iter()
        .find(|s| s.id == monster.skill_template)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Monster skill template {} not found (monster {})",
                monster.skill_template,
                monster_id
            )
        })?;

    // Calculate stats based on level
    let level = if monster.level_true != 0 {
        monster.level_true
    } else {
        monster.level
    };

    let hp = template.life + (template.life_grow * level);
    let attack = template.attack + (template.attack_grow * level);
    let defense = template.defense + (template.defense_grow * level);
    let mdefense = template.mdefense + (template.mdefense_grow * level);
    let technic = template.technic + (template.technic_grow * level);

    // Parse skills: "1#40212511#40212512|2#40212521#40212522"
    let skill_group1 = parse_monster_skill_group(&skill_template.active_skill, 1);
    let skill_group2 = parse_monster_skill_group(&skill_template.active_skill, 2);

    // Parse passive skills: "2108" or "2108#2109"
    let passive_skill: Vec<i32> = skill_template
        .passive_skill
        .split('#')
        .filter_map(|s| s.parse::<i32>().ok())
        .collect();

    // Get ex skill (first skill from uniqueSkill)
    let ex_skill = skill_template
        .unique_skill
        .split('#')
        .next()
        .and_then(|s| s.parse::<i32>().ok())
        .unwrap_or(0);

    let uid = enemy_entity_uid(idx);

    tracing::debug!(
        "Enemy: monster_id={}, skill_template={}, uid={}, hp={}, skills1={:?}",
        monster_id,
        monster.skill_template,
        uid,
        hp,
        skill_group1
    );

    Ok(FightEntityInfo {
        uid: Some(uid),
        model_id: Some(monster.id), // Use monster.id as model_id
        skin: Some(monster.skin_id),
        position: Some(position),
        entity_type: Some(2), // 2 = Enemy
        user_id: Some(0),
        ex_point: Some(0),
        level: Some(level),
        current_hp: Some(hp),
        attr: Some(HeroAttribute {
            hp: Some(hp),
            attack: Some(attack),
            defense: Some(defense),
            mdefense: Some(mdefense),
            technic: Some(technic),
            multi_hp_idx: Some(0),
            multi_hp_num: Some(0),
        }),
        buffs: vec![],
        skill_group1,
        skill_group2,
        passive_skill,
        ex_skill: Some(ex_skill),
        shield_value: Some(0),
        no_effect_buffs: vec![],
        expoint_max_add: Some(0),
        buff_harm_statistic: Some(0),
        equip_uid: Some(0),
        trial_equip: Some(EquipRecord {
            equip_uid: Some(0),
            equip_id: Some(0),
            equip_lv: Some(0),
            refine_lv: Some(0),
        }),
        ex_skill_level: Some(0),
        power_infos: vec![],
        act104_equip_uids: vec![],
        trial_act104_equips: vec![],
        summoned_list: vec![],
        base_attr: Some(HeroAttribute {
            hp: Some(hp),
            attack: Some(attack),
            defense: Some(defense),
            mdefense: Some(mdefense),
            technic: Some(technic),
            multi_hp_idx: Some(0),
            multi_hp_num: Some(0),
        }),
        ex_skill_point_change: Some(0),
        team_type: Some(team_type),
        enhance_info_box: Some(sonettobuf::EnhanceInfoBox {
            uid: Some(uid),
            can_upgrade_ids: vec![],
            upgraded_options: vec![],
        }),
        trial_id: Some(0),
        career: Some(skill_template.career),
        status: Some(0),
        guard: Some(-1),
        sub_cd: Some(0),
        ex_point_type: Some(0),
        equips: vec![],
        destiny_stone: Some(0),
        destiny_rank: Some(0),
        custom_unit_id: Some(0),
    })
}

/// Keep defender UIDs distinct from attacker trial ordinals (-1, -2, ...).
/// The existing AI deck wire convention starts defenders at -1001.
pub fn enemy_entity_uid(index: usize) -> i64 {
    -1001 - index as i64
}

/// Reserve `-1..=-trial_count` for trial heroes, then assign substitutes.
pub fn attacker_substitute_uid(trial_count: usize, substitute_index: usize) -> i64 {
    try_attacker_substitute_uid(trial_count, substitute_index)
        .expect("attacker substitute UID crosses the defender namespace")
}

/// Allocate a substitute UID without entering the defender namespace.
pub fn try_attacker_substitute_uid(trial_count: usize, substitute_index: usize) -> Result<i64> {
    let ordinal = trial_count
        .checked_add(substitute_index)
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| anyhow::anyhow!("attacker substitute UID ordinal overflow"))?;
    if ordinal > MAX_TRIAL_UID_ORDINAL {
        return Err(anyhow::anyhow!(
            "attacker substitute UID ordinal {ordinal} crosses the defender UID namespace"
        ));
    }
    Ok(-(ordinal as i64))
}

fn parse_monster_skill_group(active_skill: &str, target_group: i32) -> Vec<i32> {
    // Parse: "1#40212511#40212512|2#40212521#40212522"
    for group_str in active_skill.split('|') {
        let parts: Vec<&str> = group_str.split('#').collect();

        if let Some(first) = parts.first()
            && let Ok(group_num) = first.parse::<i32>()
            && group_num == target_group
        {
            return parts[1..]
                .iter()
                .filter_map(|s| s.parse::<i32>().ok())
                .collect();
        }
    }

    vec![]
}

fn build_player_skills(cloth_id: Option<i32>) -> Vec<sonettobuf::PlayerSkillInfo> {
    use config::configs;

    let game_data = configs::get();
    let cloth_id = cloth_id.unwrap_or(1);

    let cloth_level = game_data
        .cloth_level
        .iter()
        .find(|c| c.id == cloth_id && c.level == 1);

    if let Some(cloth) = cloth_level {
        let mut skills = Vec::new();

        // Skill 1
        if cloth.skill1 != 0 {
            skills.push(sonettobuf::PlayerSkillInfo {
                skill_id: Some(cloth.skill1),
                cd: Some(cloth.cd1),
                need_power: Some(cloth.use_power1.first().copied().unwrap_or(0)),
                r#type: Some(0),
            });
        }

        // Skill 2
        if cloth.skill2 != 0 {
            skills.push(sonettobuf::PlayerSkillInfo {
                skill_id: Some(cloth.skill2),
                cd: Some(cloth.cd2),
                need_power: Some(cloth.use_power2.first().copied().unwrap_or(0)),
                r#type: Some(0),
            });
        }

        // Skill 3
        if cloth.skill3 != 0 {
            skills.push(sonettobuf::PlayerSkillInfo {
                skill_id: Some(cloth.skill3),
                cd: Some(cloth.cd3),
                need_power: None,
                r#type: Some(0),
            });
        }

        return skills;
    }

    vec![]
}
