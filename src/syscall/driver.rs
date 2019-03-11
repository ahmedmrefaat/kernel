use interrupt::syscall::SyscallStack;
use memory::{allocate_frames, deallocate_frames, Frame};
use paging::{ActivePageTable, PhysicalAddress, VirtualAddress, PageTableType, VAddrType};
use paging::entry::EntryFlags;
use context;
use context::memory::Grant;
use syscall::error::{Error, EFAULT, EINVAL, ENOMEM, EPERM, ESRCH, Result};
use syscall::flag::{PHYSMAP_WRITE, PHYSMAP_WRITE_COMBINE};

fn enforce_root() -> Result<()> {
    let contexts = context::contexts();
    let context_lock = contexts.current().ok_or(Error::new(ESRCH))?;
    let context = context_lock.read();
    if context.euid == 0 {
        Ok(())
    } else {
        Err(Error::new(EPERM))
    }
}

pub fn iopl(level: usize, stack: &mut SyscallStack) -> Result<usize> {
    enforce_root()?;

    if level > 3 {
        return Err(Error::new(EINVAL));
    }

    stack.rflags = (stack.rflags & !(3 << 12)) | ((level & 3) << 12);

    Ok(0)
}

pub fn inner_physalloc(size: usize) -> Result<usize> {
    allocate_frames((size + 4095)/4096).ok_or(Error::new(ENOMEM)).map(|frame| frame.start_address().get())
}
pub fn physalloc(size: usize) -> Result<usize> {
    enforce_root()?;
    inner_physalloc(size)
}

pub fn inner_physfree(physical_address: usize, size: usize) -> Result<usize> {
    deallocate_frames(Frame::containing_address(PhysicalAddress::new(physical_address)), (size + 4095)/4096);

    //TODO: Check that no double free occured
    Ok(0)
}
pub fn physfree(physical_address: usize, size: usize) -> Result<usize> {
    enforce_root()?;
    inner_physfree(physical_address, size)
}

//TODO: verify exlusive access to physical memory
pub fn inner_physmap(physical_address: usize, size: usize, flags: usize) -> Result<usize> {
    //TODO: Abstract with other grant creation
    if size == 0 {
        Ok(0)
    } else {
        let contexts = context::contexts();
        let context_lock = contexts.current().ok_or(Error::new(ESRCH))?;
        let context = context_lock.read();

        let mut grants = context.grants.lock();

        let from_address = (physical_address/4096) * 4096;
        let offset = physical_address - from_address;
        let full_size = ((offset + size + 4095)/4096) * 4096;
        let mut to_address = ::USER_GRANT_OFFSET;

        let mut entry_flags = EntryFlags::PRESENT | EntryFlags::NO_EXECUTE | EntryFlags::USER_ACCESSIBLE;
        if flags & PHYSMAP_WRITE == PHYSMAP_WRITE {
            entry_flags |= EntryFlags::WRITABLE;
        }
        if flags & PHYSMAP_WRITE_COMBINE == PHYSMAP_WRITE_COMBINE {
            entry_flags |= EntryFlags::HUGE_PAGE;
        }

        let mut i = 0;
        while i < grants.len() {
            let start = grants[i].start_address().get();
            if to_address + full_size < start {
                break;
            }

            let pages = (grants[i].size() + 4095) / 4096;
            let end = start + pages * 4096;
            to_address = end;
            i += 1;
        }

        grants.insert(i, Grant::physmap(
            PhysicalAddress::new(from_address),
            VirtualAddress::new(to_address),
            full_size,
            entry_flags
        ));

        Ok(to_address + offset)
    }
}
pub fn physmap(physical_address: usize, size: usize, flags: usize) -> Result<usize> {
    enforce_root()?;
    inner_physmap(physical_address, size, flags)
}

pub fn inner_physunmap(virtual_address: usize) -> Result<usize> {
    if virtual_address == 0 {
        Ok(0)
    } else {
        let contexts = context::contexts();
        let context_lock = contexts.current().ok_or(Error::new(ESRCH))?;
        let context = context_lock.read();

        let mut grants = context.grants.lock();

        for i in 0 .. grants.len() {
            let start = grants[i].start_address().get();
            let end = start + grants[i].size();
            if virtual_address >= start && virtual_address < end {
                grants.remove(i).unmap();

                return Ok(0);
            }
        }

        Err(Error::new(EFAULT))
    }
}
pub fn physunmap(virtual_address: usize) -> Result<usize> {
    enforce_root()?;
    inner_physunmap(virtual_address)
}

pub fn virttophys(virtual_address: usize) -> Result<usize> {
    enforce_root()?;

    let v = VirtualAddress::new(virtual_address);
    let mut active_table: ActivePageTable;
    match v.get_type() {
        VAddrType::User => active_table = unsafe { ActivePageTable::new(PageTableType::User) },
        VAddrType::Kernel => active_table = unsafe { ActivePageTable::new(PageTableType::Kernel) },
    }

    match active_table.translate(VirtualAddress::new(virtual_address)) {
        Some(physical_address) => Ok(physical_address.get()),
        None => Err(Error::new(EFAULT))
    }
}
