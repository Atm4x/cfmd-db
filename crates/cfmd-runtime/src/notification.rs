use std::{sync::Arc, task::Waker};

use crate::RelationId;

/// Wake-only notification backend for committed Revision publication.
///
/// Notifications are not database state and carry no mutation authority. A
/// backend may emit duplicate or spurious wakes; watches always recover the
/// authoritative transition from durable history before producing an event.
pub trait PublicationNotifier: std::fmt::Debug + Send + Sync {
    #[must_use]
    fn generation(&self) -> u64;

    #[must_use]
    fn generation_for(&self, _dependencies: &[RelationId]) -> u64 {
        self.generation()
    }

    #[must_use]
    fn wait_after(&self, observed: u64) -> u64;

    #[must_use]
    fn wait_after_relations(&self, observed: u64, _dependencies: &[RelationId]) -> u64 {
        self.wait_after(observed)
    }

    /// Registers or refreshes one executor waker and returns the generation
    /// sampled after registration. Implementations must make this race-free
    /// with [`Self::notify_waiters`].
    #[must_use]
    fn register_waker_after(&self, waiter_id: u64, observed: u64, waker: &Waker) -> u64;

    #[must_use]
    fn register_waker_after_relations(
        &self,
        waiter_id: u64,
        observed: u64,
        _dependencies: &[RelationId],
        waker: &Waker,
    ) -> u64 {
        self.register_waker_after(waiter_id, observed, waker)
    }

    fn unregister_waker(&self, waiter_id: u64);

    /// Wakes waiters without claiming that a Revision was published.
    ///
    /// Cancellation, shutdown and host-specific liveness signals use this path.
    /// Correctness never depends on the wake itself: subscribers re-read durable
    /// Revision/history authority before emitting an event.
    fn notify_waiters(&self);

    /// Runtime publication uses the same wake primitive after durable authority
    /// has advanced. Providers never receive mutation authority through this call.
    fn notify_revision_published(&self) {
        self.notify_waiters();
    }

    fn notify_relations_published(&self, _relations: &[RelationId]) {
        self.notify_revision_published();
    }
}

/// Standard single-process notifier used when a host wants an explicit,
/// shareable notification provider.
#[derive(Debug, Default)]
pub struct InProcessPublicationNotifier {
    inner: kernel_plan::InProcessRevisionPublicationNotifier,
}

impl PublicationNotifier for InProcessPublicationNotifier {
    fn generation(&self) -> u64 {
        kernel_plan::RuntimeRevisionPublicationNotifier::generation(&self.inner)
    }

    fn generation_for(&self, dependencies: &[RelationId]) -> u64 {
        let dependencies = dependencies
            .iter()
            .copied()
            .map(Into::into)
            .collect::<Vec<_>>();
        kernel_plan::RuntimeRevisionPublicationNotifier::generation_for(&self.inner, &dependencies)
    }

    fn wait_after(&self, observed: u64) -> u64 {
        kernel_plan::RuntimeRevisionPublicationNotifier::wait_after(&self.inner, observed)
    }

    fn wait_after_relations(&self, observed: u64, dependencies: &[RelationId]) -> u64 {
        let dependencies = dependencies
            .iter()
            .copied()
            .map(Into::into)
            .collect::<Vec<_>>();
        kernel_plan::RuntimeRevisionPublicationNotifier::wait_after_relations(
            &self.inner,
            observed,
            &dependencies,
        )
    }

    fn register_waker_after(&self, waiter_id: u64, observed: u64, waker: &Waker) -> u64 {
        kernel_plan::RuntimeRevisionPublicationNotifier::register_waker_after(
            &self.inner,
            waiter_id,
            observed,
            waker,
        )
    }

    fn register_waker_after_relations(
        &self,
        waiter_id: u64,
        observed: u64,
        dependencies: &[RelationId],
        waker: &Waker,
    ) -> u64 {
        let dependencies = dependencies
            .iter()
            .copied()
            .map(Into::into)
            .collect::<Vec<_>>();
        kernel_plan::RuntimeRevisionPublicationNotifier::register_waker_after_relations(
            &self.inner,
            waiter_id,
            observed,
            &dependencies,
            waker,
        )
    }

    fn unregister_waker(&self, waiter_id: u64) {
        kernel_plan::RuntimeRevisionPublicationNotifier::unregister_waker(&self.inner, waiter_id);
    }

    fn notify_waiters(&self) {
        kernel_plan::RuntimeRevisionPublicationNotifier::notify_waiters(&self.inner);
    }

    fn notify_relations_published(&self, relations: &[RelationId]) {
        let relations = relations
            .iter()
            .copied()
            .map(Into::into)
            .collect::<Vec<_>>();
        kernel_plan::RuntimeRevisionPublicationNotifier::notify_relations_published(
            &self.inner,
            &relations,
        );
    }
}

#[derive(Debug)]
pub(crate) struct KernelPublicationNotifierBridge {
    inner: Arc<dyn PublicationNotifier>,
}

impl KernelPublicationNotifierBridge {
    pub(crate) fn new(inner: Arc<dyn PublicationNotifier>) -> Self {
        Self { inner }
    }
}

impl kernel_plan::RuntimeRevisionPublicationNotifier for KernelPublicationNotifierBridge {
    fn generation(&self) -> u64 {
        self.inner.generation()
    }

    fn generation_for(&self, dependencies: &[kernel_types::SemanticId]) -> u64 {
        let dependencies = dependencies
            .iter()
            .map(|relation| RelationId::new(relation.raw()))
            .collect::<Vec<_>>();
        self.inner.generation_for(&dependencies)
    }

    fn wait_after(&self, observed: u64) -> u64 {
        self.inner.wait_after(observed)
    }

    fn wait_after_relations(
        &self,
        observed: u64,
        dependencies: &[kernel_types::SemanticId],
    ) -> u64 {
        let dependencies = dependencies
            .iter()
            .map(|relation| RelationId::new(relation.raw()))
            .collect::<Vec<_>>();
        self.inner.wait_after_relations(observed, &dependencies)
    }

    fn register_waker_after(&self, waiter_id: u64, observed: u64, waker: &Waker) -> u64 {
        self.inner.register_waker_after(waiter_id, observed, waker)
    }

    fn register_waker_after_relations(
        &self,
        waiter_id: u64,
        observed: u64,
        dependencies: &[kernel_types::SemanticId],
        waker: &Waker,
    ) -> u64 {
        let dependencies = dependencies
            .iter()
            .map(|relation| RelationId::new(relation.raw()))
            .collect::<Vec<_>>();
        self.inner
            .register_waker_after_relations(waiter_id, observed, &dependencies, waker)
    }

    fn unregister_waker(&self, waiter_id: u64) {
        self.inner.unregister_waker(waiter_id);
    }

    fn notify_waiters(&self) {
        self.inner.notify_waiters();
    }

    fn notify_relations_published(&self, relations: &[kernel_types::SemanticId]) {
        let relations = relations
            .iter()
            .map(|relation| RelationId::new(relation.raw()))
            .collect::<Vec<_>>();
        self.inner.notify_relations_published(&relations);
    }
}
