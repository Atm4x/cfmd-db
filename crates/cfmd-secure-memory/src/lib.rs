//! Page-backed secret memory for CFMD.
//!
//! This crate is deliberately the only low-level platform-memory boundary for
//! secrets. Callers receive scoped access to secret bytes rather than an owned
//! byte array that can be copied through ordinary Rust value semantics.

use std::{
    fmt, io,
    marker::PhantomData,
    mem::{align_of, size_of},
    sync::Arc,
};

use zeroize::Zeroizing;

mod platform;

/// Properties that were actually established for a secure allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryProtection {
    page_locked: bool,
    native_crash_dump_excluded: bool,
    guard_pages: bool,
}

impl MemoryProtection {
    #[must_use]
    pub const fn page_locked(self) -> bool {
        self.page_locked
    }

    #[must_use]
    pub const fn native_crash_dump_excluded(self) -> bool {
        self.native_crash_dump_excluded
    }

    #[must_use]
    pub const fn guard_pages(self) -> bool {
        self.guard_pages
    }

    #[must_use]
    pub const fn is_hardened(self) -> bool {
        self.page_locked && self.native_crash_dump_excluded && self.guard_pages
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecureMemoryOperation {
    ComputeLayout,
    QueryPageSize,
    MapPages,
    EnableDataPages,
    LockPages,
    ExcludeFromDump,
}

impl fmt::Display for SecureMemoryOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ComputeLayout => "compute allocation layout",
            Self::QueryPageSize => "query page size",
            Self::MapPages => "map guarded pages",
            Self::EnableDataPages => "enable secret data pages",
            Self::LockPages => "lock secret pages",
            Self::ExcludeFromDump => "exclude secret pages from dumps",
        })
    }
}

#[derive(Debug)]
pub enum SecureMemoryError {
    UnsupportedPlatform,
    EmptySecret,
    Platform {
        operation: SecureMemoryOperation,
        source: io::Error,
    },
}

impl SecureMemoryError {
    #[must_use]
    pub const fn operation(&self) -> Option<SecureMemoryOperation> {
        match self {
            Self::Platform { operation, .. } => Some(*operation),
            Self::UnsupportedPlatform | Self::EmptySecret => None,
        }
    }
}

impl fmt::Display for SecureMemoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedPlatform => {
                formatter.write_str("secure memory is unsupported on this platform")
            }
            Self::EmptySecret => {
                formatter.write_str("secure memory cannot allocate a zero-length secret")
            }
            Self::Platform { operation, source } => {
                write!(formatter, "secure memory failed to {operation}: {source}")
            }
        }
    }
}

impl std::error::Error for SecureMemoryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Platform { source, .. } => Some(source),
            Self::UnsupportedPlatform | Self::EmptySecret => None,
        }
    }
}

/// Fixed-size secret bytes stored in locked, dump-excluded guarded pages.
///
/// The type is intentionally neither `Clone` nor `Copy`. Access is scoped by a
/// closure so ordinary callers do not receive an owned secret byte array.
pub struct SecureBytes<const N: usize> {
    allocation: platform::Allocation,
}

impl<const N: usize> SecureBytes<N> {
    pub fn try_zeroed() -> Result<Self, SecureMemoryError> {
        if N == 0 {
            return Err(SecureMemoryError::EmptySecret);
        }
        Ok(Self {
            allocation: platform::Allocation::new(N)?,
        })
    }

    pub fn try_from_array(bytes: [u8; N]) -> Result<Self, SecureMemoryError> {
        let bytes = Zeroizing::new(bytes);
        let mut secret = Self::try_zeroed()?;
        secret.with_secret_mut(|destination| destination.copy_from_slice(bytes.as_ref()));
        Ok(secret)
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        N
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        N == 0
    }

    #[must_use]
    pub fn protection(&self) -> MemoryProtection {
        platform::Allocation::protection()
    }

    pub fn with_secret<R>(&self, operation: impl FnOnce(&[u8; N]) -> R) -> R {
        operation(self.allocation.as_array::<N>())
    }

    pub fn with_secret_mut<R>(&mut self, operation: impl FnOnce(&mut [u8; N]) -> R) -> R {
        operation(self.allocation.as_array_mut::<N>())
    }
}

impl<const N: usize> fmt::Debug for SecureBytes<N> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecureBytes")
            .field("len", &N)
            .field("protection", &self.protection())
            .field("contents", &"<redacted>")
            .finish()
    }
}

/// A value whose complete object representation lives in hardened pages.
///
/// Construction scrubs the moved-from source storage after ownership is copied into
/// the secure mapping. Destruction runs `T`'s destructor while pages are still locked
/// and then scrubs the complete backing pages before unmapping.
pub struct SecureBox<T> {
    allocation: platform::Allocation,
    _type: PhantomData<T>,
}

impl<T> SecureBox<T> {
    pub fn try_new(value: T) -> Result<Self, SecureMemoryError> {
        Self::try_new_with(|| value)
    }

    pub fn try_new_with(constructor: impl FnOnce() -> T) -> Result<Self, SecureMemoryError> {
        if size_of::<T>() == 0 {
            return Err(SecureMemoryError::EmptySecret);
        }
        let mut allocation = platform::Allocation::new_aligned(size_of::<T>(), align_of::<T>())?;
        allocation.emplace(constructor());
        Ok(Self {
            allocation,
            _type: PhantomData,
        })
    }

    #[must_use]
    pub fn protection(&self) -> MemoryProtection {
        platform::Allocation::protection()
    }
}

impl<T> SecureBox<T> {
    pub fn with_secret<R>(&self, operation: impl FnOnce(&T) -> R) -> R {
        operation(self.allocation.value_ref())
    }

    pub fn with_secret_mut<R>(&mut self, operation: impl FnOnce(&mut T) -> R) -> R {
        operation(self.allocation.value_mut())
    }
}

impl<T> fmt::Debug for SecureBox<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecureBox")
            .field("size", &size_of::<T>())
            .field("protection", &self.protection())
            .field("contents", &"<redacted>")
            .finish()
    }
}

impl<T> Drop for SecureBox<T> {
    fn drop(&mut self) {
        self.allocation.drop_value::<T>();
    }
}

/// Cloneable opaque ownership of a long-lived secret object.
///
/// Cloning this type only clones the handle; it never clones `T`. The secret
/// object remains in one hardened mapping for the lifetime of all handles.
/// Access is read-only and closure-scoped, so ownership cannot be moved out.
/// Callers that require non-extractable state must store a non-`Clone` opaque
/// context; `with_secret` cannot prevent an explicitly cloneable `T` from being
/// cloned by code that is deliberately given `&T`.
pub struct SecretHandle<T> {
    inner: Arc<SecureBox<T>>,
}

impl<T> SecretHandle<T> {
    pub fn try_new(value: T) -> Result<Self, SecureMemoryError> {
        SecureBox::try_new(value).map(Self::from_box)
    }

    pub fn try_new_with(constructor: impl FnOnce() -> T) -> Result<Self, SecureMemoryError> {
        SecureBox::try_new_with(constructor).map(Self::from_box)
    }

    fn from_box(inner: SecureBox<T>) -> Self {
        Self {
            inner: Arc::new(inner),
        }
    }

    #[must_use]
    pub fn protection(&self) -> MemoryProtection {
        self.inner.protection()
    }

    pub fn with_secret<R>(&self, operation: impl FnOnce(&T) -> R) -> R {
        self.inner.with_secret(operation)
    }
}

impl<T> From<SecureBox<T>> for SecretHandle<T> {
    fn from(value: SecureBox<T>) -> Self {
        Self::from_box(value)
    }
}

impl<T> Clone for SecretHandle<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<T> fmt::Debug for SecretHandle<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecretHandle")
            .field("size", &size_of::<T>())
            .field("protection", &self.protection())
            .field("contents", &"<redacted>")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hardened_secret_round_trips_without_exposing_owned_bytes() {
        let secret = SecureBytes::<32>::try_from_array([0xA5; 32]).unwrap();
        assert_eq!(secret.with_secret(|bytes| bytes[0]), 0xA5);
        assert_eq!(secret.with_secret(|bytes| bytes[31]), 0xA5);
        assert!(secret.protection().is_hardened());
        assert!(format!("{secret:?}").contains("<redacted>"));
        assert!(!format!("{secret:?}").contains("165"));
    }

    #[test]
    fn mutable_access_is_scoped() {
        let mut secret = SecureBytes::<32>::try_zeroed().unwrap();
        secret.with_secret_mut(|bytes| bytes.fill(7));
        assert!(secret.with_secret(|bytes| bytes.iter().all(|byte| *byte == 7)));
    }

    #[test]
    fn secure_box_keeps_value_in_hardened_mapping() {
        let secret = SecureBox::try_new([0x5A_u8; 64]).unwrap();
        assert_eq!(secret.with_secret(|value| value[0]), 0x5A);
        assert_eq!(secret.with_secret(|value| value[63]), 0x5A);
        assert!(secret.protection().is_hardened());
    }

    #[test]
    fn secret_handle_clone_shares_one_protected_object() {
        let secret = SecretHandle::try_new([0x33_u8; 32]).unwrap();
        let clone = secret.clone();
        let left = secret.with_secret(|value| value.as_ptr());
        let right = clone.with_secret(|value| value.as_ptr());
        assert_eq!(left, right);
        assert!(secret.protection().is_hardened());
        assert!(format!("{secret:?}").contains("<redacted>"));
    }
}
