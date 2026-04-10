use std::ptr;

use nix::sys::mman::{mmap_anonymous, mprotect, munmap, MapFlags, ProtFlags};
use tracing::{debug, info, warn};

use crate::WorkerError;

/// A shared memory segment created via mmap.
///
/// Used to store OPcodes, class tables, function tables, and interned strings.
/// After initialization, the segment is sealed as read-only via mprotect.
/// Workers inherit this via fork + Copy-on-Write — zero copy cost.
pub struct SharedMemory {
    ptr: *mut u8,
    len: usize,
    sealed: bool,
}

// SAFETY: The shared memory region is designed to be inherited by forked
// processes. After sealing, it is read-only and safe to share.
unsafe impl Send for SharedMemory {}
unsafe impl Sync for SharedMemory {}

impl SharedMemory {
    /// Create a new anonymous shared memory segment.
    ///
    /// The segment is initially read-write. Call `seal()` after populating it.
    pub fn new(size: usize) -> Result<Self, WorkerError> {
        info!(
            size_bytes = size,
            size_mb = size / (1024 * 1024),
            "Creating shared memory segment"
        );

        let ptr = unsafe {
            mmap_anonymous(
                None,
                size.try_into().expect("size fits in NonZero<usize>"),
                ProtFlags::PROT_READ | ProtFlags::PROT_WRITE,
                MapFlags::MAP_SHARED | MapFlags::MAP_ANON,
            )
            .map_err(WorkerError::SharedMemoryCreate)?
        };

        debug!(ptr = ?ptr, size = size, "Shared memory mapped");

        Ok(SharedMemory {
            ptr: ptr.as_ptr() as *mut u8,
            len: size,
            sealed: false,
        })
    }

    /// Seal the shared memory as read-only via mprotect.
    ///
    /// After sealing, any write attempt from a worker causes SIGBUS — fail fast.
    pub fn seal(&mut self) -> Result<(), WorkerError> {
        if self.sealed {
            warn!("Shared memory already sealed");
            return Ok(());
        }

        info!(size = self.len, "Sealing shared memory (PROT_READ)");

        unsafe {
            let addr = std::ptr::NonNull::new(self.ptr as *mut libc::c_void)
                .expect("mmap returned non-null pointer");
            mprotect(addr, self.len, ProtFlags::PROT_READ)
                .map_err(WorkerError::SharedMemorySeal)?;
        }

        self.sealed = true;
        debug!("Shared memory sealed — writes will cause SIGBUS");
        Ok(())
    }

    /// Check if the memory is sealed (read-only).
    pub fn is_sealed(&self) -> bool {
        self.sealed
    }

    /// Get a raw pointer to the shared memory region.
    ///
    /// # Safety
    /// Caller must ensure writes only happen before `seal()`.
    pub fn as_ptr(&self) -> *const u8 {
        self.ptr as *const u8
    }

    /// Get a mutable pointer to the shared memory region.
    ///
    /// # Safety
    /// Only valid before `seal()` is called. Writing after seal causes SIGBUS.
    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        assert!(
            !self.sealed,
            "Cannot get mutable pointer to sealed shared memory"
        );
        self.ptr
    }

    /// Get the size of the shared memory segment in bytes.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Write data into the shared memory at the given offset.
    ///
    /// # Panics
    /// Panics if sealed or if offset + data.len() exceeds the segment size.
    pub fn write(&mut self, offset: usize, data: &[u8]) {
        assert!(!self.sealed, "Cannot write to sealed shared memory");
        assert!(
            offset + data.len() <= self.len,
            "Write exceeds shared memory bounds: offset={offset}, len={}, capacity={}",
            data.len(),
            self.len
        );

        unsafe {
            ptr::copy_nonoverlapping(data.as_ptr(), self.ptr.add(offset), data.len());
        }
    }

    /// Read data from the shared memory at the given offset.
    pub fn read(&self, offset: usize, len: usize) -> &[u8] {
        assert!(
            offset + len <= self.len,
            "Read exceeds shared memory bounds"
        );

        unsafe { std::slice::from_raw_parts(self.ptr.add(offset) as *const u8, len) }
    }
}

impl Drop for SharedMemory {
    fn drop(&mut self) {
        debug!(size = self.len, "Unmapping shared memory");
        unsafe {
            let _ = munmap(
                std::ptr::NonNull::new(self.ptr as *mut libc::c_void)
                    .expect("mmap returned non-null"),
                self.len,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_write_read_seal() {
        let mut shm = SharedMemory::new(4096).expect("create shared memory");
        assert!(!shm.is_sealed());

        let data = b"Hello from shared memory!";
        shm.write(0, data);

        let read_back = shm.read(0, data.len());
        assert_eq!(read_back, data);

        shm.seal().expect("seal shared memory");
        assert!(shm.is_sealed());

        // Reading after seal should still work
        let read_after_seal = shm.read(0, data.len());
        assert_eq!(read_after_seal, data);
    }

    #[test]
    #[should_panic(expected = "Cannot write to sealed")]
    fn write_after_seal_panics() {
        let mut shm = SharedMemory::new(4096).expect("create");
        shm.seal().expect("seal");
        shm.write(0, b"boom");
    }
}
