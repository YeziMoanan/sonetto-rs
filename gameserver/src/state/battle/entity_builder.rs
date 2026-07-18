use config::configs;
use database::{
    db::game::equipment::Equipment,
    models::game::{equipment::UserEquipmentModel, heros::HeroData},
};
use anyhow::Result;
use super::destiny::{
    DestinyResolveError, DestinyState, HeroBaseAttributes, HeroBuildContext, HeroSource,
    ResolvedHeroKit, resolve_destiny_attributes, resolve_hero_kit,
};
use sonettobuf::{EquipRecord, FightEntityInfo, HeroAttribute};
use sqlx::SqlitePool;

/// Resolve the complete combat kit for every hero source. This is the only
/// entry point entity construction should use for skills/passives/ultimate.
pub fn resolve_entity_kit(
    context: &HeroBuildContext,
) -> Result<ResolvedHeroKit, DestinyResolveError> {
    let game = configs::get();
    let index = config::destiny::DestinyConfigIndex::try_from_game_db(game)
        .map_err(|error| DestinyResolveError::InvalidConfig(error.to_string()))?;

    match resolve_hero_kit(&index, game, context) {
        Ok(mut kit) => {
            apply_compatibility_passives(context, &mut kit);
            Ok(kit)
        }
        Err(error) if context.destiny.facet_id > 0 && context.destiny.rank > 0 => {
            tracing::warn!(
                hero_id = context.hero_id,
                facet_id = context.destiny.facet_id,
                rank = context.destiny.rank,
                error = %error,
                "inactive invalid Destiny facet; preserving legacy hero construction"
            );
            let mut inactive = context.clone();
            inactive.destiny = DestinyState::default();
            let mut kit = resolve_hero_kit(&index, game, &inactive)?;
            apply_compatibility_passives(&inactive, &mut kit);
            Ok(kit)
        }
        Err(error) => Err(error),
    }
}

fn hero_build_context(
    hero_data: &HeroData,
    is_substitute: bool,
    source: HeroSource,
) -> HeroBuildContext {
    let record = &hero_data.record;
    let game = configs::get();
    let character = game.character.get(record.hero_id);
    HeroBuildContext {
        hero_id: record.hero_id,
        skin: record.skin,
        rank: record.rank,
        ex_skill_level: record.ex_skill_level,
        destiny: DestinyState {
            rank: record.destiny_rank,
            level: record.destiny_level,
            facet_id: record.destiny_stone,
        },
        is_substitute,
        hero_type: character.map(|value| value.hero_type).unwrap_or(1),
        source,
    }
}

fn apply_compatibility_passives(context: &HeroBuildContext, kit: &mut ResolvedHeroKit) {
    let destiny_facet_active = kit
        .trace
        .iter()
        .any(|entry| entry.detail.contains("active at rank"));
    if context.hero_id == 3088 && destiny_facet_active {
        if let Some(passive) = kit
            .passives
            .iter_mut()
            .find(|id| **id == 30880164 || **id == 308801641)
        {
            *passive = 308801611;
        }
        for bonus in [308801911, 308801921, 308802111] {
            if !kit.passives.contains(&bonus) {
                kit.passives.push(bonus);
            }
        }
    }
    if context.hero_id == 3126 && !kit.passives.contains(&31260191) {
        kit.passives.push(31260191);
    }
}

fn append_equipment_passives(
    game: &config::GameDB,
    equip_id: Option<i32>,
    passives: &mut Vec<i32>,
) {
    let Some(eid) = equip_id else { return };
    let Some(equip) = game.equip_skill.iter().find(|equip| equip.id == eid) else {
        return;
    };
    for skill in [equip.skill, equip.skill2] {
        if skill != 0 {
            passives.push(skill);
        }
    }
}

pub async fn build_hero_entity(
    pool: &SqlitePool,
    hero_data: &HeroData,
    position: i32,
    team_type: i32,
    is_sub: bool,
) -> Result<FightEntityInfo> {
    let record = &hero_data.record;

    let equip_model = UserEquipmentModel::new(record.user_id, pool.clone());
    let equip_data = equip_model.get_equip(record.default_equip_uid).await.ok();

    let equip_id = equip_data.as_ref().map(|equip| equip.equip_id);
    let game = configs::get();
    let source = activity_source_for_hero(record.hero_id, record.skin);
    let context = hero_build_context(hero_data, is_sub, source);
    let mut kit = resolve_entity_kit(&context)?;
    append_equipment_passives(game, equip_id, &mut kit.passives);

    // Destiny percentages are defined against the hero's pre-equipment base
    // attributes. Resolve against that snapshot, then add the result to the
    // final attribute set that already includes equipment bonuses.
    let destiny_base = base_hero_attributes(hero_data);
    let mut attr = build_attr(hero_data, equip_data.as_ref());
    apply_destiny_entity_attributes(&context, &mut attr, destiny_base);
    let current_hp = attr.hp.unwrap_or(0);

    let initial_ex_point = calculate_initial_ex_point(record.hero_id, &kit.passives);

    Ok(FightEntityInfo {
        uid: Some(record.uid),
        model_id: Some(record.hero_id),
        skin: Some(record.skin),
        position: Some(position),
        entity_type: Some(1),
        user_id: Some(record.user_id),
        ex_point: Some(initial_ex_point),
        level: Some(record.level),
        current_hp: Some(current_hp),
        attr: Some(attr),
        buffs: vec![],
        skill_group1: kit.skill_group_1,
        skill_group2: kit.skill_group_2,
        passive_skill: kit.passives,
        ex_skill: Some(kit.ultimate),

        shield_value: Some(0),
        no_effect_buffs: vec![],
        expoint_max_add: Some(0),
        buff_harm_statistic: Some(0),
        equip_uid: Some(record.default_equip_uid),
        trial_equip: Some(EquipRecord {
            equip_uid: Some(0),
            equip_id: Some(0),
            equip_lv: Some(0),
            refine_lv: Some(0),
        }),
        ex_skill_level: Some(record.ex_skill_level),
        power_infos: kit.power_infos,
        act104_equip_uids: vec![],
        trial_act104_equips: vec![],
        summoned_list: vec![],
        base_attr: Some(attr),
        ex_skill_point_change: Some(0),
        team_type: Some(team_type),
        enhance_info_box: Some(sonettobuf::EnhanceInfoBox {
            uid: Some(record.uid),
            can_upgrade_ids: vec![],
            upgraded_options: vec![],
        }),
        trial_id: Some(0),
        career: Some(get_hero_career(hero_data)),
        status: Some(0),
        guard: Some(-1),
        sub_cd: Some(0),
        ex_point_type: Some(detect_ex_point_type(record.hero_id)),
        equips: vec![sonettobuf::EquipRecord {
            equip_uid: equip_data.as_ref().map(|e| e.uid),
            equip_id: equip_data.as_ref().map(|e| e.equip_id),
            equip_lv: equip_data.as_ref().map(|e| e.level),
            refine_lv: equip_data.as_ref().map(|e| e.refine_lv),
        }],
        destiny_stone: Some(record.destiny_stone),
        destiny_rank: Some(record.destiny_rank),
        custom_unit_id: Some(0),
    })
}

fn activity_source_for_hero(hero_id: i32, skin: i32) -> HeroSource {
    let game = configs::get();
    if game
        .activity174_role
        .iter()
        .any(|role| role.hero_id == hero_id && role.skin_id == skin)
    {
        HeroSource::Activity
    } else {
        HeroSource::Owned
    }
}

fn apply_destiny_entity_attributes(
    context: &HeroBuildContext,
    attr: &mut HeroAttribute,
    base: HeroBaseAttributes,
) {
    if matches!(context.source, HeroSource::Trial)
        || context.destiny.rank == 0
        || context.destiny.level == 0
    {
        return;
    }
    let game = configs::get();
    let Ok(index) = config::destiny::DestinyConfigIndex::try_from_game_db(game) else {
        tracing::warn!(hero_id = context.hero_id, "Destiny index unavailable for entity attributes");
        return;
    };
    match resolve_destiny_attributes(&index, context.hero_id, context.destiny, base) {
        Ok(resolved) => {
            attr.hp = Some(attr.hp.unwrap_or(0).saturating_add(resolved.hp));
            attr.attack = Some(attr.attack.unwrap_or(0).saturating_add(resolved.attack));
            attr.defense = Some(attr.defense.unwrap_or(0).saturating_add(resolved.defense));
            attr.mdefense = Some(attr.mdefense.unwrap_or(0).saturating_add(resolved.mdefense));
        }
        Err(error) => tracing::warn!(
            hero_id = context.hero_id,
            rank = context.destiny.rank,
            level = context.destiny.level,
            error = %error,
            "inactive invalid Destiny attributes; preserving base entity stats"
        ),
    }
}

fn build_attr(r: &HeroData, equip: Option<&Equipment>) -> HeroAttribute {
    let base = base_hero_attributes(r);
    let mut hp = base.hp;
    let mut atk = base.attack;
    let mut def = base.defense;
    let mut mdef = base.mdefense;
    let technic = ((r.record.base_technic as f32) * 1.395604).round() as i32;

    if let Some(equip) = equip {
        let game_data = configs::get();

        if let Some(strengthen) = game_data
            .equip_strengthen
            .iter()
            .find(|s| s.strength_type == equip.equip_id)
        {
            hp += strengthen.hp;
            atk += strengthen.atk;
            def += strengthen.def;
            mdef += strengthen.mdef;
        }
    }

    HeroAttribute {
        hp: Some(hp),
        attack: Some(atk),
        defense: Some(def),
        mdefense: Some(mdef),
        technic: Some(technic),
        multi_hp_idx: Some(r.record.base_multi_hp_idx),
        multi_hp_num: Some(r.record.base_multi_hp_num),
    }
}

fn base_hero_attributes(r: &HeroData) -> HeroBaseAttributes {
    HeroBaseAttributes {
        hp: ((r.record.base_hp as f32) * 1.0986541).round() as i32,
        attack: ((r.record.base_attack as f32) * 1.0786).round() as i32,
        defense: ((r.record.base_defense as f32) * 1.0942857).round() as i32,
        mdefense: ((r.record.base_mdefense as f32) * 1.0942857).round() as i32,
    }
}

pub fn build_player_entity(user_id: i64, team_type: i32) -> FightEntityInfo {
    let uid = if team_type == 1 { 0 } else { -99999 };

    FightEntityInfo {
        uid: Some(uid),
        model_id: Some(0),
        skin: Some(0),
        position: Some(0),
        entity_type: Some(3),
        user_id: Some(user_id),
        ex_point: Some(0),
        level: Some(0),
        current_hp: Some(100),
        attr: Some(HeroAttribute {
            hp: Some(100),
            attack: Some(0),
            defense: Some(0),
            mdefense: Some(0),
            technic: Some(0),
            multi_hp_idx: Some(0),
            multi_hp_num: Some(0),
        }),
        buffs: vec![],
        skill_group1: vec![],
        skill_group2: vec![],
        passive_skill: vec![],
        ex_skill: Some(0),
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
        base_attr: None,
        ex_skill_point_change: Some(0),
        team_type: Some(team_type),
        enhance_info_box: Some(sonettobuf::EnhanceInfoBox {
            uid: Some(uid),
            can_upgrade_ids: vec![],
            upgraded_options: vec![],
        }),
        trial_id: Some(0),
        career: Some(0),
        status: Some(0),
        guard: Some(-1),
        sub_cd: Some(0),
        ex_point_type: Some(0),
        equips: vec![],
        destiny_stone: Some(0),
        destiny_rank: Some(0),
        custom_unit_id: Some(0),
    }
}

fn calculate_initial_ex_point(hero_id: i32, _passives: &[i32]) -> i32 {
    match hero_id {
        3088 => 0,
        3120 => 0,
        _ => 0,
    }
}

fn detect_ex_point_type(hero_id: i32) -> i32 {
    match hero_id {
        3120 => 1,
        3123 => 2,
        3124 | 3122 => 3,
        _ => 0,
    }
}

fn get_hero_career(hero_data: &HeroData) -> i32 {
    let game_data = configs::get();
    let hero_id = hero_data.record.hero_id;

    game_data
        .character
        .iter()
        .find(|s| s.id == hero_id)
        .map(|s| s.career)
        .unwrap_or(0)
}
