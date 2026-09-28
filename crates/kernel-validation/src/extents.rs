use std::collections::{BTreeMap, BTreeSet};

use kernel_identity::{DenseEntityIds, DenseEntitySet, DenseIdentityError};
use kernel_model::FiniteModel;
use kernel_types::{EntityId, SemanticId};
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DenseTypeExtents {
    ids: DenseEntityIds,
    extents: BTreeMap<SemanticId, DenseEntitySet>,
}

impl DenseTypeExtents {
    pub fn compile(
        model: &FiniteModel,
        schema: &kernel_schema::Schema,
    ) -> Result<Self, DenseIdentityError> {
        let entities = model
            .carriers
            .values()
            .flat_map(|carrier| carrier.iter().copied())
            .collect::<BTreeSet<_>>();
        let ids = DenseEntityIds::compile(&entities)?;
        Ok(Self::compile_with_ids(model, schema, &ids))
    }

    #[must_use]
    pub fn compile_with_ids(
        model: &FiniteModel,
        schema: &kernel_schema::Schema,
        ids: &DenseEntityIds,
    ) -> Self {
        let mut targets = model.carriers.keys().copied().collect::<BTreeSet<_>>();
        targets.extend(schema.type_definitions().map(|(id, _)| id));
        targets.extend(schema.capabilities().map(|capability| capability.id));
        for (subtype, supertype) in schema.inclusions() {
            targets.insert(subtype);
            targets.insert(supertype);
        }

        let mut extents = targets
            .iter()
            .copied()
            .map(|target| (target, DenseEntitySet::with_capacity(ids.len())))
            .collect::<BTreeMap<_, _>>();

        for (&actual, carrier) in &model.carriers {
            let matching_targets = schema
                .subtype_closure()
                .ancestors(actual)
                .filter(|target| targets.contains(target))
                .collect::<Vec<_>>();
            for &entity in carrier {
                let Some(local) = ids.local(entity) else {
                    continue;
                };
                for target in &matching_targets {
                    extents
                        .get_mut(target)
                        .expect("target extent was initialized")
                        .insert(local);
                }
            }
        }

        Self {
            ids: ids.clone(),
            extents,
        }
    }

    #[must_use]
    pub fn contains(&self, entity: EntityId, expected: SemanticId) -> bool {
        self.ids.local(entity).is_some_and(|local| {
            self.extents
                .get(&expected)
                .is_some_and(|extent| extent.contains(local))
        })
    }

    pub fn entities(&self, expected: SemanticId) -> impl Iterator<Item = EntityId> + '_ {
        self.extents
            .get(&expected)
            .into_iter()
            .flat_map(DenseEntitySet::iter)
            .filter_map(|local| self.ids.external(local))
    }

    #[must_use]
    pub fn entity_count(&self) -> usize {
        self.ids.len()
    }
}
