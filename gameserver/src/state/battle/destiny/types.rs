#![allow(dead_code)]

use std::collections::BTreeMap;

use sonettobuf::{HeroExAttribute, HeroSpAttribute};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FixedTenths(pub i64);

impl FixedTenths {
    pub fn checked_add(self, other: Self) -> Result<Self, DestinyResolveError> {
        self.0
            .checked_add(other.0)
            .map(Self)
            .ok_or(DestinyResolveError::Overflow)
    }

    pub fn floor_percent_of(self, base: i32) -> Result<i32, DestinyResolveError> {
        let scaled = i64::from(base)
            .checked_mul(self.0)
            .ok_or(DestinyResolveError::Overflow)?;
        let value = scaled / 1000;
        i32::try_from(value).map_err(|_| DestinyResolveError::Overflow)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DestinyState {
    pub rank: i32,
    pub level: i32,
    pub facet_id: i32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HeroBaseAttributes {
    pub hp: i32,
    pub attack: i32,
    pub defense: i32,
    pub mdefense: i32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HeroSource {
    Owned,
    Trial,
    Activity,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HeroBuildContext {
    pub hero_id: i32,
    pub skin: i32,
    pub rank: i32,
    pub ex_skill_level: i32,
    pub destiny: DestinyState,
    pub is_substitute: bool,
    pub hero_type: i32,
    pub source: HeroSource,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DestinyTrace {
    pub source_key: String,
    pub detail: String,
}

#[derive(Debug, thiserror::Error)]
pub enum DestinyResolveError {
    #[error("invalid Destiny state: {0}")]
    InvalidState(String),
    #[error("invalid Destiny config: {0}")]
    InvalidConfig(String),
    #[error("Destiny arithmetic overflow")]
    Overflow,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ResolvedDestinyAttributes {
    pub hp: i32,
    pub attack: i32,
    pub defense: i32,
    pub mdefense: i32,
    pub raw_tenths: BTreeMap<i32, i64>,
    /// Protocol-compatible special attributes resolved from Destiny nodes.
    /// `real_hurt_rate` and `poison_add_rate` stay internal until an
    /// authoritative damage formula identifies their consumption point.
    pub ex_attr: HeroExAttribute,
    pub sp_attr: HeroSpAttribute,
    pub real_hurt_rate: i32,
    pub poison_add_rate: i32,
    pub trace: Vec<DestinyTrace>,
}

pub type DestinyModifierMap = BTreeMap<i64, ResolvedDestinyAttributes>;
