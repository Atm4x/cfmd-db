#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum DerivedArtifactTarget {
    Artifact(UnifiedArtifactId),
    RowOccurrenceAtom(SemanticId, LayoutId),
}

type DerivedArtifactDependencyMap = BTreeMap<(SemanticId, LayoutId), Vec<DerivedArtifactTarget>>;

#[derive(Debug, Clone, Default)]
struct DerivedArtifactDependencyCache(OnceLock<Arc<DerivedArtifactDependencyMap>>);

impl PartialEq for DerivedArtifactDependencyCache {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl Eq for DerivedArtifactDependencyCache {}

impl UnifiedArtifactId {
    fn capabilities(&self) -> BTreeSet<PhysicalCapability> {
        match self {
            Self::I64Index(_) => BTreeSet::from([PhysicalCapability::PointLookup]),
            Self::SemanticFiber { profile, .. } => profile.capabilities(),
            Self::SemanticQuotientSupport(_) => {
                BTreeSet::from([PhysicalCapability::RowCanonicalKey])
            }
        }
    }

    fn touches_relation_layout(&self, relation: SemanticId, layout: LayoutBinding) -> bool {
        match self {
            Self::I64Index(binding) => {
                binding.relation == relation && binding.layout.id == layout.id
            }
            Self::SemanticFiber { binding, .. } => {
                binding.relation == relation && binding.layout.id == layout.id
            }
            Self::SemanticQuotientSupport(binding) => binding.leaves.iter().any(|candidate| {
                candidate.relation == relation && candidate.layout.id == layout.id
            }),
        }
    }
}
