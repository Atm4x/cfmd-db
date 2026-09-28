use std::sync::Arc;

/// Wake-only notification backend for committed Revision publication.
///
/// Notifications are not database state and carry no mutation authority. A
/// backend may emit duplicate or spurious wakes; watches always recover the
/// authoritative transition from durable history before producing an event.
pub trait PublicationNotifier: std::fmt::Debug + Send + Sync {
    #[must_use]
    fn generation(&self) -> u64;

    #[must_use]
    fn wait_after(&self, observed: u64) -> u64;

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

    fn wait_after(&self, observed: u64) -> u64 {
        kernel_plan::RuntimeRevisionPublicationNotifier::wait_after(&self.inner, observed)
    }

    fn notify_waiters(&self) {
        kernel_plan::RuntimeRevisionPublicationNotifier::notify_waiters(&self.inner);
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

    fn wait_after(&self, observed: u64) -> u64 {
        self.inner.wait_after(observed)
    }

    fn notify_waiters(&self) {
        self.inner.notify_waiters();
    }
}
