#![allow(dead_code)]

use std::collections::BTreeMap;

use config::destiny::DestinyConfigIndex;

use super::types::{
    DestinyResolveError, DestinyState, DestinyTrace, FixedTenths, HeroBaseAttributes,
    ResolvedDestinyAttributes,
};

const DIRECT_HP: i32 = 601;
const DIRECT_ATTACK: i32 = 602;
const DIRECT_DEFENSE: i32 = 603;
const DIRECT_MDEFENSE: i32 = 604;
const PERCENT_HP: i32 = 605;
const PERCENT_ATTACK: i32 = 606;
const PERCENT_DEFENSE: i32 = 607;
const PERCENT_MDEFENSE: i32 = 608;

pub fn resolve_destiny_attributes(
    index: &DestinyConfigIndex,
    hero_id: i32,
    state: DestinyState,
    base: HeroBaseAttributes,
) -> Result<ResolvedDestinyAttributes, DestinyResolveError> {
    if state.rank == 0 && state.level == 0 {
        return Ok(ResolvedDestinyAttributes::default());
    }
    if state.rank <= 0 || state.level <= 0 {
        return Err(DestinyResolveError::InvalidState(format!(
            "rank {} level {}",
            state.rank, state.level
        )));
    }

    let hero = index
        .hero(hero_id)
        .ok_or_else(|| DestinyResolveError::InvalidConfig(format!("missing hero {hero_id}")))?;

    let mut raw_tenths = BTreeMap::new();
    let mut trace = Vec::new();

    for rank in 1..state.rank {
        let mut node = 1;
        let mut included_any = false;
        while let Some(slot) = index.slot(hero.slots_id, rank, node) {
            include_slot(
                index,
                hero.slots_id,
                rank,
                node,
                &slot.effect,
                &mut raw_tenths,
            )?;
            trace.push(DestinyTrace {
                source_key: format!(
                    "character_destiny_slots:{}:{}:{}",
                    hero.slots_id, rank, node
                ),
                detail: slot.effect.clone(),
            });
            included_any = true;
            node += 1;
        }
        if !included_any {
            return Err(DestinyResolveError::InvalidState(format!(
                "missing completed Destiny stage {rank} for slot group {}",
                hero.slots_id
            )));
        }
    }

    for node in 1..=state.level {
        let slot = index.slot(hero.slots_id, state.rank, node).ok_or_else(|| {
            DestinyResolveError::InvalidState(format!(
                "missing Destiny slot ({}, {}, {node})",
                hero.slots_id, state.rank
            ))
        })?;
        include_slot(
            index,
            hero.slots_id,
            state.rank,
            node,
            &slot.effect,
            &mut raw_tenths,
        )?;
        trace.push(DestinyTrace {
            source_key: format!(
                "character_destiny_slots:{}:{}:{}",
                hero.slots_id, state.rank, node
            ),
            detail: slot.effect.clone(),
        });
    }

    let hp = raw_value(&raw_tenths, DIRECT_HP)?
        .checked_add(percent_value(&raw_tenths, PERCENT_HP, base.hp)?)
        .ok_or(DestinyResolveError::Overflow)?;
    let attack = raw_value(&raw_tenths, DIRECT_ATTACK)?
        .checked_add(percent_value(&raw_tenths, PERCENT_ATTACK, base.attack)?)
        .ok_or(DestinyResolveError::Overflow)?;
    let defense = raw_value(&raw_tenths, DIRECT_DEFENSE)?
        .checked_add(percent_value(&raw_tenths, PERCENT_DEFENSE, base.defense)?)
        .ok_or(DestinyResolveError::Overflow)?;
    let mdefense = raw_value(&raw_tenths, DIRECT_MDEFENSE)?
        .checked_add(percent_value(&raw_tenths, PERCENT_MDEFENSE, base.mdefense)?)
        .ok_or(DestinyResolveError::Overflow)?;

    Ok(ResolvedDestinyAttributes {
        hp,
        attack,
        defense,
        mdefense,
        raw_tenths,
        trace,
    })
}

pub fn is_supported_destiny_attribute(attribute_id: i32) -> bool {
    matches!(
        attribute_id,
        201 | 203
            | 205
            | 211
            | 212
            | 214
            | 217
            | 218
            | 219
            | 220
            | 301
            | 601
            | 602
            | 603
            | 604
            | 605
            | 606
            | 607
            | 608
    )
}

fn include_slot(
    index: &DestinyConfigIndex,
    slots_id: i32,
    rank: i32,
    node: i32,
    effect: &str,
    raw_tenths: &mut BTreeMap<i32, i64>,
) -> Result<(), DestinyResolveError> {
    for entry in effect.split('|').filter(|entry| !entry.is_empty()) {
        let (id, raw) = parse_effect_entry(entry)?;
        validate_attribute_adapter(index, id, slots_id, rank, node)?;
        let total = raw_tenths.entry(id).or_insert(0_i64);
        *total = total
            .checked_add(raw)
            .ok_or(DestinyResolveError::Overflow)?;
    }
    Ok(())
}

fn parse_effect_entry(entry: &str) -> Result<(i32, i64), DestinyResolveError> {
    let mut parts = entry.split('#');
    let id = parts
        .next()
        .and_then(|value| value.parse::<i32>().ok())
        .ok_or_else(|| DestinyResolveError::InvalidConfig(format!("invalid effect {entry:?}")))?;
    let raw = parts
        .next()
        .and_then(|value| value.parse::<i64>().ok())
        .ok_or_else(|| DestinyResolveError::InvalidConfig(format!("invalid effect {entry:?}")))?;
    if parts.next().is_some() {
        return Err(DestinyResolveError::InvalidConfig(format!(
            "invalid effect {entry:?}"
        )));
    }
    Ok((id, raw))
}

fn validate_attribute_adapter(
    index: &DestinyConfigIndex,
    id: i32,
    slots_id: i32,
    rank: i32,
    node: i32,
) -> Result<(), DestinyResolveError> {
    if !is_supported_destiny_attribute(id) {
        return Err(DestinyResolveError::InvalidConfig(format!(
            "unsupported Destiny attribute {id} at character_destiny_slots:{slots_id}:{rank}:{node}"
        )));
    }
    let attribute = index.attribute(id).ok_or_else(|| {
        DestinyResolveError::InvalidConfig(format!(
            "missing character_attribute {id} for character_destiny_slots:{slots_id}:{rank}:{node}"
        ))
    })?;
    let expected_show_type = if (DIRECT_HP..=DIRECT_MDEFENSE).contains(&id) {
        0
    } else {
        1
    };
    if attribute.show_type != expected_show_type {
        return Err(DestinyResolveError::InvalidConfig(format!(
            "attribute {id} has show_type {}, expected {expected_show_type}",
            attribute.show_type
        )));
    }
    Ok(())
}

fn raw_value(raw_tenths: &BTreeMap<i32, i64>, id: i32) -> Result<i32, DestinyResolveError> {
    i32::try_from(*raw_tenths.get(&id).unwrap_or(&0)).map_err(|_| DestinyResolveError::Overflow)
}

fn percent_value(
    raw_tenths: &BTreeMap<i32, i64>,
    id: i32,
    base: i32,
) -> Result<i32, DestinyResolveError> {
    FixedTenths(*raw_tenths.get(&id).unwrap_or(&0)).floor_percent_of(base)
}
