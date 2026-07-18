#![allow(dead_code)]

use config::{GameDB, destiny::DestinyConfigIndex};
use sonettobuf::PowerInfo;

use super::types::{DestinyResolveError, DestinyTrace, HeroBuildContext};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SkillExchange {
    pub source: i32,
    pub target: i32,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ResolvedHeroKit {
    pub skill_group_1: Vec<i32>,
    pub skill_group_2: Vec<i32>,
    pub ultimate: i32,
    pub passives: Vec<i32>,
    pub power_infos: Vec<PowerInfo>,
    pub trace: Vec<DestinyTrace>,
}

pub fn parse_skill_exchanges(value: &str) -> Result<Vec<SkillExchange>, DestinyResolveError> {
    if value.is_empty() {
        return Ok(Vec::new());
    }
    let entries = value.strip_suffix('|').unwrap_or(value);

    entries
        .split('|')
        .map(|entry| {
            let mut parts = entry.split('#');
            let source = parse_i32(parts.next(), "exchange source", value)?;
            let target = parse_i32(parts.next(), "exchange target", value)?;
            if parts.next().is_some() {
                return Err(DestinyResolveError::InvalidConfig(format!(
                    "invalid skill exchange {entry:?} in {value:?}"
                )));
            }
            if source <= 0 || target <= 0 {
                return Err(DestinyResolveError::InvalidConfig(format!(
                    "skill exchange ids must be positive in {entry:?} from {value:?}"
                )));
            }
            Ok(SkillExchange { source, target })
        })
        .collect()
}

pub fn resolve_hero_kit(
    index: &DestinyConfigIndex,
    game: &GameDB,
    context: &HeroBuildContext,
) -> Result<ResolvedHeroKit, DestinyResolveError> {
    if context.ex_skill_level < 0 {
        return Err(DestinyResolveError::InvalidState(format!(
            "negative ex skill level {} for hero {}",
            context.ex_skill_level, context.hero_id
        )));
    }
    if context.destiny.rank < 0 || context.destiny.facet_id < 0 {
        return Err(DestinyResolveError::InvalidState(format!(
            "rank {} facet {} for hero {}",
            context.destiny.rank, context.destiny.facet_id, context.hero_id
        )));
    }

    let character = game.character.get(context.hero_id).ok_or_else(|| {
        DestinyResolveError::InvalidConfig(format!("missing character {}", context.hero_id))
    })?;

    let rank_replacement = active_rank_replacement(game, character, context);
    let (skill, ultimate, effective_hero_type, replacement_source) = rank_replacement
        .map(|replacement| {
            (
                replacement.skill.as_str(),
                replacement.ex_skill,
                replacement.hero_type,
                Some(format!("character_rank_replace:{}", replacement.id)),
            )
        })
        .unwrap_or((&character.skill, character.ex_skill, context.hero_type, None));

    let mut resolved = ResolvedHeroKit {
        skill_group_1: parse_character_skill_group(skill, 1, effective_hero_type)?,
        skill_group_2: parse_character_skill_group(skill, 2, effective_hero_type)?,
        ultimate,
        passives: base_passives(game, context.hero_id),
        power_infos: Vec::new(),
        trace: Vec::new(),
    };

    if let Some(source_key) = replacement_source {
        resolved.trace.push(DestinyTrace {
            source_key,
            detail: format!(
                "rank replacement active at rank {} skin {} hero type {}",
                context.rank, context.skin, effective_hero_type
            ),
        });
    }

    let active_facet = active_facet(index, context, &mut resolved.trace)?;
    let reshape_active = active_facet.is_some_and(|facet| facet.ex_level_exchange);

    if reshape_active {
        if context.ex_skill_level > 0 {
            let facet_id = context.destiny.facet_id;
            let row = index
                .reshape(facet_id, context.ex_skill_level)
                .ok_or_else(|| {
                    DestinyResolveError::InvalidConfig(format!(
                        "missing destiny_facets_ex_level ({facet_id}, {})",
                        context.ex_skill_level
                    ))
                })?;
            let source_key = format!(
                "destiny_facets_ex_level:{facet_id}:{}",
                context.ex_skill_level
            );
            replace_group_from_value(
                &mut resolved.skill_group_1,
                &row.skill_group1,
                effective_hero_type,
                "skill_group_1",
                &source_key,
                &mut resolved.trace,
            )?;
            replace_group_from_value(
                &mut resolved.skill_group_2,
                &row.skill_group2,
                effective_hero_type,
                "skill_group_2",
                &source_key,
                &mut resolved.trace,
            )?;
            replace_ultimate(
                &mut resolved.ultimate,
                row.skill_ex,
                &source_key,
                &mut resolved.trace,
            );
        }
    } else {
        for level in 1..=context.ex_skill_level {
            let Some(row) = game
                .skill_ex_level
                .iter()
                .find(|row| row.hero_id == context.hero_id && row.skill_level == level)
            else {
                continue;
            };
            let source_key = format!("skill_ex_level:{}:{level}", context.hero_id);
            replace_group_from_value(
                &mut resolved.skill_group_1,
                &row.skill_group1,
                effective_hero_type,
                "skill_group_1",
                &source_key,
                &mut resolved.trace,
            )?;
            replace_group_from_value(
                &mut resolved.skill_group_2,
                &row.skill_group2,
                effective_hero_type,
                "skill_group_2",
                &source_key,
                &mut resolved.trace,
            )?;
            replace_ultimate(
                &mut resolved.ultimate,
                row.skill_ex,
                &source_key,
                &mut resolved.trace,
            );
        }
    }

    for level in (1..=context.ex_skill_level).rev() {
        let Some(row) = game
            .skill_ex_level
            .iter()
            .find(|row| row.hero_id == context.hero_id && row.skill_level == level)
        else {
            continue;
        };
        let exchanges = parse_skill_exchanges(&row.passive_skill)?;
        apply_ordered_transitions(
            &mut resolved.passives,
            &exchanges,
            "passive",
            &format!("skill_ex_level:{}:{level}", context.hero_id),
            &mut resolved.trace,
        );
    }

    if let Some(facet) = active_facet {
        let source_key = format!(
            "character_destiny_facets:{}:{}",
            facet.facets_id, facet.level
        );
        let exchanges = parse_skill_exchanges(&facet.exchange_skills)?;
        apply_facet_transitions(
            &mut resolved.skill_group_1,
            &exchanges,
            "skill_group_1",
            &source_key,
            &mut resolved.trace,
        );
        apply_facet_transitions(
            &mut resolved.skill_group_2,
            &exchanges,
            "skill_group_2",
            &source_key,
            &mut resolved.trace,
        );
        apply_facet_scalar_transition(
            &mut resolved.ultimate,
            &exchanges,
            "ultimate",
            &source_key,
            &mut resolved.trace,
        );

        // Facet exchanges inspect each existing passive independently; mappings in one row
        // therefore do not chain through another mapping from that same row.
        apply_facet_transitions(
            &mut resolved.passives,
            &exchanges,
            "passive",
            &source_key,
            &mut resolved.trace,
        );

        for power in parse_skill_exchanges(&facet.power_add)? {
            resolved.power_infos.push(PowerInfo {
                power_id: Some(power.source),
                num: Some(0),
                max: Some(power.target),
            });
            resolved.trace.push(DestinyTrace {
                source_key: source_key.clone(),
                detail: format!("power {} max {}", power.source, power.target),
            });
        }

        if facet.ex_level_exchange {
            for level in 1..=context.ex_skill_level {
                let Some(row) = index.reshape(facet.facets_id, level) else {
                    return Err(DestinyResolveError::InvalidConfig(format!(
                        "missing destiny_facets_ex_level ({}, {level})",
                        facet.facets_id
                    )));
                };
                let exchanges = parse_skill_exchanges(&row.exchange_skill)?;
                apply_ordered_transitions(
                    &mut resolved.passives,
                    &exchanges,
                    "passive",
                    &format!("destiny_facets_ex_level:{}:{level}", facet.facets_id),
                    &mut resolved.trace,
                );
            }
        }
    }

    Ok(resolved)
}

fn active_rank_replacement<'a>(
    game: &'a GameDB,
    character: &config::character::Character,
    context: &HeroBuildContext,
) -> Option<&'a config::character_rank_replace::CharacterRankReplace> {
    // character_limited is not loaded into GameDB. In the checked-in 3.6 runtime, the only
    // replacement hero's base and mv skins both declare specialLive2d "1#3", making this
    // rank >= 3 and base/mv skin check equivalent for that data set.
    if context.rank < 3
        || context.skin <= 0
        || (context.skin != character.skin_id && context.skin != character.mvskin_id)
    {
        return None;
    }
    game.character_rank_replace.get(context.hero_id)
}

fn active_facet<'a>(
    index: &'a DestinyConfigIndex,
    context: &HeroBuildContext,
    trace: &mut Vec<DestinyTrace>,
) -> Result<Option<&'a config::character_destiny_facets::CharacterDestinyFacets>, DestinyResolveError>
{
    let facet_id = context.destiny.facet_id;
    if facet_id == 0 {
        trace.push(DestinyTrace {
            source_key: format!("character_destiny_facets:0:{}", context.destiny.rank),
            detail: "facet 0 inactive".to_string(),
        });
        return Ok(None);
    }
    if context.destiny.rank == 0 {
        trace.push(DestinyTrace {
            source_key: format!("character_destiny_facets:{facet_id}:0"),
            detail: format!("facet {facet_id} inactive at rank 0"),
        });
        return Ok(None);
    }

    let is_owned = index
        .hero(context.hero_id)
        .is_some_and(|hero| hero.facet_ids.contains(&facet_id));
    if !is_owned {
        trace.push(DestinyTrace {
            source_key: format!("character_destiny:{}", context.hero_id),
            detail: format!("foreign facet {facet_id} inactive"),
        });
        return Ok(None);
    }

    let facet = index.facet(facet_id, context.destiny.rank).ok_or_else(|| {
        DestinyResolveError::InvalidConfig(format!(
            "missing character_destiny_facets ({facet_id}, {})",
            context.destiny.rank
        ))
    })?;
    trace.push(DestinyTrace {
        source_key: format!(
            "character_destiny_facets:{facet_id}:{}",
            context.destiny.rank
        ),
        detail: format!("facet {facet_id} active at rank {}", context.destiny.rank),
    });
    Ok(Some(facet))
}

fn base_passives(game: &GameDB, hero_id: i32) -> Vec<i32> {
    let mut rows = game
        .skill_passive_level
        .iter()
        .filter(|row| row.hero_id == hero_id && row.skill_passive != 0)
        .collect::<Vec<_>>();
    rows.sort_by_key(|row| {
        if row.skill_level == 0 {
            i32::MAX
        } else {
            row.skill_level
        }
    });
    rows.into_iter().map(|row| row.skill_passive).collect()
}

fn parse_character_skill_group(
    value: &str,
    target_group: i32,
    hero_type: i32,
) -> Result<Vec<i32>, DestinyResolveError> {
    if value.is_empty() {
        return Ok(Vec::new());
    }

    for block in value.split('|') {
        let first_variant = block.split(',').next().unwrap_or_default();
        let mut first_parts = first_variant.split('#');
        let group = parse_i32(first_parts.next(), "skill group", value)?;
        if group != target_group {
            continue;
        }

        let variants = block.split(',').collect::<Vec<_>>();
        let selected = variants[variant_index(hero_type, variants.len())];
        let mut parts = selected.split('#').peekable();
        if parts.peek().and_then(|part| part.parse::<i32>().ok()) == Some(target_group) {
            parts.next();
        }
        return parts
            .map(|part| parse_i32(Some(part), "skill id", value))
            .collect();
    }

    Ok(Vec::new())
}

fn parse_skill_group_value(value: &str, hero_type: i32) -> Result<Vec<i32>, DestinyResolveError> {
    if value.is_empty() {
        return Ok(Vec::new());
    }
    let variants = value.split(',').collect::<Vec<_>>();
    variants[variant_index(hero_type, variants.len())]
        .split('|')
        .map(|part| parse_i32(Some(part), "skill id", value))
        .collect()
}

fn variant_index(hero_type: i32, variant_count: usize) -> usize {
    usize::try_from(hero_type - 1)
        .ok()
        .filter(|index| *index < variant_count)
        .unwrap_or(0)
}

fn parse_i32(value: Option<&str>, field: &str, source: &str) -> Result<i32, DestinyResolveError> {
    value
        .and_then(|value| value.parse::<i32>().ok())
        .ok_or_else(|| DestinyResolveError::InvalidConfig(format!("invalid {field} in {source:?}")))
}

fn replace_group_from_value(
    current: &mut Vec<i32>,
    value: &str,
    hero_type: i32,
    label: &str,
    source_key: &str,
    trace: &mut Vec<DestinyTrace>,
) -> Result<(), DestinyResolveError> {
    if value.is_empty() {
        return Ok(());
    }
    let replacement = parse_skill_group_value(value, hero_type)?;
    for (source, target) in current.iter().zip(&replacement) {
        if source != target {
            trace.push(DestinyTrace {
                source_key: source_key.to_string(),
                detail: format!("{label} {source} -> {target}"),
            });
        }
    }
    if current.len() != replacement.len() {
        trace.push(DestinyTrace {
            source_key: source_key.to_string(),
            detail: format!("{label} {current:?} -> {replacement:?}"),
        });
    }
    *current = replacement;
    Ok(())
}

fn replace_ultimate(
    current: &mut i32,
    replacement: i32,
    source_key: &str,
    trace: &mut Vec<DestinyTrace>,
) {
    if replacement == 0 {
        return;
    }
    if *current != replacement {
        trace.push(DestinyTrace {
            source_key: source_key.to_string(),
            detail: format!("ultimate {} -> {replacement}", *current),
        });
    }
    *current = replacement;
}

fn apply_ordered_transitions(
    values: &mut [i32],
    exchanges: &[SkillExchange],
    label: &str,
    source_key: &str,
    trace: &mut Vec<DestinyTrace>,
) {
    for exchange in exchanges {
        let Some(value) = values.iter_mut().find(|value| **value == exchange.source) else {
            continue;
        };
        let source = *value;
        *value = exchange.target;
        trace.push(DestinyTrace {
            source_key: source_key.to_string(),
            detail: format!("{label} {source} -> {}", exchange.target),
        });
    }
}

fn apply_facet_transitions(
    values: &mut [i32],
    exchanges: &[SkillExchange],
    label: &str,
    source_key: &str,
    trace: &mut Vec<DestinyTrace>,
) {
    for value in values {
        let original = *value;
        for exchange in exchanges {
            if original != exchange.source {
                continue;
            }
            *value = exchange.target;
            trace.push(DestinyTrace {
                source_key: source_key.to_string(),
                detail: format!("{label} {} -> {}", exchange.source, exchange.target),
            });
        }
    }
}

fn apply_facet_scalar_transition(
    value: &mut i32,
    exchanges: &[SkillExchange],
    label: &str,
    source_key: &str,
    trace: &mut Vec<DestinyTrace>,
) {
    let original = *value;
    for exchange in exchanges {
        if original != exchange.source {
            continue;
        }
        *value = exchange.target;
        trace.push(DestinyTrace {
            source_key: source_key.to_string(),
            detail: format!("{label} {} -> {}", exchange.source, exchange.target),
        });
    }
}
