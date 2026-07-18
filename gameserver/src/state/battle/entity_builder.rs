use config::configs;
use database::{
    db::game::equipment::Equipment,
    models::game::{equipment::UserEquipmentModel, heros::HeroData},
};
use anyhow::Result;
use super::destiny::{
    DestinyResolveError, DestinyState, HeroBaseAttributes, HeroBuildContext, HeroSource,
    ResolvedDestinyAttributes, ResolvedHeroKit, resolve_destiny_attributes, resolve_hero_kit,
};
use sonettobuf::{EquipRecord, FightEntityInfo, HeroAttribute, HeroExAttribute, HeroSpAttribute};
use sqlx::SqlitePool;

/// Resolve the complete combat kit for every hero source. This is the only
/// entry point entity construction should use for skills/passives/ultimate.
pub fn resolve_entity_kit(
    context: &HeroBuildContext,
) -> Result<ResolvedHeroKit, DestinyResolveError> {
    let game = configs::get();
    let index = config::destiny::DestinyConfigIndex::try_from_game_db(game)
        .map_err(|error| DestinyResolveError::InvalidConfig(error.to_string()))?;
    validate_ex_skill_level(game, context)?;

    let (effective_context, facet_active) = effective_destiny_context(&index, context);
    if context.destiny.facet_id > 0 && context.destiny.rank > 0 && !facet_active {
        tracing::warn!(
            hero_id = context.hero_id,
            facet_id = context.destiny.facet_id,
            rank = context.destiny.rank,
            "inactive invalid Destiny facet; preserving legacy hero construction"
        );
    }

    let mut kit = resolve_hero_kit(&index, game, &effective_context)?;
    apply_compatibility_passives(context, facet_active, &mut kit);
    Ok(kit)
}

fn effective_destiny_context(
    index: &config::destiny::DestinyConfigIndex,
    context: &HeroBuildContext,
) -> (HeroBuildContext, bool) {
    let facet_id = context.destiny.facet_id;
    let facet_rank = context.destiny.rank;
    let facet_requested = facet_id > 0 && facet_rank > 0;
    let facet_owned = index
        .hero(context.hero_id)
        .is_some_and(|hero| hero.facet_ids.contains(&facet_id));
    let facet_rank_exists = index.facet(facet_id, facet_rank).is_some();
    let facet_active = facet_requested && facet_owned && facet_rank_exists;

    let mut effective_context = context.clone();
    if facet_requested && !facet_active {
        effective_context.destiny = DestinyState::default();
    }
    (effective_context, facet_active)
}

fn validate_ex_skill_level(
    game: &config::GameDB,
    context: &HeroBuildContext,
) -> Result<(), DestinyResolveError> {
    if context.ex_skill_level == 0 {
        return Ok(());
    }

    let max_level = game
        .skill_ex_level
        .iter()
        .filter(|row| row.hero_id == context.hero_id)
        .map(|row| row.skill_level)
        .max()
        .unwrap_or(0);
    if context.ex_skill_level > max_level {
        return Err(DestinyResolveError::InvalidState(format!(
            "ex skill level {} exceeds configured maximum {} for hero {}",
            context.ex_skill_level, max_level, context.hero_id
        )));
    }
    Ok(())
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

fn apply_compatibility_passives(
    context: &HeroBuildContext,
    destiny_facet_active: bool,
    kit: &mut ResolvedHeroKit,
) {
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
    let context = hero_build_context(hero_data, is_sub, HeroSource::Owned);
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

pub fn resolve_entity_destiny_attributes(
    context: &HeroBuildContext,
    base: HeroBaseAttributes,
) -> Result<ResolvedDestinyAttributes, DestinyResolveError> {
    let game = configs::get();
    let index = config::destiny::DestinyConfigIndex::try_from_game_db(game)
        .map_err(|error| DestinyResolveError::InvalidConfig(error.to_string()))?;
    validate_ex_skill_level(game, context)?;
    let (effective_context, _) = effective_destiny_context(&index, context);
    resolve_destiny_attributes(&index, effective_context.hero_id, effective_context.destiny, base)
}

pub fn resolve_hero_destiny_attributes(
    hero_data: &HeroData,
    is_substitute: bool,
) -> Result<ResolvedDestinyAttributes, DestinyResolveError> {
    let context = hero_build_context(hero_data, is_substitute, HeroSource::Owned);
    resolve_entity_destiny_attributes(&context, base_hero_attributes(hero_data))
}

pub(crate) fn merge_hero_combat_attributes(
    mut resolved: ResolvedDestinyAttributes,
    hero_data: &HeroData,
) -> ResolvedDestinyAttributes {
    let record = &hero_data.record;
    resolved.ex_attr = add_ex_attributes(
        HeroExAttribute {
            cri: Some(record.ex_cri),
            recri: Some(record.ex_recri),
            cri_dmg: Some(record.ex_cri_dmg),
            cri_def: Some(record.ex_cri_def),
            add_dmg: Some(record.ex_add_dmg),
            drop_dmg: Some(record.ex_drop_dmg),
        },
        resolved.ex_attr,
    );
    let base_sp = hero_data
        .sp_attr
        .clone()
        .map(Into::into)
        .unwrap_or_default();
    resolved.sp_attr = add_sp_attributes(base_sp, resolved.sp_attr);
    resolved
}

fn add_ex_attributes(base: HeroExAttribute, delta: HeroExAttribute) -> HeroExAttribute {
    HeroExAttribute {
        cri: Some(base.cri.unwrap_or(0) + delta.cri.unwrap_or(0)),
        recri: Some(base.recri.unwrap_or(0) + delta.recri.unwrap_or(0)),
        cri_dmg: Some(base.cri_dmg.unwrap_or(0) + delta.cri_dmg.unwrap_or(0)),
        cri_def: Some(base.cri_def.unwrap_or(0) + delta.cri_def.unwrap_or(0)),
        add_dmg: Some(base.add_dmg.unwrap_or(0) + delta.add_dmg.unwrap_or(0)),
        drop_dmg: Some(base.drop_dmg.unwrap_or(0) + delta.drop_dmg.unwrap_or(0)),
    }
}

fn add_sp_attributes(base: HeroSpAttribute, delta: HeroSpAttribute) -> HeroSpAttribute {
    macro_rules! add {
        ($field:ident) => {
            Some(base.$field.unwrap_or(0) + delta.$field.unwrap_or(0))
        };
    }
    HeroSpAttribute {
        revive: add!(revive),
        heal: add!(heal),
        absorb: add!(absorb),
        defense_ignore: add!(defense_ignore),
        clutch: add!(clutch),
        final_add_dmg: add!(final_add_dmg),
        final_drop_dmg: add!(final_drop_dmg),
        normal_skill_rate: add!(normal_skill_rate),
        play_add_rate: add!(play_add_rate),
        play_drop_rate: add!(play_drop_rate),
        dizzy_resistances: add!(dizzy_resistances),
        sleep_resistances: add!(sleep_resistances),
        petrified_resistances: add!(petrified_resistances),
        frozen_resistances: add!(frozen_resistances),
        disarm_resistances: add!(disarm_resistances),
        forbid_resistances: add!(forbid_resistances),
        seal_resistances: add!(seal_resistances),
        cant_get_exskill_resistances: add!(cant_get_exskill_resistances),
        del_ex_point_resistances: add!(del_ex_point_resistances),
        stress_up_resistances: add!(stress_up_resistances),
        control_resilience: add!(control_resilience),
        del_ex_point_resilience: add!(del_ex_point_resilience),
        stress_up_resilience: add!(stress_up_resilience),
        charm_resistances: add!(charm_resistances),
        rebound_dmg: add!(rebound_dmg),
        extra_dmg: add!(extra_dmg),
        reuse_dmg: add!(reuse_dmg),
        big_skill_rate: add!(big_skill_rate),
        clutch_dmg: add!(clutch_dmg),
    }
}

fn apply_destiny_entity_attributes(
    context: &HeroBuildContext,
    attr: &mut HeroAttribute,
    base: HeroBaseAttributes,
) {
    if context.destiny.rank == 0 || context.destiny.level == 0 {
        return;
    }
    match resolve_entity_destiny_attributes(context, base) {
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
