use kernel_lens::RelRewriteLiftError;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) type GammaRowClass = Vec<kernel_semantics::CanonicalEqKey>;

#[derive(Debug)]
struct GammaPreimageBucket {
    source_occurrences: Vec<usize>,
    owner_classes: BTreeSet<GammaRowClass>,
    next_bag_occurrence: usize,
}

#[derive(Debug)]
pub(crate) struct GammaPreimageCatalog {
    buckets: BTreeMap<GammaRowClass, GammaPreimageBucket>,
}

fn gamma_row_class(
    row: &[kernel_model::Value],
    relation_type: &kernel_query::RelType,
    semantic: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<GammaRowClass, RelRewriteLiftError> {
    let equivalences = match &relation_type.semantics {
        kernel_schema::RelationSemantics::Set {
            column_equivalences,
        }
        | kernel_schema::RelationSemantics::Bag {
            column_equivalences,
        } => column_equivalences,
    };
    if row.len() != equivalences.len() {
        return Err(RelRewriteLiftError::RequestedViewInadmissible);
    }
    row.iter()
        .zip(equivalences)
        .map(|(value, &equivalence)| {
            registry
                .canonical_equivalence_key(semantic, equivalence, value)
                .map_err(kernel_query::RelQueryError::from)
                .map_err(RelRewriteLiftError::from)
        })
        .collect()
}

impl GammaPreimageCatalog {
    pub(crate) fn build<F>(
        source_rows: &[kernel_query::Row],
        owner_type: &kernel_query::RelType,
        projected_type: &kernel_query::RelType,
        semantic: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        mut project: F,
    ) -> Result<Self, RelRewriteLiftError>
    where
        F: FnMut(&kernel_query::Row) -> Result<kernel_query::Row, RelRewriteLiftError>,
    {
        let mut buckets = BTreeMap::<GammaRowClass, GammaPreimageBucket>::new();
        for (source_index, source_row) in source_rows.iter().enumerate() {
            let projected = project(source_row)?;
            let projected_class = gamma_row_class(&projected, projected_type, semantic, registry)?;
            let owner_class = gamma_row_class(source_row, owner_type, semantic, registry)?;
            let bucket = buckets
                .entry(projected_class)
                .or_insert_with(|| GammaPreimageBucket {
                    source_occurrences: Vec::new(),
                    owner_classes: BTreeSet::new(),
                    next_bag_occurrence: 0,
                });
            bucket.source_occurrences.push(source_index);
            bucket.owner_classes.insert(owner_class);
        }
        Ok(Self { buckets })
    }

    pub(crate) fn class_for_view_row(
        row: &[kernel_model::Value],
        projected_type: &kernel_query::RelType,
        semantic: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<GammaRowClass, RelRewriteLiftError> {
        gamma_row_class(row, projected_type, semantic, registry)
    }

    pub(crate) fn representative<'a>(
        &'a self,
        class: &GammaRowClass,
        source_rows: &'a [kernel_query::Row],
    ) -> Option<&'a kernel_query::Row> {
        self.buckets
            .get(class)
            .and_then(|bucket| bucket.source_occurrences.first())
            .and_then(|&index| source_rows.get(index))
    }

    pub(crate) fn remove_requested(
        &mut self,
        class: &GammaRowClass,
        removed: &mut [bool],
        set_semantics: bool,
    ) -> Result<(), RelRewriteLiftError> {
        let bucket = self
            .buckets
            .get_mut(class)
            .ok_or(RelRewriteLiftError::RequestedViewInadmissible)?;
        if set_semantics {
            for &index in &bucket.source_occurrences {
                removed[index] = true;
            }
            return Ok(());
        }
        if bucket.owner_classes.len() != 1 {
            return Err(RelRewriteLiftError::ProjectionPreimageAmbiguous);
        }
        while let Some(&index) = bucket.source_occurrences.get(bucket.next_bag_occurrence) {
            bucket.next_bag_occurrence += 1;
            if !removed[index] {
                removed[index] = true;
                return Ok(());
            }
        }
        Err(RelRewriteLiftError::RequestedViewInadmissible)
    }
}
