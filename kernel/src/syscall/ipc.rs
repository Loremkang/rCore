use crate::sync::Semaphore;
use crate::sync::SpinLock as Mutex;
use alloc::{boxed::Box, collections::BTreeMap, string::String, sync::Arc, sync::Weak, vec::Vec};
use bitflags::*;
use core::cell::UnsafeCell;
use spin::RwLock;

pub use crate::ipc::*;

use rcore_memory::memory_set::MemoryAttr;
use rcore_memory::{PAGE_SIZE, VirtAddr, PhysAddr};
use rcore_memory::memory_set::handler::{Shared, SharedGuard};
use crate::memory::GlobalFrameAlloc;

use super::*;

impl Syscall<'_> {
    pub fn sys_semget(&self, key: usize, nsems: isize, flags: usize) -> SysResult {
        info!("semget: key: {}", key);

        /// The maximum semaphores per semaphore set
        const SEMMSL: usize = 256;

        if nsems < 0 || nsems as usize > SEMMSL {
            return Err(SysError::EINVAL);
        }
        let nsems = nsems as usize;

        let sem_array = SemArray::get_or_create(key, nsems, flags);
        let id = self.process().semaphores.add(sem_array);
        Ok(id)
    }

    pub fn sys_semop(&self, id: usize, ops: *const SemBuf, num_ops: usize) -> SysResult {
        info!("semop: id: {}", id);
        let ops = unsafe { self.vm().check_read_array(ops, num_ops)? };

        let sem_array = self.process().semaphores.get(id).ok_or(SysError::EINVAL)?;
        for &SemBuf { num, op, flags } in ops.iter() {
            let flags = SemFlags::from_bits_truncate(flags);
            if flags.contains(SemFlags::IPC_NOWAIT) {
                unimplemented!("Semaphore: semop.IPC_NOWAIT");
            }
            let sem = &sem_array[num as usize];

            let _result = match op {
                1 => sem.release(),
                -1 => sem.acquire(),
                _ => unimplemented!("Semaphore: semop.(Not 1/-1)"),
            };
            if flags.contains(SemFlags::SEM_UNDO) {
                self.process().semaphores.add_undo(id, num, op);
            }
        }
        Ok(0)
    }

    pub fn sys_semctl(&self, id: usize, num: usize, cmd: usize, arg: isize) -> SysResult {
        info!("semctl: id: {}, num: {}, cmd: {}", id, num, cmd);
        let sem_array = self.process().semaphores.get(id).ok_or(SysError::EINVAL)?;
        let sem = &sem_array[num as usize];

        const GETVAL: usize = 12;
        const GETALL: usize = 13;
        const SETVAL: usize = 16;
        const SETALL: usize = 17;

        match cmd {
            SETVAL => sem.set(arg),
            _ => unimplemented!("Semaphore: Semctl.(Not setval)"),
        }
        Ok(0)
    }

    pub fn sys_shmget(&self, key: usize, size: usize, shmflg: usize) -> SysResult {
        info!("shmget: key: {}", key);

        let sharedGuard = ShmIdentifier::new_sharedGuard(key, size);
        let id = self.process().shmIdentifiers.add(sharedGuard);
        Ok(id)
    }

    pub fn sys_shmat(&self, id: usize, mut addr: VirtAddr, shmflg: usize) -> SysResult {
        
        let mut shmIdentifier = self.process().shmIdentifiers.get(id).ok_or(SysError::EINVAL)?;

        let mut proc = self.process();
        if addr == 0 {
            // although NULL can be a valid address
            // but in C, NULL is regarded as allocation failure
            // so just skip it
            addr = PAGE_SIZE;
        }
        let size = shmIdentifier.sharedGuard.lock().size;
        info!("shmat: id: {}, addr = {:#x}, size = {}", id, addr, size);
        addr = self.vm().find_free_area(addr, size);
        self.vm().push(
            addr,
            addr + size,
            MemoryAttr::default().user().execute().writable(),
            Shared::new_with_guard(GlobalFrameAlloc, shmIdentifier.sharedGuard.clone()),
            "shmat",
        );
        shmIdentifier.addr = addr;
        proc.shmIdentifiers.set(id, shmIdentifier);
        //self.process().shmIdentifiers.setVirtAddr(id, addr);
        return Ok(addr);
    }

    pub fn sys_shmdt(&self, id: usize, addr: VirtAddr, shmflg: usize) -> SysResult {
        info!(
            "shmdt: addr={:#x}", addr
        );
        let mut proc = self.process();
        let optId = proc.shmIdentifiers.getId(addr);
        if let Some(id) = optId {
            proc.shmIdentifiers.pop(id);
        }
        Ok(0)
    }
}

/// An operation to be performed on a single semaphore
///
/// Ref: [http://man7.org/linux/man-pages/man2/semop.2.html]
#[repr(C)]
pub struct SemBuf {
    num: u16,
    op: i16,
    flags: i16,
}

pub union SemctlUnion {
    val: isize,
    buf: usize,   // semid_ds*, unimplemented
    array: usize, // short*, unimplemented
} // unused

bitflags! {
    pub struct SemFlags: i16 {
        /// For SemOP
        const IPC_NOWAIT = 0x800;
        /// it will be automatically undone when the process terminates.
        const SEM_UNDO = 0x1000;
    }
}
