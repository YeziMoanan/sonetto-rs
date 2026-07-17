// Auto-generated from JSON data
// Do not edit manually

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillExLevelDestinyFacets {
    pub desc: String,
    #[serde(rename = "facetsId")]
    pub facets_id: i32,
    #[serde(rename = "passiveSkill")]
    pub passive_skill: String,
    #[serde(rename = "skillEx")]
    pub skill_ex: i32,
    #[serde(rename = "skillGroup1")]
    pub skill_group1: String,
    #[serde(rename = "skillGroup2")]
    pub skill_group2: String,
    #[serde(rename = "skillLevel")]
    pub skill_level: i32,
}
pub struct SkillExLevelDestinyFacetsTable {
    records: Vec<SkillExLevelDestinyFacets>,
}

impl SkillExLevelDestinyFacetsTable {
    pub fn load(path: &str) -> anyhow::Result<Self> {
        let json = std::fs::read_to_string(path)?;
        let value: serde_json::Value = serde_json::from_str(&json)?;

        let records: Vec<SkillExLevelDestinyFacets> = if let Some(array) = value.as_array() {
            if array.len() >= 2 && array[1].is_array() {
                serde_json::from_value(array[1].clone())?
            } else {
                serde_json::from_value(value)?
            }
        } else {
            serde_json::from_value(value)?
        };

        Ok(Self {
            records,
        })
    }

    #[inline]
    pub fn all(&self) -> &[SkillExLevelDestinyFacets] {
        &self.records
    }

    #[inline]
    pub fn iter(&self) -> std::slice::Iter<'_, SkillExLevelDestinyFacets> {
        self.records.iter()
    }

    pub fn len(&self) -> usize { self.records.len() }
    pub fn is_empty(&self) -> bool { self.records.is_empty() }
}