use std::collections::{BTreeMap, BTreeSet};

use kernel_types::SemanticId;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SubtypeClosure {
    ancestors: BTreeMap<SemanticId, BTreeSet<SemanticId>>,
    descendants: BTreeMap<SemanticId, BTreeSet<SemanticId>>,
}

impl SubtypeClosure {
    #[must_use]
    pub fn is_subtype(&self, subtype: SemanticId, supertype: SemanticId) -> bool {
        subtype == supertype
            || self
                .ancestors
                .get(&subtype)
                .is_some_and(|ancestors| ancestors.contains(&supertype))
    }

    pub fn ancestors(&self, subtype: SemanticId) -> impl Iterator<Item = SemanticId> + '_ {
        std::iter::once(subtype).chain(
            self.ancestors
                .get(&subtype)
                .into_iter()
                .flat_map(|ancestors| ancestors.iter().copied()),
        )
    }

    pub(super) fn include(&mut self, subtype: SemanticId, supertype: SemanticId) {
        let lower = std::iter::once(subtype)
            .chain(
                self.descendants
                    .get(&subtype)
                    .into_iter()
                    .flat_map(|descendants| descendants.iter().copied()),
            )
            .collect::<Vec<_>>();
        let upper = std::iter::once(supertype)
            .chain(
                self.ancestors
                    .get(&supertype)
                    .into_iter()
                    .flat_map(|ancestors| ancestors.iter().copied()),
            )
            .collect::<Vec<_>>();

        for child in &lower {
            self.ancestors
                .entry(*child)
                .or_default()
                .extend(upper.iter().copied());
        }
        for parent in &upper {
            self.descendants
                .entry(*parent)
                .or_default()
                .extend(lower.iter().copied());
        }
    }
}
