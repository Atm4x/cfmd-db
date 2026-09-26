impl RuntimeRevisionSnapshot {
    #[must_use]
    pub fn root(&self) -> &RuntimeRevisionBundle {
        self.root.as_ref()
    }

    #[must_use]
    pub fn root_version(&self) -> RuntimeRootVersion {
        self.root.root_identity.version
    }
}

impl std::ops::Deref for RuntimeRevisionSnapshot {
    type Target = RuntimeRevisionBundle;

    fn deref(&self) -> &Self::Target {
        self.root.as_ref()
    }
}

