use std::collections::BTreeSet;

use crate::fiber_retention_plan::{FiberResourceCost, FiberResourceSpec};

/// Production-shaped reconstructible resource atoms for one pinned semantic fiber.
///
/// These are physical ownership units only. They do not introduce semantic family
/// identity; the retention compiler may combine them into any exact closed plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FiberPhysicalAtom {
    GlobalCardinalityMetadata,
    JointMassCounters,
    CanonicalJointKeyPool,
    SharedRowKeyRoute,
    JointRowMembership,
    CoordinateClassCatalog,
    JointSignatureCatalog,
    InternedRowRoute,
    ProjectionIncidence(usize),
    DirectSlotMass(usize),
    DirectSlotRows(usize),
}

impl FiberPhysicalAtom {
    #[must_use]
    pub fn maintenance_dependencies(self) -> BTreeSet<Self> {
        match self {
            Self::SharedRowKeyRoute | Self::JointMassCounters | Self::JointRowMembership => {
                BTreeSet::from([Self::CanonicalJointKeyPool])
            }
            Self::JointSignatureCatalog => BTreeSet::from([Self::CoordinateClassCatalog]),
            Self::InternedRowRoute | Self::ProjectionIncidence(_) => {
                BTreeSet::from([Self::JointSignatureCatalog])
            }
            Self::GlobalCardinalityMetadata
            | Self::CanonicalJointKeyPool
            | Self::CoordinateClassCatalog
            | Self::DirectSlotMass(_)
            | Self::DirectSlotRows(_) => BTreeSet::new(),
        }
    }
}

#[must_use]
pub fn physical_resource_spec(
    atom: FiberPhysicalAtom,
    cost: FiberResourceCost,
) -> FiberResourceSpec<FiberPhysicalAtom> {
    FiberResourceSpec {
        atom,
        cost,
        requires: atom.maintenance_dependencies(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_row_route_and_joint_rows_share_one_canonical_key_pool() {
        assert_eq!(
            FiberPhysicalAtom::SharedRowKeyRoute.maintenance_dependencies(),
            BTreeSet::from([FiberPhysicalAtom::CanonicalJointKeyPool])
        );
        assert_eq!(
            FiberPhysicalAtom::JointRowMembership.maintenance_dependencies(),
            BTreeSet::from([FiberPhysicalAtom::CanonicalJointKeyPool])
        );
    }

    #[test]
    fn projection_backbone_dependencies_do_not_force_row_membership() {
        assert_eq!(
            FiberPhysicalAtom::ProjectionIncidence(2).maintenance_dependencies(),
            BTreeSet::from([FiberPhysicalAtom::JointSignatureCatalog])
        );
        assert!(
            !FiberPhysicalAtom::ProjectionIncidence(2)
                .maintenance_dependencies()
                .contains(&FiberPhysicalAtom::JointRowMembership)
        );
    }
}
