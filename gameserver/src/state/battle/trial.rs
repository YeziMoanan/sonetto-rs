use anyhow::{Result, anyhow};
use sonettobuf::FightGroup;
use std::collections::HashSet;

pub const MAX_TRIAL_UID_ORDINAL: usize = 1_000;
const MAX_BATTLE_AID_ORDINAL: i64 = 4;
const MIN_3_6_TRIAL_ID: i64 = 1_001;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NormalizedTrial {
    pub trial_id: i32,
    pub uid: i64,
    pub position: Option<i32>,
    pub is_substitute: bool,
}

impl NormalizedTrial {
    pub fn has_initial_card(self) -> bool {
        !self.is_substitute
    }
}

pub fn normalize_trial_requests(group: &FightGroup) -> Result<Vec<NormalizedTrial>> {
    validate_negative_hero_uids(group)?;
    let reserved_aid_ordinal = max_battle_aid_ordinal(group);

    if !group.trial_hero_list.is_empty() {
        if group
            .hero_list
            .iter()
            .chain(group.sub_hero_list.iter())
            .any(|uid| is_legacy_trial_uid(*uid))
        {
            return Err(anyhow!(
                "explicit trial_hero_list cannot be mixed with legacy negative trial UIDs"
            ));
        }
        return normalize_explicit_trials(group, reserved_aid_ordinal);
    }

    let main_legacy = group
        .hero_list
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, uid)| is_legacy_trial_uid(*uid))
        .collect::<Vec<_>>();
    let substitute_legacy = group
        .sub_hero_list
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, uid)| is_legacy_trial_uid(*uid))
        .collect::<Vec<_>>();
    let total = main_legacy.len() + substitute_legacy.len();
    ensure_trial_uid_capacity(reserved_aid_ordinal, total)?;

    let mut trials = Vec::with_capacity(total);
    let mut seen = HashSet::new();
    for (ordinal, (position, raw_uid)) in main_legacy.into_iter().enumerate() {
        let trial_id = legacy_trial_id(raw_uid)?;
        if !seen.insert(trial_id) {
            return Err(anyhow!("duplicate legacy trial_id {trial_id}"));
        }
        trials.push(NormalizedTrial {
            trial_id,
            uid: -((reserved_aid_ordinal + ordinal + 1) as i64),
            position: Some(
                i32::try_from(position + 1)
                    .map_err(|_| anyhow!("legacy trial position is outside i32 range"))?,
            ),
            is_substitute: false,
        });
    }
    for (position, raw_uid) in substitute_legacy {
        let trial_id = legacy_trial_id(raw_uid)?;
        if !seen.insert(trial_id) {
            return Err(anyhow!("duplicate legacy trial_id {trial_id}"));
        }
        trials.push(NormalizedTrial {
            trial_id,
            uid: -((reserved_aid_ordinal + trials.len() + 1) as i64),
            position: Some(
                -i32::try_from(position + 1)
                    .map_err(|_| anyhow!("legacy trial position is outside i32 range"))?,
            ),
            is_substitute: true,
        });
    }
    Ok(trials)
}

pub fn active_hero_count(group: &FightGroup, trials: &[NormalizedTrial]) -> usize {
    group
        .hero_list
        .iter()
        .filter(|uid| **uid > 0 || is_battle_aid_placeholder(**uid))
        .count()
        + trials
            .iter()
            .copied()
            .filter(|trial| trial.has_initial_card())
            .count()
}

pub fn card_source_hero_count(group: &FightGroup, trials: &[NormalizedTrial]) -> usize {
    group.hero_list.iter().filter(|uid| **uid > 0).count()
        + trials
            .iter()
            .copied()
            .filter(|trial| trial.has_initial_card())
            .count()
}

pub fn reserved_attacker_uid_ordinal(group: &FightGroup, trials: &[NormalizedTrial]) -> usize {
    let trial_ordinal = trials
        .iter()
        .filter_map(|trial| trial.uid.checked_neg())
        .filter_map(|uid| usize::try_from(uid).ok())
        .max()
        .unwrap_or(0);
    max_battle_aid_ordinal(group).max(trial_ordinal)
}

fn normalize_explicit_trials(
    group: &FightGroup,
    reserved_aid_ordinal: usize,
) -> Result<Vec<NormalizedTrial>> {
    ensure_trial_uid_capacity(reserved_aid_ordinal, group.trial_hero_list.len())?;
    let mut seen = HashSet::new();
    let mut seen_positions = HashSet::new();
    let mut trials = Vec::with_capacity(group.trial_hero_list.len());
    for (index, trial) in group.trial_hero_list.iter().enumerate() {
        let trial_id = trial
            .trial_id
            .ok_or_else(|| anyhow!("trial hero at index {index} is missing trial_id"))?;
        if trial_id <= 0 {
            return Err(anyhow!(
                "trial hero at index {index} has invalid trial_id {trial_id}"
            ));
        }
        let position = trial
            .pos
            .ok_or_else(|| anyhow!("trial hero at index {index} is missing position"))?;
        if position == 0 {
            return Err(anyhow!(
                "trial hero at index {index} has invalid position 0"
            ));
        }
        if position.checked_abs().is_none() {
            return Err(anyhow!(
                "trial hero at index {index} has unrepresentable position {position}"
            ));
        }
        if !seen.insert(trial_id) {
            return Err(anyhow!("duplicate trial_id {trial_id}"));
        }
        if !seen_positions.insert(position) {
            return Err(anyhow!("duplicate trial position {position}"));
        }
        let is_substitute = position < 0;
        trials.push(NormalizedTrial {
            trial_id,
            uid: -((reserved_aid_ordinal + index + 1) as i64),
            position: Some(position),
            is_substitute,
        });
    }
    Ok(trials)
}

fn legacy_trial_id(raw_uid: i64) -> Result<i32> {
    let magnitude = raw_uid
        .checked_neg()
        .ok_or_else(|| anyhow!("legacy trial UID {raw_uid} overflows"))?;
    let trial_id = i32::try_from(magnitude)
        .map_err(|_| anyhow!("legacy trial UID {raw_uid} is outside trial ID range"))?;
    if trial_id <= 0 {
        return Err(anyhow!("legacy trial UID {raw_uid} has invalid trial ID"));
    }
    Ok(trial_id)
}

fn is_legacy_trial_uid(raw_uid: i64) -> bool {
    raw_uid <= -MIN_3_6_TRIAL_ID
}

fn is_battle_aid_placeholder(raw_uid: i64) -> bool {
    (-MAX_BATTLE_AID_ORDINAL..=-1).contains(&raw_uid)
}

fn validate_negative_hero_uids(group: &FightGroup) -> Result<()> {
    for raw_uid in group.hero_list.iter().chain(group.sub_hero_list.iter()) {
        if *raw_uid < 0 && !is_battle_aid_placeholder(*raw_uid) && !is_legacy_trial_uid(*raw_uid) {
            return Err(anyhow!("unsupported negative hero UID {raw_uid}"));
        }
    }
    Ok(())
}

fn max_battle_aid_ordinal(group: &FightGroup) -> usize {
    group
        .hero_list
        .iter()
        .chain(group.sub_hero_list.iter())
        .copied()
        .filter(|uid| is_battle_aid_placeholder(*uid))
        .filter_map(i64::checked_neg)
        .filter_map(|uid| usize::try_from(uid).ok())
        .max()
        .unwrap_or(0)
}

fn ensure_trial_uid_capacity(reserved_ordinal: usize, count: usize) -> Result<()> {
    let last_ordinal = reserved_ordinal
        .checked_add(count)
        .ok_or_else(|| anyhow!("trial UID ordinal overflow"))?;
    if last_ordinal > MAX_TRIAL_UID_ORDINAL {
        return Err(anyhow!(
            "trial UID ordinal {last_ordinal} crosses the defender UID namespace"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sonettobuf::{FightGroup, TrialHero};

    #[test]
    fn legacy_trial_id_is_not_treated_as_an_ordinal() {
        let group = FightGroup {
            hero_list: vec![-2_241_001],
            ..Default::default()
        };
        assert_eq!(
            normalize_trial_requests(&group).unwrap(),
            vec![NormalizedTrial {
                trial_id: 2_241_001,
                uid: -1,
                position: Some(1),
                is_substitute: false,
            }]
        );
    }

    #[test]
    fn battle_aid_placeholders_are_not_legacy_trials() {
        let group = FightGroup {
            hero_list: vec![-1, -4],
            ..Default::default()
        };

        assert!(normalize_trial_requests(&group).unwrap().is_empty());
    }

    #[test]
    fn negative_uid_namespace_boundaries_are_exact() {
        for aid_uid in -4..=-1 {
            let group = FightGroup {
                hero_list: vec![aid_uid],
                ..Default::default()
            };
            assert!(normalize_trial_requests(&group).unwrap().is_empty());
        }

        for unsupported_uid in [-5, -1_000] {
            let group = FightGroup {
                hero_list: vec![unsupported_uid],
                ..Default::default()
            };
            assert!(normalize_trial_requests(&group).is_err());
        }

        let group = FightGroup {
            hero_list: vec![-1_001],
            ..Default::default()
        };
        assert_eq!(
            normalize_trial_requests(&group).unwrap(),
            vec![NormalizedTrial {
                trial_id: 1_001,
                uid: -1,
                position: Some(1),
                is_substitute: false,
            }]
        );
    }

    #[test]
    fn explicit_trial_can_coexist_with_battle_aid_placeholder() {
        let group = FightGroup {
            hero_list: vec![-1],
            trial_hero_list: vec![TrialHero {
                trial_id: Some(2_241_001),
                pos: Some(2),
                ..Default::default()
            }],
            ..Default::default()
        };

        assert_eq!(
            normalize_trial_requests(&group).unwrap(),
            vec![NormalizedTrial {
                trial_id: 2_241_001,
                uid: -2,
                position: Some(2),
                is_substitute: false,
            }]
        );
    }

    #[test]
    fn negative_uid_below_the_3_6_trial_id_namespace_is_rejected() {
        let group = FightGroup {
            hero_list: vec![-1_000],
            ..Default::default()
        };

        let error = normalize_trial_requests(&group).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("unsupported negative hero UID -1000")
        );
    }

    #[test]
    fn minimum_legacy_uid_is_rejected_without_overflowing() {
        let group = FightGroup {
            hero_list: vec![i64::MIN],
            ..Default::default()
        };

        let error = normalize_trial_requests(&group).unwrap_err();
        assert!(error.to_string().contains("overflows"));
    }

    #[test]
    fn explicit_negative_position_is_a_substitute_without_initial_card() {
        let group = FightGroup {
            trial_hero_list: vec![TrialHero {
                trial_id: Some(2_241_001),
                pos: Some(-1),
                ..Default::default()
            }],
            ..Default::default()
        };
        let trial = normalize_trial_requests(&group).unwrap()[0];
        assert!(trial.is_substitute);
        assert!(!trial.has_initial_card());
        assert_eq!(trial.uid, -1);
    }

    #[test]
    fn negative_substitute_trial_id_is_normalized_into_sub_entity_uid() {
        let group = FightGroup {
            sub_hero_list: vec![-2_241_001],
            ..Default::default()
        };
        let trial = normalize_trial_requests(&group).unwrap()[0];
        assert!(trial.is_substitute);
        assert_eq!(trial.uid, -1);
    }

    #[test]
    fn explicit_and_legacy_trial_encodings_cannot_be_mixed() {
        let group = FightGroup {
            hero_list: vec![-2_241_001],
            trial_hero_list: vec![TrialHero {
                trial_id: Some(2_241_001),
                pos: Some(1),
                ..Default::default()
            }],
            ..Default::default()
        };

        let error = normalize_trial_requests(&group).unwrap_err();
        assert!(error.to_string().contains("cannot be mixed"));
    }

    #[test]
    fn explicit_zero_position_is_rejected() {
        let group = FightGroup {
            trial_hero_list: vec![TrialHero {
                trial_id: Some(2_241_001),
                pos: Some(0),
                ..Default::default()
            }],
            ..Default::default()
        };

        assert!(normalize_trial_requests(&group).is_err());
    }

    #[test]
    fn explicit_missing_position_is_rejected() {
        let group = FightGroup {
            trial_hero_list: vec![TrialHero {
                trial_id: Some(2_241_001),
                ..Default::default()
            }],
            ..Default::default()
        };

        assert!(normalize_trial_requests(&group).is_err());
    }

    #[test]
    fn active_count_includes_main_aids_and_excludes_substitutes() {
        let group = FightGroup {
            hero_list: vec![42, -1],
            sub_hero_list: vec![43, -2],
            trial_hero_list: vec![
                TrialHero {
                    trial_id: Some(101),
                    pos: Some(1),
                    ..Default::default()
                },
                TrialHero {
                    trial_id: Some(102),
                    pos: Some(2),
                    ..Default::default()
                },
                TrialHero {
                    trial_id: Some(103),
                    pos: Some(-1),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let trials = normalize_trial_requests(&group).unwrap();

        assert_eq!(active_hero_count(&group, &trials), 4);
    }

    #[test]
    fn explicit_substitute_uids_use_one_ordinal_per_trial() {
        let group = FightGroup {
            trial_hero_list: (1..=501)
                .map(|index| TrialHero {
                    trial_id: Some(index),
                    pos: Some(-index),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };

        let trials = normalize_trial_requests(&group).unwrap();
        assert_eq!(trials.len(), 501);
        assert_eq!(trials.last().unwrap().uid, -501);
        assert!(trials.iter().all(|trial| trial.is_substitute));
    }

    #[test]
    fn explicit_trial_capacity_stops_before_defender_namespace() {
        let make_group = |count| FightGroup {
            trial_hero_list: (1..=count)
                .map(|index| TrialHero {
                    trial_id: Some(index),
                    pos: Some(-index),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };

        let at_limit = normalize_trial_requests(&make_group(1_000)).unwrap();
        assert_eq!(at_limit.last().unwrap().uid, -1_000);
        assert!(normalize_trial_requests(&make_group(1_001)).is_err());
    }

    #[test]
    fn aid_uid_prefix_reduces_available_trial_uid_capacity() {
        let make_group = |count| FightGroup {
            hero_list: vec![-4],
            trial_hero_list: (1..=count)
                .map(|index| TrialHero {
                    trial_id: Some(index),
                    pos: Some(index),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };

        let at_limit = normalize_trial_requests(&make_group(996)).unwrap();
        assert_eq!(at_limit.first().unwrap().uid, -5);
        assert_eq!(at_limit.last().unwrap().uid, -1_000);
        assert!(normalize_trial_requests(&make_group(997)).is_err());
    }
}
