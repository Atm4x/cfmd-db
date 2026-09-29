use std::{
    io,
    mem::{ManuallyDrop, align_of, size_of},
    ptr::{self, NonNull},
    sync::atomic::{Ordering, compiler_fence},
};

use crate::{MemoryProtection, SecureMemoryError, SecureMemoryOperation};

pub(crate) struct Allocation {
    base: NonNull<libc::c_void>,
    data: NonNull<u8>,
    data_len: usize,
    mapped_len: usize,
}

impl Allocation {
    pub(crate) fn new(secret_len: usize) -> Result<Self, SecureMemoryError> {
        Self::new_aligned(secret_len, 1)
    }

    pub(crate) fn new_aligned(
        secret_len: usize,
        alignment: usize,
    ) -> Result<Self, SecureMemoryError> {
        let page_size = page_size()?;
        if alignment > page_size || !alignment.is_power_of_two() {
            return Err(platform_error(
                SecureMemoryOperation::ComputeLayout,
                io::Error::other("secure allocation alignment exceeds page alignment"),
            ));
        }
        let data_len = secret_len
            .checked_add(page_size - 1)
            .and_then(|len| len.checked_div(page_size))
            .and_then(|pages| pages.checked_mul(page_size))
            .ok_or_else(|| {
                platform_error(
                    SecureMemoryOperation::ComputeLayout,
                    io::Error::other("secure allocation size overflow"),
                )
            })?;
        let mapped_len = data_len
            .checked_add(page_size.checked_mul(2).ok_or_else(|| {
                platform_error(
                    SecureMemoryOperation::ComputeLayout,
                    io::Error::other("secure guard size overflow"),
                )
            })?)
            .ok_or_else(|| {
                platform_error(
                    SecureMemoryOperation::ComputeLayout,
                    io::Error::other("secure mapping size overflow"),
                )
            })?;

        // SAFETY: The mapping is anonymous/private, the length is non-zero and checked,
        // and its lifetime is owned exclusively by the returned Allocation.
        let raw = unsafe {
            libc::mmap(
                ptr::null_mut(),
                mapped_len,
                libc::PROT_NONE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                -1,
                0,
            )
        };
        if raw == libc::MAP_FAILED {
            return Err(platform_error(
                SecureMemoryOperation::MapPages,
                io::Error::last_os_error(),
            ));
        }
        let base = NonNull::new(raw).expect("mmap returned a non-null non-MAP_FAILED address");
        let data = NonNull::new(base.as_ptr().cast::<u8>().wrapping_add(page_size))
            .expect("offset from a valid mapping cannot be null");

        // SAFETY: data points to the interior data pages of the mapping and data_len
        // exactly covers those pages, leaving one PROT_NONE guard page on each side.
        if unsafe {
            libc::mprotect(
                data.as_ptr().cast(),
                data_len,
                libc::PROT_READ | libc::PROT_WRITE,
            )
        } != 0
        {
            let error = io::Error::last_os_error();
            unmap(base, mapped_len);
            return Err(platform_error(
                SecureMemoryOperation::EnableDataPages,
                error,
            ));
        }

        // SAFETY: data/data_len identify valid mapped pages owned by this allocation.
        if unsafe { libc::mlock(data.as_ptr().cast(), data_len) } != 0 {
            let error = io::Error::last_os_error();
            unmap(base, mapped_len);
            return Err(platform_error(SecureMemoryOperation::LockPages, error));
        }

        // SAFETY: MADV_DONTDUMP applies to the valid data-page range and changes only
        // kernel dump policy, not pointer validity or Rust aliasing.
        if unsafe { libc::madvise(data.as_ptr().cast(), data_len, libc::MADV_DONTDUMP) } != 0 {
            let error = io::Error::last_os_error();
            // SAFETY: the pages were successfully locked above and are still mapped.
            unsafe { libc::munlock(data.as_ptr().cast(), data_len) };
            unmap(base, mapped_len);
            return Err(platform_error(
                SecureMemoryOperation::ExcludeFromDump,
                error,
            ));
        }

        Ok(Self {
            base,
            data,
            data_len,
            mapped_len,
        })
    }

    pub(crate) const fn protection() -> MemoryProtection {
        MemoryProtection {
            page_locked: true,
            native_crash_dump_excluded: true,
            guard_pages: true,
        }
    }

    pub(crate) fn emplace<T>(&mut self, value: T) {
        debug_assert!(size_of::<T>() <= self.data_len);
        debug_assert!(align_of::<T>() <= page_size().unwrap_or(1));
        let mut source = ManuallyDrop::new(value);
        // SAFETY: destination is page-aligned writable storage large enough for T.
        // The bitwise copy transfers ownership to the secure mapping; source is a
        // ManuallyDrop and is scrubbed immediately rather than dropped twice.
        unsafe {
            ptr::copy_nonoverlapping::<T>(&raw const *source, self.data.as_ptr().cast::<T>(), 1);
            ptr::write_bytes((&raw mut *source).cast::<u8>(), 0, size_of::<T>());
        }
        compiler_fence(Ordering::SeqCst);
    }

    pub(crate) fn value_ref<T>(&self) -> &T {
        // SAFETY: SecureBox calls this only after emplace<T> initialized the mapping.
        unsafe { &*self.data.as_ptr().cast::<T>() }
    }

    pub(crate) fn value_mut<T>(&mut self) -> &mut T {
        // SAFETY: &mut self is exclusive and SecureBox initialized the same T.
        unsafe { &mut *self.data.as_ptr().cast::<T>() }
    }

    pub(crate) fn drop_value<T>(&mut self) {
        // SAFETY: SecureBox calls this exactly once for the initialized T before the
        // allocation itself is scrubbed and unmapped.
        unsafe { ptr::drop_in_place(self.data.as_ptr().cast::<T>()) };
    }

    pub(crate) fn as_array<const N: usize>(&self) -> &[u8; N] {
        debug_assert!(N <= self.data_len);
        // SAFETY: the data mapping is readable for at least data_len bytes, N is
        // bounded by data_len, and shared access preserves aliasing rules.
        unsafe { &*self.data.as_ptr().cast::<[u8; N]>() }
    }

    pub(crate) fn as_array_mut<const N: usize>(&mut self) -> &mut [u8; N] {
        debug_assert!(N <= self.data_len);
        // SAFETY: &mut self gives exclusive access to the allocation; the data mapping
        // is writable for at least data_len bytes and N is bounded by data_len.
        unsafe { &mut *self.data.as_ptr().cast::<[u8; N]>() }
    }
}

// SAFETY: Allocation owns a disjoint mapping. Shared access is read-only and mutable
// access requires &mut self, so moving/sharing the owner does not weaken aliasing.
unsafe impl Send for Allocation {}
// SAFETY: See Send justification; interior bytes are not mutated through &self.
unsafe impl Sync for Allocation {}

impl Drop for Allocation {
    fn drop(&mut self) {
        // Volatile stores plus a compiler fence make secret erasure observable before
        // the mapping is unlocked/unmapped.
        for offset in 0..self.data_len {
            // SAFETY: every offset is within the writable data-page mapping.
            unsafe { ptr::write_volatile(self.data.as_ptr().add(offset), 0) };
        }
        compiler_fence(Ordering::SeqCst);

        // SAFETY: data/data_len remain a valid locked mapping until munmap below.
        unsafe { libc::munlock(self.data.as_ptr().cast(), self.data_len) };
        unmap(self.base, self.mapped_len);
    }
}

fn page_size() -> Result<usize, SecureMemoryError> {
    // SAFETY: sysconf with _SC_PAGESIZE has no pointer arguments and no Rust aliasing
    // requirements.
    let raw = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    usize::try_from(raw)
        .ok()
        .filter(|size| *size > 0 && size.is_power_of_two())
        .ok_or_else(|| {
            platform_error(
                SecureMemoryOperation::QueryPageSize,
                io::Error::last_os_error(),
            )
        })
}

fn platform_error(operation: SecureMemoryOperation, source: io::Error) -> SecureMemoryError {
    SecureMemoryError::Platform { operation, source }
}

fn unmap(base: NonNull<libc::c_void>, mapped_len: usize) {
    // SAFETY: base/mapped_len describe the complete mapping created by mmap. This
    // helper is called exactly once on each ownership path.
    unsafe {
        libc::munmap(base.as_ptr(), mapped_len);
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn proc_smaps_confirms_locked_and_dontdump_flags() {
        let allocation = Allocation::new(32).unwrap();
        let address = allocation.data.as_ptr() as usize;
        let smaps = fs::read_to_string("/proc/self/smaps").unwrap();
        let mut lines = smaps.lines().peekable();
        let mut vm_flags = None;

        while let Some(line) = lines.next() {
            let Some((range, _rest)) = line.split_once(' ') else {
                continue;
            };
            let Some((start, end)) = range.split_once('-') else {
                continue;
            };
            let Ok(start) = usize::from_str_radix(start, 16) else {
                continue;
            };
            let Ok(end) = usize::from_str_radix(end, 16) else {
                continue;
            };
            if !(start..end).contains(&address) {
                continue;
            }
            for detail in lines.by_ref() {
                if let Some(flags) = detail.strip_prefix("VmFlags:") {
                    vm_flags = Some(flags.split_whitespace().collect::<Vec<_>>());
                    break;
                }
            }
            break;
        }

        let flags = vm_flags.expect("secure data mapping must appear in /proc/self/smaps");
        assert!(
            flags.contains(&"lo"),
            "secure mapping must be locked: {flags:?}"
        );
        assert!(
            flags.contains(&"dd"),
            "secure mapping must be excluded from dumps: {flags:?}"
        );
    }
}
