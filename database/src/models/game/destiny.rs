use std::{collections::BTreeMap, error::Error, fmt};

use config::destiny::DestinyConfigIndex;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DestinyState {
    pub rank: i32,
    pub level: i32,
    pub stone: i32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[repr(i32)]
pub enum MaterialKind {
    Item = 1,
    Currency = 2,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct MaterialCost {
    pub kind: MaterialKind,
    pub id: i32,
    pub amount: i32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DestinyCommand {
    RankUp { hero_id: i32 },
    LevelUp { hero_id: i32, target_level: i32 },
    UnlockStone { hero_id: i32, stone_id: i32 },
    UseStone { hero_id: i32, stone_id: i32 },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnedDestinyHero {
    pub hero_uid: i64,
    pub user_id: i64,
    pub hero_id: i32,
    pub state: DestinyState,
    pub unlocked_stones: Vec<i32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MutationKind {
    Progress { target_rank: i32, target_level: i32 },
    UnlockStone { stone_id: i32 },
    UseStone { stone_id: i32 },
    NoChange,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MutationPlan {
    pub expected: DestinyState,
    pub kind: MutationKind,
    pub costs: Vec<MaterialCost>,
}

#[derive(Debug)]
pub enum ProgressionError {
    Invalid(String),
    Insufficient(MaterialCost),
    Conflict,
    Database(sqlx::Error),
}

impl fmt::Display for ProgressionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => write!(formatter, "invalid Destiny transition: {message}"),
            Self::Insufficient(cost) => write!(
                formatter,
                "insufficient {:?} {} (required {})",
                cost.kind, cost.id, cost.amount
            ),
            Self::Conflict => formatter.write_str("Destiny state changed concurrently"),
            Self::Database(error) => write!(formatter, "Destiny database error: {error}"),
        }
    }
}

impl Error for ProgressionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Database(error) => Some(error),
            _ => None,
        }
    }
}

impl From<sqlx::Error> for ProgressionError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

pub fn parse_material_costs(raw_values: &[&str]) -> Result<Vec<MaterialCost>, ProgressionError> {
    let mut totals = BTreeMap::<(MaterialKind, i32), i64>::new();

    for raw in raw_values {
        if raw.is_empty() {
            continue;
        }

        for entry in raw.split('|') {
            let mut fields = entry.split('#');
            let kind = parse_kind(fields.next(), entry)?;
            let id = parse_id(fields.next(), entry)?;
            let amount = parse_amount(fields.next(), entry)?;
            if fields.next().is_some() {
                return invalid(format!("malformed material cost {entry:?}"));
            }

            let total = totals.entry((kind, id)).or_default();
            *total = total
                .checked_add(amount)
                .ok_or_else(|| ProgressionError::Invalid("material total overflow".to_owned()))?;
        }
    }

    totals
        .into_iter()
        .map(|((kind, id), amount)| {
            let amount = i32::try_from(amount).map_err(|_| {
                ProgressionError::Invalid(format!("material total for {:?} {id} exceeds i32", kind))
            })?;
            Ok(MaterialCost { kind, id, amount })
        })
        .collect()
}

pub fn plan_transition(
    catalog: &DestinyConfigIndex,
    current: &OwnedDestinyHero,
    command: DestinyCommand,
) -> Result<MutationPlan, ProgressionError> {
    let hero = catalog.hero(current.hero_id).ok_or_else(|| {
        ProgressionError::Invalid(format!(
            "hero {} has no Destiny configuration",
            current.hero_id
        ))
    })?;

    let command_hero_id = match command {
        DestinyCommand::RankUp { hero_id }
        | DestinyCommand::LevelUp { hero_id, .. }
        | DestinyCommand::UnlockStone { hero_id, .. }
        | DestinyCommand::UseStone { hero_id, .. } => hero_id,
    };
    if command_hero_id != current.hero_id {
        return invalid(format!(
            "command hero {command_hero_id} does not match owned hero {}",
            current.hero_id
        ));
    }

    match command {
        DestinyCommand::RankUp { .. } => plan_rank_up(catalog, hero.slots_id, current),
        DestinyCommand::LevelUp { target_level, .. } => {
            plan_level_up(catalog, hero.slots_id, current, target_level)
        }
        DestinyCommand::UnlockStone { stone_id, .. } => {
            plan_stone_unlock(catalog, &hero.facet_ids, current, stone_id)
        }
        DestinyCommand::UseStone { stone_id, .. } => {
            plan_stone_use(&hero.facet_ids, current, stone_id)
        }
    }
}

fn plan_rank_up(
    catalog: &DestinyConfigIndex,
    slots_id: i32,
    current: &OwnedDestinyHero,
) -> Result<MutationPlan, ProgressionError> {
    let target_rank = if current.state.rank == 0 && current.state.level == 0 {
        1
    } else {
        validate_progress_state(catalog, slots_id, current.state)?;
        let next_level = current
            .state
            .level
            .checked_add(1)
            .ok_or_else(|| ProgressionError::Invalid("Destiny level overflow".to_owned()))?;
        if catalog
            .slot(slots_id, current.state.rank, next_level)
            .is_some()
        {
            return invalid("current Destiny stage is not complete");
        }
        current
            .state
            .rank
            .checked_add(1)
            .ok_or_else(|| ProgressionError::Invalid("Destiny rank overflow".to_owned()))?
    };

    let slot = catalog.slot(slots_id, target_rank, 1).ok_or_else(|| {
        ProgressionError::Invalid(format!(
            "missing Destiny slot ({slots_id}, {target_rank}, 1)"
        ))
    })?;
    let costs = parse_material_costs(&[slot.consume.as_str()])?;

    Ok(MutationPlan {
        expected: current.state,
        kind: MutationKind::Progress {
            target_rank,
            target_level: 1,
        },
        costs,
    })
}

fn plan_level_up(
    catalog: &DestinyConfigIndex,
    slots_id: i32,
    current: &OwnedDestinyHero,
    target_level: i32,
) -> Result<MutationPlan, ProgressionError> {
    validate_progress_state(catalog, slots_id, current.state)?;
    if target_level < current.state.level {
        return invalid(format!(
            "target level {target_level} is below current level {}",
            current.state.level
        ));
    }
    if target_level == current.state.level {
        return Ok(no_change(current.state));
    }

    let first_node = current
        .state
        .level
        .checked_add(1)
        .ok_or_else(|| ProgressionError::Invalid("Destiny level overflow".to_owned()))?;
    let mut raw_costs = Vec::new();
    for node in first_node..=target_level {
        let slot = catalog
            .slot(slots_id, current.state.rank, node)
            .ok_or_else(|| {
                ProgressionError::Invalid(format!(
                    "missing Destiny slot ({slots_id}, {}, {node})",
                    current.state.rank
                ))
            })?;
        raw_costs.push(slot.consume.as_str());
    }

    Ok(MutationPlan {
        expected: current.state,
        kind: MutationKind::Progress {
            target_rank: current.state.rank,
            target_level,
        },
        costs: parse_material_costs(&raw_costs)?,
    })
}

fn plan_stone_unlock(
    catalog: &DestinyConfigIndex,
    facet_ids: &[i32],
    current: &OwnedDestinyHero,
    stone_id: i32,
) -> Result<MutationPlan, ProgressionError> {
    if current.state.rank <= 0 {
        return invalid("a Destiny rank is required before unlocking a stone");
    }
    if !facet_ids.contains(&stone_id) {
        return invalid(format!(
            "stone {stone_id} is not owned by hero {}",
            current.hero_id
        ));
    }
    let consume = catalog.facet_consume(stone_id).ok_or_else(|| {
        ProgressionError::Invalid(format!(
            "missing consume configuration for stone {stone_id}"
        ))
    })?;
    if current.unlocked_stones.contains(&stone_id) {
        return Ok(no_change(current.state));
    }

    Ok(MutationPlan {
        expected: current.state,
        kind: MutationKind::UnlockStone { stone_id },
        costs: parse_material_costs(&[consume.consume.as_str()])?,
    })
}

fn plan_stone_use(
    facet_ids: &[i32],
    current: &OwnedDestinyHero,
    stone_id: i32,
) -> Result<MutationPlan, ProgressionError> {
    if stone_id != 0 {
        if !facet_ids.contains(&stone_id) {
            return invalid(format!(
                "stone {stone_id} is not owned by hero {}",
                current.hero_id
            ));
        }
        if !current.unlocked_stones.contains(&stone_id) {
            return invalid(format!("stone {stone_id} is locked"));
        }
    }
    if current.state.stone == stone_id {
        return Ok(no_change(current.state));
    }

    Ok(MutationPlan {
        expected: current.state,
        kind: MutationKind::UseStone { stone_id },
        costs: Vec::new(),
    })
}

fn validate_progress_state(
    catalog: &DestinyConfigIndex,
    slots_id: i32,
    state: DestinyState,
) -> Result<(), ProgressionError> {
    if state.rank <= 0 || state.level <= 0 {
        return invalid(format!(
            "invalid Destiny state ({}, {})",
            state.rank, state.level
        ));
    }
    if catalog.slot(slots_id, state.rank, state.level).is_none() {
        return invalid(format!(
            "Destiny state ({}, {}) is not configured for slot group {slots_id}",
            state.rank, state.level
        ));
    }
    Ok(())
}

fn no_change(expected: DestinyState) -> MutationPlan {
    MutationPlan {
        expected,
        kind: MutationKind::NoChange,
        costs: Vec::new(),
    }
}

fn parse_kind(raw: Option<&str>, entry: &str) -> Result<MaterialKind, ProgressionError> {
    match raw.and_then(|value| value.parse::<i32>().ok()) {
        Some(1) => Ok(MaterialKind::Item),
        Some(2) => Ok(MaterialKind::Currency),
        _ => invalid(format!("unknown material type in {entry:?}")),
    }
}

fn parse_id(raw: Option<&str>, entry: &str) -> Result<i32, ProgressionError> {
    let id = raw
        .and_then(|value| value.parse::<i32>().ok())
        .ok_or_else(|| ProgressionError::Invalid(format!("invalid material id in {entry:?}")))?;
    if id <= 0 {
        return invalid(format!("material id must be positive in {entry:?}"));
    }
    Ok(id)
}

fn parse_amount(raw: Option<&str>, entry: &str) -> Result<i64, ProgressionError> {
    let amount = raw
        .and_then(|value| value.parse::<i64>().ok())
        .ok_or_else(|| {
            ProgressionError::Invalid(format!("invalid material amount in {entry:?}"))
        })?;
    if !(1..=i32::MAX as i64).contains(&amount) {
        return invalid(format!("material amount is out of range in {entry:?}"));
    }
    Ok(amount)
}

fn invalid<T>(message: impl Into<String>) -> Result<T, ProgressionError> {
    Err(ProgressionError::Invalid(message.into()))
}
