use crate::Row;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryKind {
    RelationData,
    RelationRewrite,
    RelationResolution,
    MixedRevision,
    FullRevision,
    SchemaMigration,
    LegacyTargetOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryReversibilityDto {
    ExactPlanInverse,
    ComplementRequired,
    NonPlanTransition,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryRelationChangeDto {
    pub relation: u128,
    pub inserted: Vec<Row>,
    pub removed: Vec<Row>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntryDto {
    pub effect_id: u128,
    pub prerequisites: Vec<u128>,
    pub transaction: u128,
    pub source_revision: u64,
    pub target_revision: u64,
    pub kind: HistoryKind,
    pub reversibility: HistoryReversibilityDto,
    pub changes: Vec<HistoryRelationChangeDto>,
}

impl From<&cfmd_runtime::HistoryEntry> for HistoryEntryDto {
    fn from(entry: &cfmd_runtime::HistoryEntry) -> Self {
        Self {
            effect_id: entry.effect_id(),
            prerequisites: entry.prerequisites().to_vec(),
            transaction: entry.transaction().raw(),
            source_revision: entry.source_revision().raw(),
            target_revision: entry.target_revision().raw(),
            kind: match entry.kind() {
                cfmd_runtime::HistoryEffectKind::RelationData => HistoryKind::RelationData,
                cfmd_runtime::HistoryEffectKind::RelationRewrite => HistoryKind::RelationRewrite,
                cfmd_runtime::HistoryEffectKind::RelationResolution => {
                    HistoryKind::RelationResolution
                }
                cfmd_runtime::HistoryEffectKind::MixedRevision => HistoryKind::MixedRevision,
                cfmd_runtime::HistoryEffectKind::FullRevision => HistoryKind::FullRevision,
                cfmd_runtime::HistoryEffectKind::SchemaMigration => HistoryKind::SchemaMigration,
                cfmd_runtime::HistoryEffectKind::LegacyTargetOnly => HistoryKind::LegacyTargetOnly,
            },
            reversibility: match entry.reversibility() {
                cfmd_runtime::HistoryReversibility::ExactPlanInverse => {
                    HistoryReversibilityDto::ExactPlanInverse
                }
                cfmd_runtime::HistoryReversibility::ComplementRequired => {
                    HistoryReversibilityDto::ComplementRequired
                }
                cfmd_runtime::HistoryReversibility::NonPlanTransition => {
                    HistoryReversibilityDto::NonPlanTransition
                }
            },
            changes: entry
                .changes()
                .iter()
                .map(|change| HistoryRelationChangeDto {
                    relation: change.relation().raw(),
                    inserted: change
                        .inserted()
                        .iter()
                        .cloned()
                        .map(|row| row.into_iter().map(Into::into).collect())
                        .collect(),
                    removed: change
                        .removed()
                        .iter()
                        .cloned()
                        .map(|row| row.into_iter().map(Into::into).collect())
                        .collect(),
                })
                .collect(),
        }
    }
}
