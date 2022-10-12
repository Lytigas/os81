use bootloader::boot_info::{MemoryRegionKind, MemoryRegions};
use x86_64::{
    structures::paging::{FrameAllocator, OffsetPageTable, PageTable, PhysFrame, Size4KiB},
    PhysAddr, VirtAddr,
};

/// Initialize a new OffsetPageTable.
///
/// This function is unsafe because the caller must guarantee that the
/// complete physical memory is mapped to virtual memory at the passed
/// `physical_memory_offset`. Also, this function must be only called once
/// to avoid aliasing `&mut` references (which is undefined behavior).
pub unsafe fn init(physical_memory_offset: VirtAddr) -> OffsetPageTable<'static> {
    let level_4_table = active_level_4_table(physical_memory_offset);
    OffsetPageTable::new(level_4_table, physical_memory_offset)
}

pub fn phys_addr_of(table: &mut OffsetPageTable) -> PhysAddr {
    let virt_addr = table.level_4_table() as *mut _ as u64;
    let phys_offset = table.phys_offset().as_u64();
    // virt_addr == phys_offset + physical_addr
    PhysAddr::new(virt_addr - phys_offset)
}

/// Returns a mutable reference to the active level 4 table.
///
/// This function is unsafe because the caller must guarantee that the
/// complete physical memory is mapped to virtual memory at the passed
/// `physical_memory_offset`. Also, this function must be only called once
/// to avoid aliasing `&mut` references (which is undefined behavior).
unsafe fn active_level_4_table(physical_memory_offset: VirtAddr) -> &'static mut PageTable {
    use x86_64::registers::control::Cr3;

    let (level_4_table_frame, _) = Cr3::read();

    let phys = level_4_table_frame.start_address();
    let virt = physical_memory_offset + phys.as_u64();
    let page_table_ptr: *mut PageTable = virt.as_mut_ptr();

    &mut *page_table_ptr // unsafe
}

// pub fn print_lookup_path(level_4_table: &PageTable, addr: u64) {
//     let page_table_walker = PageTableWa
//     let p4 = &self.level_4_table;
//     let p3 = self.page_table_walker.next_table(&p4[page.p4_index()])?;
//     let p2 = self.page_table_walker.next_table(&p3[page.p3_index()])?;
//     let p1 = self.page_table_walker.next_table(&p2[page.p2_index()])?;

//     let p1_entry = &p1[page.p1_index()];

//     if p1_entry.is_unused() {
//         return Err(TranslateError::PageNotMapped);
//     }

//     PhysFrame::from_start_address(p1_entry.addr())
//         .map_err(|AddressNotAligned| TranslateError::InvalidFrameAddress(p1_entry.addr()))
//     const MASK: u64 = (1 << 9) - 1;
//     let l4 = (addr >> 39) & MASK;
//     let l3 = (addr >> 30) & MASK;
//     let l2 = (addr >> 21) & MASK;
//     let l1 = (addr >> 12) & MASK;

//     let l4_entry = &level_4_table[l4 as usize];
//     serial_println!("l4 entry: {:?}", l4_entry);

//     let level_3_table = l4_entry.

// }

/// A FrameAllocator that always returns `None`.
pub struct EmptyFrameAllocator;

unsafe impl FrameAllocator<Size4KiB> for EmptyFrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame> {
        None
    }
}

pub static FRAME_ALLOC: spin::Mutex<BootInfoFrameAllocator> =
    spin::Mutex::new(BootInfoFrameAllocator {
        memory_map: None,
        next: usize::MAX,
    });

/// SAFETY: same preconditions as BootInfoFrameAllocator::init
pub unsafe fn init_frame_alloc(memory_map: &'static MemoryRegions) {
    let new_allocator = unsafe { BootInfoFrameAllocator::init(memory_map) };
    let mut guard = FRAME_ALLOC.lock();
    let old_allocator = core::mem::replace(&mut *guard, new_allocator);
    // make sure to not call Drop because the initial value is invalid
    core::mem::forget(old_allocator);
}

/// A FrameAllocator that returns usable frames from the bootloader's memory map.
pub struct BootInfoFrameAllocator {
    memory_map: Option<&'static MemoryRegions>,
    next: usize,
}

unsafe impl Send for BootInfoFrameAllocator {}

impl BootInfoFrameAllocator {
    /// Create a FrameAllocator from the passed memory map.
    ///
    /// This function is unsafe because the caller must guarantee that the passed
    /// memory map is valid. The main requirement is that all frames that are marked
    /// as `USABLE` in it are really unused.
    pub unsafe fn init(memory_map: &'static MemoryRegions) -> Self {
        BootInfoFrameAllocator {
            memory_map: Some(memory_map),
            next: 0,
        }
    }

    /// Returns an iterator over the usable frames specified in the memory map.
    fn usable_frames(&self) -> impl Iterator<Item = PhysFrame> {
        // get usable regions from memory map
        let regions = self.memory_map.unwrap().iter();
        let usable_regions = regions.filter(|r| r.kind == MemoryRegionKind::Usable);
        // map each region to its address range
        let addr_ranges = usable_regions.map(|r| r.start..r.end);
        // transform to an iterator of frame start addresses
        let frame_addresses = addr_ranges.flat_map(|r| r.step_by(4096));
        // create `PhysFrame` types from the start addresses
        frame_addresses.map(|addr| PhysFrame::containing_address(PhysAddr::new(addr)))
    }

    /// Allocate count contigous frames, return the start address of the first frame.
    pub fn allocate_contiguous_frames(&mut self, count: usize) -> Option<PhysAddr> {
        // Be really dumb: burn frames until we find a contiguous region
        assert_ne!(count, 0);
        loop {
            let mut prev_frame = self.allocate_frame()?;
            let base_addr = prev_frame.start_address();

            let mut remaining = count - 1;
            while remaining > 0 {
                let new_frame = self.allocate_frame()?;
                if prev_frame.start_address() + prev_frame.size() != new_frame.start_address() {
                    break;
                } else {
                    prev_frame = new_frame;
                    remaining -= 1;
                }
            }
            if remaining <= 0 {
                return Some(base_addr);
            }
        }
    }
}

unsafe impl FrameAllocator<Size4KiB> for BootInfoFrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame> {
        let frame = self.usable_frames().nth(self.next);
        self.next += 1;
        frame
    }
}
