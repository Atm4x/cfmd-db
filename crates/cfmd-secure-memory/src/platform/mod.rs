#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
pub(crate) use linux::Allocation;

#[cfg(not(target_os = "linux"))]
mod unsupported {
    use crate::{MemoryProtection, SecureMemoryError};

    pub(crate) struct Allocation;

    impl Allocation {
        pub(crate) fn new(_secret_len: usize) -> Result<Self, SecureMemoryError> {
            Err(SecureMemoryError::UnsupportedPlatform)
        }

        pub(crate) fn new_aligned(
            _secret_len: usize,
            _alignment: usize,
        ) -> Result<Self, SecureMemoryError> {
            Err(SecureMemoryError::UnsupportedPlatform)
        }

        pub(crate) fn emplace<T>(&mut self, _value: T) {
            unreachable!("unsupported platform cannot create an allocation")
        }

        pub(crate) fn value_ref<T>(&self) -> &T {
            unreachable!("unsupported platform cannot create an allocation")
        }

        pub(crate) fn value_mut<T>(&mut self) -> &mut T {
            unreachable!("unsupported platform cannot create an allocation")
        }

        pub(crate) fn drop_value<T>(&mut self) {
            unreachable!("unsupported platform cannot create an allocation")
        }

        pub(crate) fn protection(&self) -> MemoryProtection {
            MemoryProtection {
                page_locked: false,
                native_crash_dump_excluded: false,
                guard_pages: false,
            }
        }

        pub(crate) fn as_array<const N: usize>(&self) -> &[u8; N] {
            unreachable!("unsupported platform cannot create an allocation")
        }

        pub(crate) fn as_array_mut<const N: usize>(&mut self) -> &mut [u8; N] {
            unreachable!("unsupported platform cannot create an allocation")
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub(crate) use unsupported::Allocation;
