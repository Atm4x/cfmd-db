use super::*;

fn event(id: u128, prerequisites: &[u128], payload: &'static str) -> RevisionEffect<&'static str> {
    RevisionEffect {
        id: RevisionEffectId(id),
        prerequisites: prerequisites
            .iter()
            .copied()
            .map(RevisionEffectId)
            .collect(),
        payload,
    }
}

#[test]
fn criss_cross_common_history_is_canonical_effect_intersection() {
    let root = event(1, &[], "root");
    let a = event(2, &[1], "a");
    let b = event(3, &[1], "b");
    let left_resolution = event(4, &[2, 3], "left=10");
    let right_resolution = event(5, &[2, 3], "right=20");
    let left =
        RevisionEffectIdeal::new([root.clone(), a.clone(), b.clone(), left_resolution.clone()])
            .unwrap();
    let right =
        RevisionEffectIdeal::new([root.clone(), a.clone(), b.clone(), right_resolution.clone()])
            .unwrap();

    let common = left.common_ideal(&right).unwrap();
    assert_eq!(
        common.events().keys().copied().collect::<BTreeSet<_>>(),
        BTreeSet::from([
            RevisionEffectId(1),
            RevisionEffectId(2),
            RevisionEffectId(3),
        ])
    );
    assert_eq!(
        left.exclusive_from(&common).unwrap(),
        vec![&left_resolution]
    );
    assert_eq!(
        right.exclusive_from(&common).unwrap(),
        vec![&right_resolution]
    );
    assert_eq!(left.union(&right).unwrap().events().len(), 5);
}

#[test]
fn causal_layer_schedule_peels_deeper_exclusive_suffixes_in_dependency_order() {
    let root = event(1, &[], "root");
    let left = RevisionEffectIdeal::new([
        root.clone(),
        event(2, &[1], "left-a"),
        event(3, &[2], "left-b"),
    ])
    .unwrap();
    let right =
        RevisionEffectIdeal::new([root, event(4, &[1], "right-a"), event(5, &[4], "right-b")])
            .unwrap();
    let schedule = left.causal_layer_schedule(&right).unwrap();
    assert_eq!(schedule.common, BTreeSet::from([RevisionEffectId(1)]));
    assert_eq!(
        schedule.left_layers,
        vec![
            BTreeSet::from([RevisionEffectId(2)]),
            BTreeSet::from([RevisionEffectId(3)])
        ]
    );
    assert_eq!(
        schedule.right_layers,
        vec![
            BTreeSet::from([RevisionEffectId(4)]),
            BTreeSet::from([RevisionEffectId(5)])
        ]
    );
}

#[test]
fn two_layer_chain_consumes_first_residual_then_transports_second_layer() {
    let root = rewrite_event(1, &[], rewrite(1, 0));
    let a1 = rewrite(10, 1);
    let a2 = rewrite(11, 4);
    let b1 = rewrite(20, 2);
    let b2 = rewrite(21, 6);
    let left = RevisionEffectIdeal::new([
        root.clone(),
        rewrite_event(2, &[1], a1.clone()),
        rewrite_event(4, &[2], a2.clone()),
    ])
    .unwrap();
    let right = RevisionEffectIdeal::new([
        root,
        rewrite_event(3, &[1], b1.clone()),
        rewrite_event(5, &[3], b2.clone()),
    ])
    .unwrap();

    let b1_after_a1 = rewrite(30, 3);
    let a1_after_b1 = rewrite(31, 3);
    let b1_prefix_after_a2 = rewrite(32, 5);
    let a2_after_b1_prefix = rewrite(33, 5);
    let a1_prefix_after_b2 = rewrite(34, 7);
    let b2_after_a1_prefix = rewrite(35, 7);
    let b2_after_a2 = rewrite(36, 8);
    let a2_after_b2 = rewrite(37, 8);

    let mut registry = RewriteResidualFamilyRegistry::default();
    register_residual_pair(&mut registry, 800, &a1, &b1, &b1_after_a1, &a1_after_b1);
    register_residual_pair(
        &mut registry,
        801,
        &a2,
        &b1_after_a1,
        &b1_prefix_after_a2,
        &a2_after_b1_prefix,
    );
    register_residual_pair(
        &mut registry,
        802,
        &b2,
        &a1_after_b1,
        &a1_prefix_after_b2,
        &b2_after_a1_prefix,
    );
    register_residual_pair(
        &mut registry,
        803,
        &a2_after_b1_prefix,
        &b2_after_a1_prefix,
        &b2_after_a2,
        &a2_after_b2,
    );

    let certificate = left
        .certify_registered_two_layer_chain(
            &right,
            &registry,
            RevisionEffectTwoLayerResidualWitness {
                first_right_after_left: b1_after_a1,
                first_left_after_right: a1_after_b1,
                right_prefix_after_left_second: b1_prefix_after_a2,
                left_second_after_right_prefix: a2_after_b1_prefix,
                left_prefix_after_right_second: a1_prefix_after_b2,
                right_second_after_left_prefix: b2_after_a1_prefix,
                second_right_after_left: b2_after_a2,
                second_left_after_right: a2_after_b2,
            },
        )
        .unwrap();
    assert_eq!(certificate.first.common_endpoint(), &3);
    assert_eq!(certificate.left_transport.common_endpoint(), &5);
    assert_eq!(certificate.right_transport.common_endpoint(), &7);
    assert_eq!(certificate.common_endpoint(), &8);
    assert_eq!(certificate.schedule.left_layers.len(), 2);
    assert_eq!(certificate.schedule.right_layers.len(), 2);
}

#[test]
fn two_layer_chain_rejects_deeper_suffix_without_composite_residual_family() {
    let root = rewrite_event(1, &[], rewrite(1, 0));
    let left = RevisionEffectIdeal::new([
        root.clone(),
        rewrite_event(2, &[1], rewrite(10, 1)),
        rewrite_event(4, &[2], rewrite(11, 4)),
        rewrite_event(6, &[4], rewrite(12, 9)),
    ])
    .unwrap();
    let right = RevisionEffectIdeal::new([
        root,
        rewrite_event(3, &[1], rewrite(20, 2)),
        rewrite_event(5, &[3], rewrite(21, 6)),
    ])
    .unwrap();
    let witness = RevisionEffectTwoLayerResidualWitness {
        first_right_after_left: rewrite(30, 3),
        first_left_after_right: rewrite(31, 3),
        right_prefix_after_left_second: rewrite(32, 5),
        left_second_after_right_prefix: rewrite(33, 5),
        left_prefix_after_right_second: rewrite(34, 7),
        right_second_after_left_prefix: rewrite(35, 7),
        second_right_after_left: rewrite(36, 8),
        second_left_after_right: rewrite(37, 8),
    };
    assert_eq!(
        left.certify_registered_two_layer_chain(
            &right,
            &RewriteResidualFamilyRegistry::default(),
            witness,
        ),
        Err(RevisionEffectResidualLayerError::UnsupportedLayerShape)
    );
}

#[test]
fn residual_chain_uses_registered_composite_prefix_for_third_layer() {
    let root = rewrite_event(1, &[], rewrite(1, 0));
    let left_rw = [rewrite(10, 1), rewrite(11, 4), rewrite(12, 9)];
    let right_rw = [rewrite(20, 2), rewrite(21, 6), rewrite(22, 10)];
    let left = RevisionEffectIdeal::new([
        root.clone(),
        rewrite_event(2, &[1], left_rw[0].clone()),
        rewrite_event(4, &[2], left_rw[1].clone()),
        rewrite_event(6, &[4], left_rw[2].clone()),
    ])
    .unwrap();
    let right = RevisionEffectIdeal::new([
        root,
        rewrite_event(3, &[1], right_rw[0].clone()),
        rewrite_event(5, &[3], right_rw[1].clone()),
        rewrite_event(7, &[5], right_rw[2].clone()),
    ])
    .unwrap();
    let r = [
        rewrite(30, 3),
        rewrite(31, 3),
        rewrite(32, 5),
        rewrite(33, 5),
        rewrite(34, 7),
        rewrite(35, 7),
        rewrite(36, 8),
        rewrite(37, 8),
        rewrite(38, 8),
        rewrite(39, 8),
        rewrite(40, 11),
        rewrite(41, 11),
        rewrite(42, 12),
        rewrite(43, 12),
        rewrite(44, 13),
        rewrite(45, 13),
    ];
    let mut residuals = RewriteResidualFamilyRegistry::default();
    register_residual_pair(&mut residuals, 800, &left_rw[0], &right_rw[0], &r[0], &r[1]);
    register_residual_pair(&mut residuals, 801, &left_rw[1], &r[0], &r[2], &r[3]);
    register_residual_pair(&mut residuals, 802, &right_rw[1], &r[1], &r[4], &r[5]);
    register_residual_pair(&mut residuals, 803, &r[3], &r[5], &r[6], &r[7]);
    register_residual_pair(&mut residuals, 804, &left_rw[2], &r[8], &r[10], &r[11]);
    register_residual_pair(&mut residuals, 805, &right_rw[2], &r[9], &r[12], &r[13]);
    register_residual_pair(&mut residuals, 806, &r[11], &r[13], &r[14], &r[15]);
    let mut sequential = RewriteSequentialFamilyRegistry::default();
    for (id, first, second, composite) in [(900, &r[2], &r[6], &r[8]), (901, &r[4], &r[7], &r[9])] {
        sequential
            .register(RewriteSequentialFamilySpec {
                id: RewriteSequentialFamilyId(SemanticId(id)),
                key: RewriteSequentialFamilyKey {
                    first: first.into(),
                    second: second.into(),
                },
                composite: composite.into(),
            })
            .unwrap();
    }
    let witness = RevisionEffectResidualChainWitness {
        first_right_after_left: r[0].clone(),
        first_left_after_right: r[1].clone(),
        subsequent: vec![
            RevisionEffectResidualChainStepWitness {
                right_prefix_after_left: r[2].clone(),
                left_after_right_prefix: r[3].clone(),
                left_prefix_after_right: r[4].clone(),
                right_after_left_prefix: r[5].clone(),
                cross_right_after_left: r[6].clone(),
                cross_left_after_right: r[7].clone(),
                cumulative_right_prefix: Some(r[8].clone()),
                cumulative_left_prefix: Some(r[9].clone()),
            },
            RevisionEffectResidualChainStepWitness {
                right_prefix_after_left: r[10].clone(),
                left_after_right_prefix: r[11].clone(),
                left_prefix_after_right: r[12].clone(),
                right_after_left_prefix: r[13].clone(),
                cross_right_after_left: r[14].clone(),
                cross_left_after_right: r[15].clone(),
                cumulative_right_prefix: None,
                cumulative_left_prefix: None,
            },
        ],
    };
    let certificate = left
        .certify_registered_residual_chain(&right, &residuals, &sequential, witness)
        .unwrap();
    assert_eq!(certificate.schedule.left_layers.len(), 3);
    assert_eq!(certificate.subsequent.len(), 2);
    assert_eq!(certificate.common_endpoint(), &13);
    assert!(certificate.subsequent[0].cumulative_right_prefix.is_some());
    assert!(certificate.subsequent[0].cumulative_left_prefix.is_some());
}

#[test]
fn effect_identity_is_exact_and_ideal_must_be_down_closed_and_acyclic() {
    assert_eq!(
        RevisionEffectIdeal::new([event(2, &[1], "orphan")]),
        Err(RevisionEffectIdealError::MissingPrerequisite {
            effect: RevisionEffectId(2),
            prerequisite: RevisionEffectId(1),
        })
    );
    assert!(matches!(
        RevisionEffectIdeal::new([event(1, &[2], "a"), event(2, &[1], "b")]),
        Err(RevisionEffectIdealError::CyclicPrerequisites(_))
    ));
    let left = RevisionEffectIdeal::new([event(1, &[], "left")]).unwrap();
    let right = RevisionEffectIdeal::new([event(1, &[], "right")]).unwrap();
    assert_eq!(
        left.common_ideal(&right),
        Err(RevisionEffectIdealError::EffectIdentityConflict(
            RevisionEffectId(1)
        ))
    );
}

fn rewrite(spec: u128, endpoint: i32) -> PreparedRewrite<i32, ()> {
    PreparedRewrite {
        spec: RewriteSpecId(SemanticId(spec)),
        explicit_inputs: Vec::new(),
        effect: RewriteEffect::Replace(endpoint),
        law_set: RewriteLawSetId(SemanticId(900)),
    }
}

#[test]
fn residual_diamond_checks_endpoint_and_cube_checks_exact_residual_intent() {
    let right_after_left = rewrite(12, 3);
    let left_after_right = rewrite(13, 3);
    let diamond = certify_residual_diamond(right_after_left.clone(), left_after_right).unwrap();
    assert_eq!(diamond.common_endpoint(), &3);
    assert_eq!(
        certify_residual_diamond(right_after_left.clone(), rewrite(13, 4)),
        Err(RewriteCoherenceError::DiamondEndpointMismatch)
    );
    assert!(certify_cube_coherence(&right_after_left, &right_after_left).is_ok());
    assert_eq!(
        certify_cube_coherence(&right_after_left, &rewrite(99, 3)),
        Err(RewriteCoherenceError::CubeResidualIntentMismatch)
    );
}

#[test]
fn cube_coherence_proof_does_not_require_cloneable_endpoint_or_intent() {
    #[derive(Debug, PartialEq, Eq)]
    struct NoClone(i32);

    let rewrite = |value| PreparedRewrite {
        spec: RewriteSpecId(SemanticId(77)),
        explicit_inputs: vec![NoClone(value)],
        effect: RewriteEffect::Replace(NoClone(value)),
        law_set: RewriteLawSetId(SemanticId(78)),
    };
    let left = rewrite(9);
    let right = rewrite(9);
    let certificate = certify_cube_coherence(&left, &right).unwrap();
    assert_eq!(
        certificate.coherent_family(),
        RewriteFamilyIdentity::from(&left)
    );
}

#[test]
fn residual_family_registry_binds_pair_and_residual_intent_identity() {
    let left = rewrite(10, 1);
    let right = rewrite(11, 2);
    let right_after_left = rewrite(12, 3);
    let left_after_right = rewrite(13, 3);
    let spec = RewriteResidualFamilySpec {
        id: RewriteResidualFamilyId(SemanticId(700)),
        key: RewriteResidualFamilyKey {
            left: (&left).into(),
            right: (&right).into(),
        },
        right_after_left: (&right_after_left).into(),
        left_after_right: (&left_after_right).into(),
    };
    let mut registry = RewriteResidualFamilyRegistry::default();
    registry.register(spec).unwrap();
    assert_eq!(registry.family(spec.key), Some(&spec));
    assert_eq!(
        registry
            .certify(
                &left,
                &right,
                right_after_left.clone(),
                left_after_right.clone(),
            )
            .unwrap()
            .common_endpoint(),
        &3
    );
    assert_eq!(
        registry.certify(&left, &right, rewrite(99, 3), left_after_right,),
        Err(RewriteResidualRegistryError::ResidualIdentityMismatch)
    );
    assert_eq!(
        registry.register(RewriteResidualFamilySpec {
            id: RewriteResidualFamilyId(SemanticId(701)),
            ..spec
        }),
        Err(RewriteResidualRegistryError::PairAlreadyRegistered)
    );
}

#[test]
fn sequential_family_registry_certifies_exact_composite_identity_and_endpoint() {
    let first = rewrite(40, 1);
    let second = rewrite(41, 3);
    let composite = rewrite(42, 3);
    let mut registry = RewriteSequentialFamilyRegistry::default();
    registry
        .register(RewriteSequentialFamilySpec {
            id: RewriteSequentialFamilyId(SemanticId(900)),
            key: RewriteSequentialFamilyKey {
                first: (&first).into(),
                second: (&second).into(),
            },
            composite: (&composite).into(),
        })
        .unwrap();

    let certificate = registry
        .certify(first.clone(), second.clone(), composite.clone())
        .unwrap();
    assert_eq!(certificate.composite.as_rewrite(), &composite);

    assert_eq!(
        registry.certify(first.clone(), second.clone(), rewrite(99, 3)),
        Err(RewriteSequentialRegistryError::CompositeIdentityMismatch)
    );
    assert_eq!(
        registry.certify(first, second, rewrite(42, 4)),
        Err(RewriteSequentialRegistryError::CompositeEndpointMismatch)
    );
}

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Debug)]
struct CloneProbe {
    value: i32,
    clones: Arc<AtomicUsize>,
}

impl Clone for CloneProbe {
    fn clone(&self) -> Self {
        self.clones.fetch_add(1, Ordering::Relaxed);
        Self {
            value: self.value,
            clones: Arc::clone(&self.clones),
        }
    }
}

impl PartialEq for CloneProbe {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl Eq for CloneProbe {}

fn probe_rewrite(
    spec: u128,
    value: i32,
    clones: &Arc<AtomicUsize>,
) -> PreparedRewrite<CloneProbe, ()> {
    PreparedRewrite {
        spec: RewriteSpecId(SemanticId(spec)),
        explicit_inputs: Vec::new(),
        effect: RewriteEffect::Replace(CloneProbe {
            value,
            clones: Arc::clone(clones),
        }),
        law_set: RewriteLawSetId(SemanticId(900)),
    }
}

#[test]
fn rewrite_revision_effect_ideal_sharing_does_not_require_cloneable_endpoint() {
    #[derive(Debug, PartialEq, Eq)]
    struct NonCloneEndpoint(i32);
    #[derive(Debug, PartialEq, Eq)]
    struct NonCloneIntent(i32);

    fn shared_event(
        id: u128,
        prerequisites: &[u128],
        spec: u128,
        endpoint: i32,
    ) -> RevisionEffect<SharedPreparedRewrite<NonCloneEndpoint, NonCloneIntent>> {
        RevisionEffect {
            id: RevisionEffectId(id),
            prerequisites: prerequisites
                .iter()
                .copied()
                .map(RevisionEffectId)
                .collect(),
            payload: PreparedRewrite {
                spec: RewriteSpecId(SemanticId(spec)),
                explicit_inputs: vec![NonCloneIntent(endpoint)],
                effect: RewriteEffect::Replace(NonCloneEndpoint(endpoint)),
                law_set: RewriteLawSetId(SemanticId(900)),
            }
            .into(),
        }
    }

    let root = shared_event(1, &[], 1, 0);
    let left = RevisionEffectIdeal::new([root.clone(), shared_event(2, &[1], 10, 1)]).unwrap();
    let right = RevisionEffectIdeal::new([root, shared_event(3, &[1], 11, 2)]).unwrap();

    let common = left.common_ideal(&right).unwrap();
    assert_eq!(common.events().len(), 1);
    assert_eq!(left.union(&right).unwrap().events().len(), 3);
    assert_eq!(
        left.causal_layer_schedule(&right).unwrap().common,
        BTreeSet::from([RevisionEffectId(1)])
    );
}

#[test]
fn endpoint_backed_diamond_and_sequential_proofs_do_not_clone_large_states() {
    let clones = Arc::new(AtomicUsize::new(0));
    let left = probe_rewrite(10, 1, &clones);
    let right = probe_rewrite(11, 2, &clones);
    let right_after_left = probe_rewrite(12, 3, &clones);
    let left_after_right = probe_rewrite(13, 3, &clones);
    let mut residual = RewriteResidualFamilyRegistry::default();
    residual
        .register(RewriteResidualFamilySpec {
            id: RewriteResidualFamilyId(SemanticId(700)),
            key: RewriteResidualFamilyKey {
                left: (&left).into(),
                right: (&right).into(),
            },
            right_after_left: (&right_after_left).into(),
            left_after_right: (&left_after_right).into(),
        })
        .unwrap();
    clones.store(0, Ordering::Relaxed);
    let diamond = residual
        .certify(&left, &right, right_after_left, left_after_right)
        .unwrap();
    assert_eq!(diamond.common_endpoint().value, 3);
    assert_eq!(clones.load(Ordering::Relaxed), 0);

    let first = probe_rewrite(40, 1, &clones);
    let second = probe_rewrite(41, 5, &clones);
    let composite = probe_rewrite(42, 5, &clones);
    let mut sequential = RewriteSequentialFamilyRegistry::default();
    sequential
        .register(RewriteSequentialFamilySpec {
            id: RewriteSequentialFamilyId(SemanticId(701)),
            key: RewriteSequentialFamilyKey {
                first: (&first).into(),
                second: (&second).into(),
            },
            composite: (&composite).into(),
        })
        .unwrap();
    clones.store(0, Ordering::Relaxed);
    let composition = sequential.certify(first, second, composite).unwrap();
    assert_eq!(composition.composite.endpoint().value, 5);
    assert_eq!(clones.load(Ordering::Relaxed), 0);
}

fn register_probe_residual_pair(
    registry: &mut RewriteResidualFamilyRegistry,
    family_id: u128,
    left: &PreparedRewrite<CloneProbe, ()>,
    right: &PreparedRewrite<CloneProbe, ()>,
    right_after_left: &PreparedRewrite<CloneProbe, ()>,
    left_after_right: &PreparedRewrite<CloneProbe, ()>,
) {
    registry
        .register(RewriteResidualFamilySpec {
            id: RewriteResidualFamilyId(SemanticId(family_id)),
            key: RewriteResidualFamilyKey {
                left: left.into(),
                right: right.into(),
            },
            right_after_left: right_after_left.into(),
            left_after_right: left_after_right.into(),
        })
        .unwrap();
}

#[test]
fn endpoint_backed_cube_proof_does_not_clone_large_states() {
    let clones = Arc::new(AtomicUsize::new(0));
    let a = probe_rewrite(100, 1, &clones);
    let b = probe_rewrite(101, 2, &clones);
    let c = probe_rewrite(102, 3, &clones);
    let witness = RewriteResidualCubeWitness {
        b_after_a: probe_rewrite(110, 4, &clones).into(),
        a_after_b: probe_rewrite(111, 4, &clones).into(),
        c_after_a: probe_rewrite(112, 5, &clones).into(),
        a_after_c: probe_rewrite(113, 5, &clones).into(),
        c_after_b: probe_rewrite(114, 6, &clones).into(),
        b_after_c: probe_rewrite(115, 6, &clones).into(),
        c_after_ab: probe_rewrite(120, 7, &clones).into(),
        b_after_ac: probe_rewrite(121, 7, &clones).into(),
        c_after_ba: probe_rewrite(120, 7, &clones).into(),
        a_after_bc: probe_rewrite(122, 7, &clones).into(),
        b_after_ca: probe_rewrite(123, 7, &clones).into(),
        a_after_cb: probe_rewrite(124, 7, &clones).into(),
    };
    let mut registry = RewriteResidualFamilyRegistry::default();
    register_probe_residual_pair(
        &mut registry,
        800,
        &a,
        &b,
        &witness.b_after_a,
        &witness.a_after_b,
    );
    register_probe_residual_pair(
        &mut registry,
        801,
        &a,
        &c,
        &witness.c_after_a,
        &witness.a_after_c,
    );
    register_probe_residual_pair(
        &mut registry,
        802,
        &b,
        &c,
        &witness.c_after_b,
        &witness.b_after_c,
    );
    register_probe_residual_pair(
        &mut registry,
        803,
        &witness.b_after_a,
        &witness.c_after_a,
        &witness.c_after_ab,
        &witness.b_after_ac,
    );
    register_probe_residual_pair(
        &mut registry,
        804,
        &witness.a_after_b,
        &witness.c_after_b,
        &witness.c_after_ba,
        &witness.a_after_bc,
    );
    register_probe_residual_pair(
        &mut registry,
        805,
        &witness.a_after_c,
        &witness.b_after_c,
        &witness.b_after_ca,
        &witness.a_after_cb,
    );

    clones.store(0, Ordering::Relaxed);
    let cube = registry.certify_cube(&a, &b, &c, witness).unwrap();
    assert_eq!(cube.common_endpoint().value, 7);
    assert_eq!(
        cube.coherent_residual().spec,
        RewriteSpecId(SemanticId(120))
    );
    assert_eq!(clones.load(Ordering::Relaxed), 0);
}

fn register_residual_pair(
    registry: &mut RewriteResidualFamilyRegistry,
    id: u128,
    left: &PreparedRewrite<i32, ()>,
    right: &PreparedRewrite<i32, ()>,
    right_after_left: &PreparedRewrite<i32, ()>,
    left_after_right: &PreparedRewrite<i32, ()>,
) {
    registry
        .register(RewriteResidualFamilySpec {
            id: RewriteResidualFamilyId(SemanticId(id)),
            key: RewriteResidualFamilyKey {
                left: left.into(),
                right: right.into(),
            },
            right_after_left: right_after_left.into(),
            left_after_right: left_after_right.into(),
        })
        .unwrap();
}

fn register_sequential_pair(
    registry: &mut RewriteSequentialFamilyRegistry,
    id: u128,
    first: &PreparedRewrite<i32, ()>,
    second: &PreparedRewrite<i32, ()>,
    composite: &PreparedRewrite<i32, ()>,
) {
    registry
        .register(RewriteSequentialFamilySpec {
            id: RewriteSequentialFamilyId(SemanticId(id)),
            key: RewriteSequentialFamilyKey {
                first: first.into(),
                second: second.into(),
            },
            composite: composite.into(),
        })
        .unwrap();
}

struct SquareFixture {
    left: RevisionEffectIdeal<SharedPreparedRewrite<i32, ()>>,
    right: RevisionEffectIdeal<SharedPreparedRewrite<i32, ()>>,
    residual: RewriteResidualFamilyRegistry,
    sequential: RewriteSequentialFamilyRegistry,
    witness: RevisionEffectResidualSquareLayerWitness<i32, ()>,
}

fn square_fixture() -> SquareFixture {
    let a = rewrite(100, 1);
    let b = rewrite(101, 2);
    let c = rewrite(102, 4);
    let d = rewrite(103, 5);
    let b_after_a = rewrite(110, 3);
    let a_after_b = rewrite(111, 3);
    let d_after_c = rewrite(112, 6);
    let c_after_d = rewrite(113, 6);
    let left_composite = rewrite(120, 3);
    let right_composite = rewrite(121, 6);
    let right_after_left = rewrite(130, 7);
    let left_after_right = rewrite(131, 7);
    let left = RevisionEffectIdeal::new([
        rewrite_event(10, &[], a.clone()),
        rewrite_event(11, &[], b.clone()),
    ])
    .unwrap();
    let right = RevisionEffectIdeal::new([
        rewrite_event(20, &[], c.clone()),
        rewrite_event(21, &[], d.clone()),
    ])
    .unwrap();
    let mut residual = RewriteResidualFamilyRegistry::default();
    register_residual_pair(&mut residual, 1000, &a, &b, &b_after_a, &a_after_b);
    register_residual_pair(&mut residual, 1001, &c, &d, &d_after_c, &c_after_d);
    register_residual_pair(
        &mut residual,
        1002,
        &left_composite,
        &right_composite,
        &right_after_left,
        &left_after_right,
    );
    let mut sequential = RewriteSequentialFamilyRegistry::default();
    register_sequential_pair(&mut sequential, 1100, &a, &b_after_a, &left_composite);
    register_sequential_pair(&mut sequential, 1101, &b, &a_after_b, &left_composite);
    register_sequential_pair(&mut sequential, 1102, &c, &d_after_c, &right_composite);
    register_sequential_pair(&mut sequential, 1103, &d, &c_after_d, &right_composite);
    SquareFixture {
        left,
        right,
        residual,
        sequential,
        witness: RevisionEffectResidualSquareLayerWitness {
            left: RewriteConcurrentPairWitness {
                right_after_left: b_after_a.into(),
                left_after_right: a_after_b.into(),
                left_then_right_composite: left_composite.clone().into(),
                right_then_left_composite: left_composite.into(),
            },
            right: RewriteConcurrentPairWitness {
                right_after_left: d_after_c.into(),
                left_after_right: c_after_d.into(),
                left_then_right_composite: right_composite.clone().into(),
                right_then_left_composite: right_composite.into(),
            },
            right_after_left,
            left_after_right,
        },
    }
}

#[test]
fn two_by_two_frontier_normalizes_both_orders_before_cross_residual() {
    let fixture = square_fixture();
    let certificate = fixture
        .left
        .certify_registered_residual_square_frontier(
            &fixture.right,
            &fixture.residual,
            &fixture.sequential,
            fixture.witness,
        )
        .unwrap();
    assert_eq!(certificate.left_frontier.len(), 2);
    assert_eq!(certificate.right_frontier.len(), 2);
    assert_eq!(certificate.common_endpoint(), &7);
    assert_eq!(
        certificate.left_normalization.left_then_right.composite,
        certificate.left_normalization.right_then_left.composite
    );
    assert_eq!(
        certificate.right_normalization.left_then_right.composite,
        certificate.right_normalization.right_then_left.composite
    );
}

#[test]
fn square_first_layer_carries_exact_residual_prefix_into_singleton_suffix() {
    let SquareFixture {
        left: first_left,
        right: first_right,
        mut residual,
        sequential,
        witness: first,
    } = square_fixture();
    let left_second = rewrite(140, 8);
    let right_second = rewrite(141, 9);
    let right_prefix_after_left = rewrite(142, 10);
    let left_after_right_prefix = rewrite(143, 10);
    let left_prefix_after_right = rewrite(144, 11);
    let right_after_left_prefix = rewrite(145, 11);
    let cross_right_after_left = rewrite(146, 12);
    let cross_left_after_right = rewrite(147, 12);
    let left =
        RevisionEffectIdeal::new(first_left.events().values().cloned().chain([rewrite_event(
            12,
            &[10, 11],
            left_second.clone(),
        )]))
        .unwrap();
    let right =
        RevisionEffectIdeal::new(first_right.events().values().cloned().chain([rewrite_event(
            22,
            &[20, 21],
            right_second.clone(),
        )]))
        .unwrap();
    register_residual_pair(
        &mut residual,
        1003,
        &left_second,
        &first.right_after_left,
        &right_prefix_after_left,
        &left_after_right_prefix,
    );
    register_residual_pair(
        &mut residual,
        1004,
        &right_second,
        &first.left_after_right,
        &left_prefix_after_right,
        &right_after_left_prefix,
    );
    register_residual_pair(
        &mut residual,
        1005,
        &left_after_right_prefix,
        &right_after_left_prefix,
        &cross_right_after_left,
        &cross_left_after_right,
    );
    let certificate = left
        .certify_registered_residual_square_chain(
            &right,
            &residual,
            &sequential,
            RevisionEffectResidualSquareChainWitness {
                first,
                subsequent: vec![RevisionEffectResidualChainStepWitness {
                    right_prefix_after_left,
                    left_after_right_prefix,
                    left_prefix_after_right,
                    right_after_left_prefix,
                    cross_right_after_left,
                    cross_left_after_right,
                    cumulative_right_prefix: None,
                    cumulative_left_prefix: None,
                }],
            },
        )
        .unwrap();
    assert_eq!(certificate.schedule.left_layers.len(), 2);
    assert_eq!(certificate.schedule.right_layers.len(), 2);
    assert_eq!(certificate.common_endpoint(), &12);
}

struct EmbeddedSquareFixture {
    left: RevisionEffectIdeal<SharedPreparedRewrite<i32, ()>>,
    right: RevisionEffectIdeal<SharedPreparedRewrite<i32, ()>>,
    residual: RewriteResidualFamilyRegistry,
    sequential: RewriteSequentialFamilyRegistry,
    witness: RevisionEffectResidualMixedChainWitness<i32, ()>,
}

struct EmbeddedSquareIdeals {
    left: RevisionEffectIdeal<SharedPreparedRewrite<i32, ()>>,
    right: RevisionEffectIdeal<SharedPreparedRewrite<i32, ()>>,
}

struct EmbeddedSquareRewrites {
    left_first: PreparedRewrite<i32, ()>,
    right_first: PreparedRewrite<i32, ()>,
    right_after_left: PreparedRewrite<i32, ()>,
    left_after_right: PreparedRewrite<i32, ()>,
    left_a: PreparedRewrite<i32, ()>,
    left_b: PreparedRewrite<i32, ()>,
    left_b_after_a: PreparedRewrite<i32, ()>,
    left_a_after_b: PreparedRewrite<i32, ()>,
    left_composite: PreparedRewrite<i32, ()>,
    right_a: PreparedRewrite<i32, ()>,
    right_b: PreparedRewrite<i32, ()>,
    right_b_after_a: PreparedRewrite<i32, ()>,
    right_a_after_b: PreparedRewrite<i32, ()>,
    right_composite: PreparedRewrite<i32, ()>,
    right_prefix_after_left: PreparedRewrite<i32, ()>,
    left_after_right_prefix: PreparedRewrite<i32, ()>,
    left_prefix_after_right: PreparedRewrite<i32, ()>,
    right_after_left_prefix: PreparedRewrite<i32, ()>,
    cross_right_after_left: PreparedRewrite<i32, ()>,
    cross_left_after_right: PreparedRewrite<i32, ()>,
}

fn embedded_square_rewrites() -> EmbeddedSquareRewrites {
    EmbeddedSquareRewrites {
        left_first: rewrite(200, 1),
        right_first: rewrite(201, 2),
        right_after_left: rewrite(202, 3),
        left_after_right: rewrite(203, 3),
        left_a: rewrite(204, 4),
        left_b: rewrite(205, 5),
        left_b_after_a: rewrite(206, 6),
        left_a_after_b: rewrite(207, 6),
        left_composite: rewrite(208, 6),
        right_a: rewrite(209, 7),
        right_b: rewrite(210, 8),
        right_b_after_a: rewrite(211, 9),
        right_a_after_b: rewrite(212, 9),
        right_composite: rewrite(213, 9),
        right_prefix_after_left: rewrite(214, 10),
        left_after_right_prefix: rewrite(215, 10),
        left_prefix_after_right: rewrite(216, 11),
        right_after_left_prefix: rewrite(217, 11),
        cross_right_after_left: rewrite(218, 12),
        cross_left_after_right: rewrite(219, 12),
    }
}

fn embedded_square_ideals(r: &EmbeddedSquareRewrites) -> EmbeddedSquareIdeals {
    let left = RevisionEffectIdeal::new([
        rewrite_event(30, &[], r.left_first.clone()),
        rewrite_event(31, &[30], r.left_a.clone()),
        rewrite_event(32, &[30], r.left_b.clone()),
    ])
    .unwrap();
    let right = RevisionEffectIdeal::new([
        rewrite_event(40, &[], r.right_first.clone()),
        rewrite_event(41, &[40], r.right_a.clone()),
        rewrite_event(42, &[40], r.right_b.clone()),
    ])
    .unwrap();
    EmbeddedSquareIdeals { left, right }
}

fn embedded_square_registries(
    r: &EmbeddedSquareRewrites,
) -> (
    RewriteResidualFamilyRegistry,
    RewriteSequentialFamilyRegistry,
) {
    let mut residual = RewriteResidualFamilyRegistry::default();
    register_residual_pair(
        &mut residual,
        1200,
        &r.left_first,
        &r.right_first,
        &r.right_after_left,
        &r.left_after_right,
    );
    register_residual_pair(
        &mut residual,
        1201,
        &r.left_a,
        &r.left_b,
        &r.left_b_after_a,
        &r.left_a_after_b,
    );
    register_residual_pair(
        &mut residual,
        1202,
        &r.right_a,
        &r.right_b,
        &r.right_b_after_a,
        &r.right_a_after_b,
    );
    register_residual_pair(
        &mut residual,
        1203,
        &r.left_composite,
        &r.right_after_left,
        &r.right_prefix_after_left,
        &r.left_after_right_prefix,
    );
    register_residual_pair(
        &mut residual,
        1204,
        &r.right_composite,
        &r.left_after_right,
        &r.left_prefix_after_right,
        &r.right_after_left_prefix,
    );
    register_residual_pair(
        &mut residual,
        1205,
        &r.left_after_right_prefix,
        &r.right_after_left_prefix,
        &r.cross_right_after_left,
        &r.cross_left_after_right,
    );
    let mut sequential = RewriteSequentialFamilyRegistry::default();
    register_sequential_pair(
        &mut sequential,
        1300,
        &r.left_a,
        &r.left_b_after_a,
        &r.left_composite,
    );
    register_sequential_pair(
        &mut sequential,
        1301,
        &r.left_b,
        &r.left_a_after_b,
        &r.left_composite,
    );
    register_sequential_pair(
        &mut sequential,
        1302,
        &r.right_a,
        &r.right_b_after_a,
        &r.right_composite,
    );
    register_sequential_pair(
        &mut sequential,
        1303,
        &r.right_b,
        &r.right_a_after_b,
        &r.right_composite,
    );
    (residual, sequential)
}

fn embedded_square_witness(
    r: &EmbeddedSquareRewrites,
) -> RevisionEffectResidualMixedChainWitness<i32, ()> {
    RevisionEffectResidualMixedChainWitness {
        first: RevisionEffectResidualMixedFirstWitness::Singleton {
            right_after_left: r.right_after_left.clone(),
            left_after_right: r.left_after_right.clone(),
        },
        subsequent: vec![RevisionEffectResidualMixedStepWitness::Square(Box::new(
            RevisionEffectResidualMixedSquareStepWitness {
                left: RewriteConcurrentPairWitness {
                    right_after_left: r.left_b_after_a.clone().into(),
                    left_after_right: r.left_a_after_b.clone().into(),
                    left_then_right_composite: r.left_composite.clone().into(),
                    right_then_left_composite: r.left_composite.clone().into(),
                },
                right: RewriteConcurrentPairWitness {
                    right_after_left: r.right_b_after_a.clone().into(),
                    left_after_right: r.right_a_after_b.clone().into(),
                    left_then_right_composite: r.right_composite.clone().into(),
                    right_then_left_composite: r.right_composite.clone().into(),
                },
                transport: RevisionEffectResidualChainStepWitness {
                    right_prefix_after_left: r.right_prefix_after_left.clone(),
                    left_after_right_prefix: r.left_after_right_prefix.clone(),
                    left_prefix_after_right: r.left_prefix_after_right.clone(),
                    right_after_left_prefix: r.right_after_left_prefix.clone(),
                    cross_right_after_left: r.cross_right_after_left.clone(),
                    cross_left_after_right: r.cross_left_after_right.clone(),
                    cumulative_right_prefix: None,
                    cumulative_left_prefix: None,
                },
            },
        ))],
    }
}

fn embedded_square_fixture() -> EmbeddedSquareFixture {
    let rewrites = embedded_square_rewrites();
    let EmbeddedSquareIdeals { left, right } = embedded_square_ideals(&rewrites);
    let (residual, sequential) = embedded_square_registries(&rewrites);
    let witness = embedded_square_witness(&rewrites);
    EmbeddedSquareFixture {
        left,
        right,
        residual,
        sequential,
        witness,
    }
}

#[test]
fn square_layer_after_singleton_prefix_is_residualized_without_order_authority() {
    let fixture = embedded_square_fixture();
    let certificate = fixture
        .left
        .certify_registered_residual_mixed_chain(
            &fixture.right,
            &fixture.residual,
            &fixture.sequential,
            fixture.witness,
        )
        .unwrap();
    assert_eq!(certificate.schedule.left_layers.len(), 2);
    assert_eq!(certificate.schedule.left_layers[1].len(), 2);
    assert_eq!(certificate.schedule.right_layers[1].len(), 2);
    assert!(matches!(
        certificate.subsequent[0],
        RevisionEffectResidualMixedStepCertificate::Square { .. }
    ));
    assert_eq!(certificate.common_endpoint(), &12);
}

struct CubeFixture {
    registry: RewriteResidualFamilyRegistry,
    a: PreparedRewrite<i32, ()>,
    b: PreparedRewrite<i32, ()>,
    c: PreparedRewrite<i32, ()>,
    witness: RewriteResidualCubeWitness<i32, ()>,
}

fn cube_fixture(final_c_spec: u128) -> CubeFixture {
    let a = rewrite(10, 1);
    let b = rewrite(11, 2);
    let c = rewrite(12, 3);
    let witness = RewriteResidualCubeWitness {
        b_after_a: rewrite(20, 4).into(),
        a_after_b: rewrite(21, 4).into(),
        c_after_a: rewrite(22, 5).into(),
        a_after_c: rewrite(23, 5).into(),
        c_after_b: rewrite(24, 6).into(),
        b_after_c: rewrite(25, 6).into(),
        c_after_ab: rewrite(26, 7).into(),
        b_after_ac: rewrite(27, 7).into(),
        c_after_ba: rewrite(final_c_spec, 7).into(),
        a_after_bc: rewrite(28, 7).into(),
        b_after_ca: rewrite(29, 7).into(),
        a_after_cb: rewrite(30, 7).into(),
    };
    let mut registry = RewriteResidualFamilyRegistry::default();
    register_residual_pair(
        &mut registry,
        700,
        &a,
        &b,
        &witness.b_after_a,
        &witness.a_after_b,
    );
    register_residual_pair(
        &mut registry,
        701,
        &a,
        &c,
        &witness.c_after_a,
        &witness.a_after_c,
    );
    register_residual_pair(
        &mut registry,
        702,
        &b,
        &c,
        &witness.c_after_b,
        &witness.b_after_c,
    );
    register_residual_pair(
        &mut registry,
        703,
        &witness.b_after_a,
        &witness.c_after_a,
        &witness.c_after_ab,
        &witness.b_after_ac,
    );
    register_residual_pair(
        &mut registry,
        704,
        &witness.a_after_b,
        &witness.c_after_b,
        &witness.c_after_ba,
        &witness.a_after_bc,
    );
    register_residual_pair(
        &mut registry,
        705,
        &witness.a_after_c,
        &witness.b_after_c,
        &witness.b_after_ca,
        &witness.a_after_cb,
    );
    CubeFixture {
        registry,
        a,
        b,
        c,
        witness,
    }
}

fn braid_cube_fixture() -> CubeFixture {
    let mut fixture = cube_fixture(26);
    fixture.witness.b_after_ca = fixture.witness.b_after_ac.clone();
    fixture.witness.a_after_cb = fixture.witness.a_after_bc.clone();
    let mut registry = RewriteResidualFamilyRegistry::default();
    register_residual_pair(
        &mut registry,
        700,
        &fixture.a,
        &fixture.b,
        &fixture.witness.b_after_a,
        &fixture.witness.a_after_b,
    );
    register_residual_pair(
        &mut registry,
        701,
        &fixture.a,
        &fixture.c,
        &fixture.witness.c_after_a,
        &fixture.witness.a_after_c,
    );
    register_residual_pair(
        &mut registry,
        702,
        &fixture.b,
        &fixture.c,
        &fixture.witness.c_after_b,
        &fixture.witness.b_after_c,
    );
    register_residual_pair(
        &mut registry,
        703,
        &fixture.witness.b_after_a,
        &fixture.witness.c_after_a,
        &fixture.witness.c_after_ab,
        &fixture.witness.b_after_ac,
    );
    register_residual_pair(
        &mut registry,
        704,
        &fixture.witness.a_after_b,
        &fixture.witness.c_after_b,
        &fixture.witness.c_after_ba,
        &fixture.witness.a_after_bc,
    );
    register_residual_pair(
        &mut registry,
        705,
        &fixture.witness.a_after_c,
        &fixture.witness.b_after_c,
        &fixture.witness.b_after_ca,
        &fixture.witness.a_after_cb,
    );
    fixture.registry = registry;
    fixture
}

fn concurrent_triple_sequential_registry(
    fixture: &CubeFixture,
    ab: &PreparedRewrite<i32, ()>,
    ac: &PreparedRewrite<i32, ()>,
    bc: &PreparedRewrite<i32, ()>,
    final_composite: &PreparedRewrite<i32, ()>,
) -> RewriteSequentialFamilyRegistry {
    let mut registry = RewriteSequentialFamilyRegistry::default();
    let paths = [
        (&fixture.a, &fixture.witness.b_after_a, ab),
        (&fixture.b, &fixture.witness.a_after_b, ab),
        (&fixture.a, &fixture.witness.c_after_a, ac),
        (&fixture.c, &fixture.witness.a_after_c, ac),
        (&fixture.b, &fixture.witness.c_after_b, bc),
        (&fixture.c, &fixture.witness.b_after_c, bc),
    ];
    for (offset, (first, second, composite)) in paths.into_iter().enumerate() {
        register_sequential_pair(
            &mut registry,
            1500 + offset as u128,
            first,
            second,
            composite,
        );
    }
    let final_paths = [
        (ab, &fixture.witness.c_after_ab),
        (ac, &fixture.witness.b_after_ac),
        (ac, &fixture.witness.b_after_ca),
        (bc, &fixture.witness.a_after_bc),
        (bc, &fixture.witness.a_after_cb),
    ];
    for (offset, (first, second)) in final_paths.into_iter().enumerate() {
        register_sequential_pair(
            &mut registry,
            1510 + offset as u128,
            first,
            second,
            final_composite,
        );
    }
    registry
}

#[test]
fn concurrent_triple_normalizes_all_six_orders_to_one_exact_composite() {
    let fixture = cube_fixture(26);
    let ab = rewrite(40, 4);
    let ac = rewrite(41, 5);
    let bc = rewrite(42, 6);
    let final_composite = rewrite(43, 7);
    let sequential =
        concurrent_triple_sequential_registry(&fixture, &ab, &ac, &bc, &final_composite);
    let a = SharedPreparedRewrite::new(fixture.a);
    let b = SharedPreparedRewrite::new(fixture.b);
    let c = SharedPreparedRewrite::new(fixture.c);
    let certificate = certify_registered_concurrent_triple(
        &a,
        &b,
        &c,
        &fixture.registry,
        &sequential,
        RewriteConcurrentTripleWitness {
            cube: fixture.witness,
            ab_composite: ab.into(),
            ac_composite: ac.into(),
            bc_composite: bc.into(),
            final_composite: final_composite.clone().into(),
        },
    )
    .unwrap();
    assert_eq!(certificate.final_paths.len(), 6);
    assert_eq!(certificate.composite(), &final_composite);
    assert_eq!(certificate.cube.common_endpoint(), &7);
}

#[test]
fn normalized_frontier_consumes_three_by_one_without_branch_order_authority() {
    let mut fixture = cube_fixture(26);
    let ab = rewrite(40, 4);
    let ac = rewrite(41, 5);
    let bc = rewrite(42, 6);
    let left_composite = rewrite(43, 7);
    let sequential =
        concurrent_triple_sequential_registry(&fixture, &ab, &ac, &bc, &left_composite);
    let right_rewrite = rewrite(44, 8);
    let right_after_left = rewrite(45, 9);
    let left_after_right = rewrite(46, 9);
    register_residual_pair(
        &mut fixture.registry,
        706,
        &left_composite,
        &right_rewrite,
        &right_after_left,
        &left_after_right,
    );
    let left = RevisionEffectIdeal::new([
        rewrite_event(1, &[], fixture.a.clone()),
        rewrite_event(2, &[], fixture.b.clone()),
        rewrite_event(3, &[], fixture.c.clone()),
    ])
    .unwrap();
    let right = RevisionEffectIdeal::new([rewrite_event(4, &[], right_rewrite)]).unwrap();
    let normalized = RevisionEffectResidualNormalizedLayerWitness {
        left: RewriteConcurrentBranchWitness::Triple(Box::new(RewriteConcurrentTripleWitness {
            cube: fixture.witness,
            ab_composite: ab.into(),
            ac_composite: ac.into(),
            bc_composite: bc.into(),
            final_composite: left_composite.clone().into(),
        })),
        right: RewriteConcurrentBranchWitness::Single,
        right_after_left,
        left_after_right,
    };
    let certificate = left
        .certify_registered_residual_normalized_frontier(
            &right,
            &fixture.registry,
            &sequential,
            normalized.clone(),
        )
        .unwrap();
    assert_eq!(certificate.left_frontier.len(), 3);
    assert_eq!(certificate.right_frontier.len(), 1);
    assert_eq!(certificate.left_normalization.composite(), &left_composite);
    assert_eq!(certificate.common_endpoint(), &9);

    let mixed = left
        .certify_registered_residual_mixed_chain(
            &right,
            &fixture.registry,
            &sequential,
            RevisionEffectResidualMixedChainWitness {
                first: RevisionEffectResidualMixedFirstWitness::Normalized(Box::new(normalized)),
                subsequent: Vec::new(),
            },
        )
        .unwrap();
    assert_eq!(mixed.common_endpoint(), &9);
    assert!(matches!(
        mixed.first,
        RevisionEffectResidualMixedFirstCertificate::Normalized(_)
    ));
}

#[test]
fn normalized_frontier_consumes_three_by_two_without_branch_order_authority() {
    let mut fixture = cube_fixture(26);
    let ab = rewrite(40, 4);
    let ac = rewrite(41, 5);
    let bc = rewrite(42, 6);
    let left_composite = rewrite(43, 7);
    let mut sequential =
        concurrent_triple_sequential_registry(&fixture, &ab, &ac, &bc, &left_composite);

    let right_a = rewrite(50, 8);
    let right_b = rewrite(51, 10);
    let right_b_after_a = rewrite(52, 11);
    let right_a_after_b = rewrite(53, 11);
    let right_composite = rewrite(54, 11);
    register_residual_pair(
        &mut fixture.registry,
        706,
        &right_a,
        &right_b,
        &right_b_after_a,
        &right_a_after_b,
    );
    register_sequential_pair(
        &mut sequential,
        1520,
        &right_a,
        &right_b_after_a,
        &right_composite,
    );
    register_sequential_pair(
        &mut sequential,
        1521,
        &right_b,
        &right_a_after_b,
        &right_composite,
    );

    let right_after_left = rewrite(55, 12);
    let left_after_right = rewrite(56, 12);
    register_residual_pair(
        &mut fixture.registry,
        707,
        &left_composite,
        &right_composite,
        &right_after_left,
        &left_after_right,
    );

    let left = RevisionEffectIdeal::new([
        rewrite_event(1, &[], fixture.a.clone()),
        rewrite_event(2, &[], fixture.b.clone()),
        rewrite_event(3, &[], fixture.c.clone()),
    ])
    .unwrap();
    let right = RevisionEffectIdeal::new([
        rewrite_event(4, &[], right_a),
        rewrite_event(5, &[], right_b),
    ])
    .unwrap();

    let certificate = left
        .certify_registered_residual_normalized_frontier(
            &right,
            &fixture.registry,
            &sequential,
            RevisionEffectResidualNormalizedLayerWitness {
                left: RewriteConcurrentBranchWitness::Triple(Box::new(
                    RewriteConcurrentTripleWitness {
                        cube: fixture.witness,
                        ab_composite: ab.into(),
                        ac_composite: ac.into(),
                        bc_composite: bc.into(),
                        final_composite: left_composite.clone().into(),
                    },
                )),
                right: RewriteConcurrentBranchWitness::Pair(Box::new(
                    RewriteConcurrentPairWitness {
                        right_after_left: right_b_after_a.into(),
                        left_after_right: right_a_after_b.into(),
                        left_then_right_composite: right_composite.clone().into(),
                        right_then_left_composite: right_composite.clone().into(),
                    },
                )),
                right_after_left,
                left_after_right,
            },
        )
        .unwrap();
    assert_eq!(certificate.left_frontier.len(), 3);
    assert_eq!(certificate.right_frontier.len(), 2);
    assert_eq!(certificate.left_normalization.composite(), &left_composite);
    assert_eq!(
        certificate.right_normalization.composite(),
        &right_composite
    );
    assert_eq!(certificate.common_endpoint(), &12);
}

#[test]
fn historical_cube_does_not_imply_full_braid_residual_tuple() {
    let fixture = cube_fixture(26);
    assert!(
        fixture
            .registry
            .certify_cube(&fixture.a, &fixture.b, &fixture.c, fixture.witness.clone(),)
            .is_ok()
    );
    assert_eq!(
        fixture
            .registry
            .certify_braid_cube(&fixture.a, &fixture.b, &fixture.c, fixture.witness,),
        Err(RewriteResidualRegistryError::Coherence(
            RewriteCoherenceError::BraidResidualTupleMismatch,
        ))
    );
}

#[test]
fn braid_cube_certifies_all_three_residual_components() {
    let fixture = braid_cube_fixture();
    let certificate = fixture
        .registry
        .certify_braid_cube(&fixture.a, &fixture.b, &fixture.c, fixture.witness)
        .unwrap();
    assert_eq!(certificate.common_endpoint(), &7);
    assert_eq!(
        certificate.cube.after_a.left_after_right,
        certificate.cube.after_c.right_after_left,
    );
    assert_eq!(
        certificate.cube.after_b.left_after_right,
        certificate.cube.after_c.left_after_right,
    );
}

#[test]
fn registered_cube_checks_all_residual_faces_and_exact_tp2_intent() {
    let fixture = cube_fixture(26);
    let certificate = fixture
        .registry
        .certify_cube(&fixture.a, &fixture.b, &fixture.c, fixture.witness)
        .unwrap();
    assert_eq!(certificate.ab.common_endpoint(), &4);
    assert_eq!(certificate.after_a.common_endpoint(), &7);
    assert_eq!(certificate.after_b.common_endpoint(), &7);
    assert_eq!(certificate.after_c.common_endpoint(), &7);
    assert_eq!(certificate.common_endpoint(), &7);
    assert_eq!(
        certificate.coherent_residual().spec,
        RewriteSpecId(SemanticId(26))
    );

    let fixture = cube_fixture(99);
    assert_eq!(
        fixture
            .registry
            .certify_cube(&fixture.a, &fixture.b, &fixture.c, fixture.witness),
        Err(RewriteResidualRegistryError::Coherence(
            RewriteCoherenceError::CubeResidualIntentMismatch
        ))
    );
}

#[test]
fn registered_cube_rejects_disagreeing_upper_face_endpoint() {
    let mut fixture = cube_fixture(26);
    fixture.witness.b_after_ca = rewrite(29, 8).into();
    fixture.witness.a_after_cb = rewrite(30, 8).into();
    assert_eq!(
        fixture
            .registry
            .certify_cube(&fixture.a, &fixture.b, &fixture.c, fixture.witness,),
        Err(RewriteResidualRegistryError::Coherence(
            RewriteCoherenceError::CubeEndpointMismatch
        ))
    );
}

#[test]
fn non_singleton_frontier_consumes_registered_cube_without_serializing_branch_events() {
    let fixture = cube_fixture(26);
    let left = RevisionEffectIdeal::new(vec![
        rewrite_event(1, &[], fixture.a.clone()),
        rewrite_event(2, &[], fixture.b.clone()),
    ])
    .unwrap();
    let right = RevisionEffectIdeal::new(vec![rewrite_event(3, &[], fixture.c.clone())]).unwrap();

    let certificate = left
        .certify_registered_residual_cube_frontier(&right, &fixture.registry, fixture.witness)
        .unwrap();
    assert_eq!(
        certificate.left_frontier,
        BTreeSet::from([RevisionEffectId(1), RevisionEffectId(2)])
    );
    assert_eq!(
        certificate.right_frontier,
        BTreeSet::from([RevisionEffectId(3)])
    );
    assert_eq!(certificate.common_endpoint(), &7);
}

fn rewrite_event(
    id: u128,
    prerequisites: &[u128],
    rewrite: PreparedRewrite<i32, ()>,
) -> RevisionEffect<SharedPreparedRewrite<i32, ()>> {
    RevisionEffect {
        id: RevisionEffectId(id),
        prerequisites: prerequisites
            .iter()
            .copied()
            .map(RevisionEffectId)
            .collect(),
        payload: rewrite.into(),
    }
}

#[test]
fn registered_residual_frontier_consumes_every_required_pair() {
    let root = rewrite_event(1, &[], rewrite(1, 0));
    let left_rewrite = rewrite(10, 1);
    let right_rewrite = rewrite(11, 2);
    let right_after_left = rewrite(12, 3);
    let left_after_right = rewrite(13, 3);
    let left =
        RevisionEffectIdeal::new([root.clone(), rewrite_event(2, &[1], left_rewrite.clone())])
            .unwrap();
    let right =
        RevisionEffectIdeal::new([root, rewrite_event(3, &[1], right_rewrite.clone())]).unwrap();
    let mut registry = RewriteResidualFamilyRegistry::default();
    registry
        .register(RewriteResidualFamilySpec {
            id: RewriteResidualFamilyId(SemanticId(700)),
            key: RewriteResidualFamilyKey {
                left: (&left_rewrite).into(),
                right: (&right_rewrite).into(),
            },
            right_after_left: (&right_after_left).into(),
            left_after_right: (&left_after_right).into(),
        })
        .unwrap();

    let certificate = left
        .certify_registered_residual_frontier(
            &right,
            &registry,
            |_, _| PairCoordinationDecision::RequiresCoordination,
            |left_id, right_id| {
                (left_id == RevisionEffectId(2) && right_id == RevisionEffectId(3))
                    .then(|| (right_after_left.clone(), left_after_right.clone()))
            },
        )
        .unwrap();
    assert_eq!(certificate.common, BTreeSet::from([RevisionEffectId(1)]));
    assert_eq!(
        certificate.left_frontier,
        BTreeSet::from([RevisionEffectId(2)])
    );
    assert_eq!(
        certificate.right_frontier,
        BTreeSet::from([RevisionEffectId(3)])
    );
    assert_eq!(
        certificate.diamonds[&(RevisionEffectId(2), RevisionEffectId(3))].common_endpoint(),
        &3
    );
}

#[test]
fn compiled_coordination_frontier_skips_proven_disjoint_pairs() {
    use std::cell::Cell;

    let root = rewrite_event(1, &[], rewrite(1, 0));
    let interacting_left = rewrite(10, 1);
    let disjoint_left = rewrite(20, 4);
    let right_rewrite = rewrite(11, 2);
    let right_after_left = rewrite(12, 3);
    let left_after_right = rewrite(13, 3);
    let left = RevisionEffectIdeal::new([
        root.clone(),
        rewrite_event(2, &[1], interacting_left.clone()),
        rewrite_event(4, &[1], disjoint_left.clone()),
    ])
    .unwrap();
    let right =
        RevisionEffectIdeal::new([root, rewrite_event(3, &[1], right_rewrite.clone())]).unwrap();

    let field = |id| SemanticWriteCoordinate::ProductField(SemanticId(id));
    let spec = |id, coordinate| RewriteSpec {
        id: RewriteSpecId(SemanticId(id)),
        law_set: RewriteLawSetId(SemanticId(900)),
        footprint: RewriteFootprint {
            writes: [(field(coordinate), RewriteActionLaw::Opaque)]
                .into_iter()
                .collect(),
            ..RewriteFootprint::default()
        },
    };
    let mut coordination = RewriteCoordinationRegistry::default();
    coordination.register(&spec(10, 1)).unwrap();
    coordination.register(&spec(20, 2)).unwrap();
    coordination.register(&spec(11, 1)).unwrap();

    let mut residual_registry = RewriteResidualFamilyRegistry::default();
    register_residual_pair(
        &mut residual_registry,
        1700,
        &interacting_left,
        &right_rewrite,
        &right_after_left,
        &left_after_right,
    );

    let calls = Cell::new(0_usize);
    let certificate = left
        .certify_compiled_residual_frontier(
            &right,
            &residual_registry,
            &coordination,
            |left_id, right_id| {
                calls.set(calls.get() + 1);
                (left_id == RevisionEffectId(2) && right_id == RevisionEffectId(3))
                    .then(|| (right_after_left.clone(), left_after_right.clone()))
            },
        )
        .unwrap();

    assert_eq!(calls.get(), 1);
    assert_eq!(certificate.diamonds.len(), 1);
    assert!(
        certificate
            .diamonds
            .contains_key(&(RevisionEffectId(2), RevisionEffectId(3)))
    );
    assert!(
        !certificate
            .diamonds
            .contains_key(&(RevisionEffectId(4), RevisionEffectId(3)))
    );
}

#[test]
fn residual_frontier_rejects_missing_pair_and_deeper_causal_suffix() {
    let root = rewrite_event(1, &[], rewrite(1, 0));
    let left_rewrite = rewrite(10, 1);
    let right_rewrite = rewrite(11, 2);
    let left =
        RevisionEffectIdeal::new([root.clone(), rewrite_event(2, &[1], left_rewrite.clone())])
            .unwrap();
    let right =
        RevisionEffectIdeal::new([root.clone(), rewrite_event(3, &[1], right_rewrite.clone())])
            .unwrap();
    assert_eq!(
        left.certify_registered_residual_frontier(
            &right,
            &RewriteResidualFamilyRegistry::default(),
            |_, _| PairCoordinationDecision::RequiresCoordination,
            |_, _| None,
        ),
        Err(RevisionEffectResidualLayerError::MissingResidualPair(
            RevisionEffectId(2),
            RevisionEffectId(3),
        ))
    );

    let deeper_left = RevisionEffectIdeal::new([
        root,
        rewrite_event(2, &[1], left_rewrite),
        rewrite_event(4, &[2], rewrite(14, 4)),
    ])
    .unwrap();
    let classifier_calls = std::cell::Cell::new(0_usize);
    assert_eq!(
        deeper_left.certify_registered_residual_frontier(
            &right,
            &RewriteResidualFamilyRegistry::default(),
            |_, _| {
                classifier_calls.set(classifier_calls.get() + 1);
                PairCoordinationDecision::CoordinationFree
            },
            |_, _| None,
        ),
        Err(RevisionEffectResidualLayerError::NonFrontierExclusiveEffect(RevisionEffectId(4)))
    );
    assert_eq!(classifier_calls.get(), 0);
}

#[test]
fn reic_branch_exclusive_events_use_fail_closed_pair_coordination() {
    let root = event(1, &[], "root");
    let left = RevisionEffectIdeal::new([root.clone(), event(2, &[1], "left")]).unwrap();
    let right = RevisionEffectIdeal::new([root, event(3, &[1], "right")]).unwrap();

    let unknown = left
        .merge_requirements(&right, |_, _| {
            PairCoordinationDecision::RequiresCoordination
        })
        .unwrap();
    assert_eq!(unknown.common, BTreeSet::from([RevisionEffectId(1)]));
    assert_eq!(
        unknown.requires_residual,
        BTreeSet::from([(RevisionEffectId(2), RevisionEffectId(3))])
    );
    assert!(!unknown.coordination_free());

    let conflict = left
        .merge_requirements(&right, |_, _| PairCoordinationDecision::IntentConflict)
        .unwrap();
    assert_eq!(
        conflict.intent_conflicts,
        BTreeSet::from([(RevisionEffectId(2), RevisionEffectId(3))])
    );

    let safe = left
        .merge_requirements(&right, |_, _| PairCoordinationDecision::CoordinationFree)
        .unwrap();
    assert!(safe.coordination_free());
}
