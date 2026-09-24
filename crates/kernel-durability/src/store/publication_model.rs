use std::collections::BTreeSet;

use super::StoreFaultPoint;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Prerequisite {
    Checkpoint,
    Wal,
    Metadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PublishPhase {
    Building,
    PrerequisiteDirectorySynced,
    PendingManifestSynced,
    ManifestRenamed,
    ManifestDirectorySynced,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CandidateGeneration {
    generation: u8,
    file_synced: BTreeSet<Prerequisite>,
    phase: PublishPhase,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PublicationModel {
    durable_components: BTreeSet<u8>,
    durable_manifests: BTreeSet<u8>,
    candidate: Option<CandidateGeneration>,
    gc_remove_attempts: BTreeSet<(u8, DurableEntry)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum DurableEntry {
    Components,
    Manifest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModelBoundary {
    CheckpointFileSynced,
    WalFileSynced,
    MetadataFileSynced,
    PrerequisiteDirectorySynced,
    PendingManifestFileSynced,
    ManifestRenameUncertain,
    ManifestDirectorySynced,
    BeforeGcRemove,
    AfterGcRemoveUncertain,
    GcDirectorySynced,
}

fn production_boundary(point: StoreFaultPoint) -> ModelBoundary {
    match point {
        StoreFaultPoint::AfterCheckpointSync => ModelBoundary::CheckpointFileSynced,
        StoreFaultPoint::AfterWalSync => ModelBoundary::WalFileSynced,
        StoreFaultPoint::AfterMetadataSync => ModelBoundary::MetadataFileSynced,
        StoreFaultPoint::AfterPrerequisiteDirectorySync => {
            ModelBoundary::PrerequisiteDirectorySynced
        }
        StoreFaultPoint::AfterPendingManifestSync => ModelBoundary::PendingManifestFileSynced,
        StoreFaultPoint::AfterManifestRename => ModelBoundary::ManifestRenameUncertain,
        StoreFaultPoint::AfterManifestDirectorySync => ModelBoundary::ManifestDirectorySynced,
        StoreFaultPoint::BeforeCompactionRemove => ModelBoundary::BeforeGcRemove,
        StoreFaultPoint::AfterCompactionRemove => ModelBoundary::AfterGcRemoveUncertain,
        StoreFaultPoint::AfterCompactionDirectorySync => ModelBoundary::GcDirectorySynced,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CrashImage {
    components: BTreeSet<u8>,
    manifests: BTreeSet<u8>,
}

impl CrashImage {
    fn authority(&self) -> Option<u8> {
        self.manifests.iter().next_back().copied()
    }

    fn authority_is_recoverable(&self) -> bool {
        self.authority()
            .is_none_or(|generation| self.components.contains(&generation))
    }
}

impl PublicationModel {
    fn with_published_generation(generation: u8) -> Self {
        Self {
            durable_components: BTreeSet::from([generation]),
            durable_manifests: BTreeSet::from([generation]),
            candidate: None,
            gc_remove_attempts: BTreeSet::new(),
        }
    }

    fn begin(&mut self, generation: u8) {
        assert!(self.candidate.is_none());
        assert!(
            self.durable_manifests
                .iter()
                .all(|current| *current < generation)
        );
        self.candidate = Some(CandidateGeneration {
            generation,
            file_synced: BTreeSet::new(),
            phase: PublishPhase::Building,
        });
    }

    fn sync_file(&mut self, prerequisite: Prerequisite) {
        let candidate = self.candidate.as_mut().expect("candidate exists");
        assert_eq!(candidate.phase, PublishPhase::Building);
        candidate.file_synced.insert(prerequisite);
    }

    fn sync_prerequisite_directory(&mut self) {
        let candidate = self.candidate.as_mut().expect("candidate exists");
        assert_eq!(candidate.phase, PublishPhase::Building);
        assert_eq!(candidate.file_synced.len(), 3);
        self.durable_components.insert(candidate.generation);
        candidate.phase = PublishPhase::PrerequisiteDirectorySynced;
    }

    fn sync_pending_manifest(&mut self) {
        let candidate = self.candidate.as_mut().expect("candidate exists");
        assert_eq!(candidate.phase, PublishPhase::PrerequisiteDirectorySynced);
        candidate.phase = PublishPhase::PendingManifestSynced;
    }

    fn rename_manifest(&mut self) {
        let candidate = self.candidate.as_mut().expect("candidate exists");
        assert_eq!(candidate.phase, PublishPhase::PendingManifestSynced);
        candidate.phase = PublishPhase::ManifestRenamed;
    }

    fn sync_manifest_directory(&mut self) {
        let candidate = self.candidate.as_mut().expect("candidate exists");
        assert_eq!(candidate.phase, PublishPhase::ManifestRenamed);
        self.durable_manifests.insert(candidate.generation);
        candidate.phase = PublishPhase::ManifestDirectorySynced;
    }

    fn finish_publication(&mut self) {
        let candidate = self.candidate.as_ref().expect("candidate exists");
        assert_eq!(candidate.phase, PublishPhase::ManifestDirectorySynced);
        self.candidate = None;
    }

    fn attempt_gc_remove(&mut self, generation: u8, entry: DurableEntry) {
        let authority = self
            .durable_manifests
            .iter()
            .next_back()
            .copied()
            .expect("published authority exists");
        assert_ne!(generation, authority, "GC must never target authority");
        self.gc_remove_attempts.insert((generation, entry));
    }

    fn sync_gc_directory(&mut self) {
        for (generation, entry) in std::mem::take(&mut self.gc_remove_attempts) {
            match entry {
                DurableEntry::Components => {
                    self.durable_components.remove(&generation);
                }
                DurableEntry::Manifest => {
                    self.durable_manifests.remove(&generation);
                }
            }
        }
    }

    fn crash_images(&self) -> Vec<CrashImage> {
        let mut images = vec![CrashImage {
            components: self.durable_components.clone(),
            manifests: self.durable_manifests.clone(),
        }];

        // POSIX-style rename is atomic in the live namespace, but without the
        // post-rename directory fsync a crash may expose either the pre-rename
        // or post-rename namespace. The production protocol deliberately syncs
        // all prerequisite file contents and their directory entries first, so
        // observing the post-rename image early is still safe.
        if let Some(candidate) = &self.candidate
            && candidate.phase == PublishPhase::ManifestRenamed
        {
            let mut post_rename = images[0].clone();
            post_rename.manifests.insert(candidate.generation);
            images.push(post_rename);
        }

        // remove(2) before the final directory fsync has the same persistence
        // uncertainty. Model every subset of attempted obsolete removals: this
        // is stronger than assuming all-or-nothing GC persistence.
        for &(generation, entry) in &self.gc_remove_attempts {
            let snapshot = images.clone();
            for mut image in snapshot {
                match entry {
                    DurableEntry::Components => {
                        image.components.remove(&generation);
                    }
                    DurableEntry::Manifest => {
                        image.manifests.remove(&generation);
                    }
                }
                images.push(image);
            }
        }
        images.sort_by(|left, right| {
            left.manifests
                .cmp(&right.manifests)
                .then(left.components.cmp(&right.components))
        });
        images.dedup();
        images
    }

    fn assert_crash_safe(&self) {
        for image in self.crash_images() {
            assert!(
                image.authority_is_recoverable(),
                "selected authority must retain every prerequisite: {image:?}"
            );
        }
    }
}

fn advance_publication_and_check(model: &mut PublicationModel, generation: u8) {
    model.begin(generation);
    model.assert_crash_safe();
    for prerequisite in [
        Prerequisite::Checkpoint,
        Prerequisite::Wal,
        Prerequisite::Metadata,
    ] {
        model.sync_file(prerequisite);
        model.assert_crash_safe();
    }
    model.sync_prerequisite_directory();
    model.assert_crash_safe();
    model.sync_pending_manifest();
    model.assert_crash_safe();
    model.rename_manifest();
    model.assert_crash_safe();
    model.sync_manifest_directory();
    model.assert_crash_safe();
    assert_eq!(
        model.crash_images(),
        vec![CrashImage {
            components: model.durable_components.clone(),
            manifests: model.durable_manifests.clone(),
        }]
    );
    model.finish_publication();
}

#[test]
fn publication_protocol_preserves_unique_recoverable_authority_at_every_crash_cut() {
    let mut model = PublicationModel::with_published_generation(1);
    advance_publication_and_check(&mut model, 2);
    assert_eq!(model.crash_images()[0].authority(), Some(2));
    advance_publication_and_check(&mut model, 3);
    assert_eq!(model.crash_images()[0].authority(), Some(3));
}

#[test]
fn pending_manifest_is_never_authority_and_rename_uncertainty_is_closed_by_prerequisite_sync() {
    let mut model = PublicationModel::with_published_generation(1);
    model.begin(2);
    for prerequisite in [
        Prerequisite::Checkpoint,
        Prerequisite::Wal,
        Prerequisite::Metadata,
    ] {
        model.sync_file(prerequisite);
    }
    model.sync_prerequisite_directory();
    model.sync_pending_manifest();
    assert!(
        model
            .crash_images()
            .iter()
            .all(|image| image.authority() == Some(1))
    );

    model.rename_manifest();
    let authorities = model
        .crash_images()
        .into_iter()
        .map(|image| {
            assert!(image.authority_is_recoverable());
            image.authority().expect("authority exists")
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(authorities, BTreeSet::from([1, 2]));
}

#[test]
fn gc_crash_projection_cannot_remove_the_published_authority() {
    let mut model = PublicationModel::with_published_generation(1);
    advance_publication_and_check(&mut model, 2);
    model.attempt_gc_remove(1, DurableEntry::Components);
    model.assert_crash_safe();
    model.attempt_gc_remove(1, DurableEntry::Manifest);
    model.assert_crash_safe();
    assert!(
        model
            .crash_images()
            .iter()
            .all(|image| image.authority() == Some(2))
    );
    model.sync_gc_directory();
    model.assert_crash_safe();
    assert_eq!(model.durable_manifests, BTreeSet::from([2]));
    assert_eq!(model.durable_components, BTreeSet::from([2]));
}

#[test]
fn production_fault_points_cover_every_model_publication_and_gc_boundary() {
    let mapped = [
        (
            StoreFaultPoint::AfterCheckpointSync,
            ModelBoundary::CheckpointFileSynced,
        ),
        (StoreFaultPoint::AfterWalSync, ModelBoundary::WalFileSynced),
        (
            StoreFaultPoint::AfterMetadataSync,
            ModelBoundary::MetadataFileSynced,
        ),
        (
            StoreFaultPoint::AfterPrerequisiteDirectorySync,
            ModelBoundary::PrerequisiteDirectorySynced,
        ),
        (
            StoreFaultPoint::AfterPendingManifestSync,
            ModelBoundary::PendingManifestFileSynced,
        ),
        (
            StoreFaultPoint::AfterManifestRename,
            ModelBoundary::ManifestRenameUncertain,
        ),
        (
            StoreFaultPoint::AfterManifestDirectorySync,
            ModelBoundary::ManifestDirectorySynced,
        ),
        (
            StoreFaultPoint::BeforeCompactionRemove,
            ModelBoundary::BeforeGcRemove,
        ),
        (
            StoreFaultPoint::AfterCompactionRemove,
            ModelBoundary::AfterGcRemoveUncertain,
        ),
        (
            StoreFaultPoint::AfterCompactionDirectorySync,
            ModelBoundary::GcDirectorySynced,
        ),
    ];
    for (point, expected) in mapped {
        assert_eq!(production_boundary(point), expected);
    }
}
