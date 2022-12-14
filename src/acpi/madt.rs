use core::mem;
use core::ops::DerefMut;

// use crate::memory::{allocate_frames, Frame};
// use crate::paging::{ActivePageTable, Page, PageFlags, PhysicalAddress, VirtualAddress};

use x86_64::structures::paging::{
    Mapper, OffsetPageTable, Page, PageTableFlags, PhysFrame, Size4KiB,
};
use x86_64::{PhysAddr, VirtAddr};

use super::find_sdt;
use super::sdt::Sdt;

use core::intrinsics::{atomic_load_seqcst as atomic_load, atomic_store_seqcst as atomic_store};
use core::sync::atomic::Ordering;

use crate::device::local_apic::LOCAL_APIC;
// use crate::interrupt;
use crate::ap_init::{AP_READY, CPU_COUNT};
use crate::kstart_ap;
use crate::memory::FRAME_ALLOC;
use crate::{serial_print, serial_println};

/// The Multiple APIC Descriptor Table
#[derive(Clone, Copy, Debug)]
pub struct Madt {
    sdt: &'static Sdt,
    pub local_address: u32,
    pub flags: u32,
}

const TRAMPOLINE: u64 = 0x8000; // must match value in trampoline.asm
static TRAMPOLINE_DATA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/trampoline"));

pub static mut MADT: Option<Madt> = None;
pub const FLAG_PCAT: u32 = 1;

impl Madt {
    pub fn init(active_table: &mut OffsetPageTable) {
        let madt_sdt = find_sdt("APIC");
        let madt = if madt_sdt.len() == 1 {
            Madt::new(madt_sdt[0])
        } else {
            serial_println!("Unable to find MADT");
            return;
        };

        if let Some(madt) = madt {
            // safe because no APs have been started yet.
            unsafe { MADT = Some(madt) };

            serial_println!("  APIC: {:>08X}: {}", madt.local_address, madt.flags);

            let local_apic = unsafe { &mut LOCAL_APIC };
            let me = local_apic.id() as u8;

            if local_apic.x2 {
                serial_println!("    X2APIC {}", me);
            } else {
                serial_println!("    XAPIC {}: {:>08X}", me, local_apic.address);
            }

            // if cfg!(feature = "multi_core") {
            if true {
                // Map trampoline
                let trampoline_frame =
                    PhysFrame::<Size4KiB>::containing_address(PhysAddr::new(TRAMPOLINE));
                let trampoline_page = Page::containing_address(VirtAddr::new(TRAMPOLINE));
                let result = unsafe {
                    active_table
                        .map_to(
                            trampoline_page,
                            trampoline_frame,
                            PageTableFlags::PRESENT | PageTableFlags::WRITABLE,
                            FRAME_ALLOC.lock().deref_mut(),
                        )
                        .unwrap()
                }; //TODO: do not have writable and executable!
                result.flush();

                // Write trampoline, make sure TRAMPOLINE page is free for use
                for i in 0..TRAMPOLINE_DATA.len() {
                    unsafe {
                        atomic_store((TRAMPOLINE as *mut u8).add(i), TRAMPOLINE_DATA[i]);
                    }
                }

                for madt_entry in madt.iter() {
                    serial_println!("      {:?}", madt_entry);
                    match madt_entry {
                        MadtEntry::LocalApic(ap_local_apic) => {
                            if ap_local_apic.id == me {
                                serial_println!("        This is my local APIC");
                            } else {
                                if ap_local_apic.flags & 1 == 1 {
                                    // Increase CPU ID
                                    CPU_COUNT.fetch_add(1, Ordering::SeqCst);

                                    // Allocate a stack
                                    let stack_start =
                                        { FRAME_ALLOC.lock().allocate_contiguous_frames(64) }
                                            .expect("no more frames in acpi stack_start")
                                            .as_u64()
                                            + crate::PHYS_OFFSET;
                                    let stack_end = stack_start + 64 * 4096;

                                    let ap_ready = (TRAMPOLINE + 8) as *mut u64;
                                    let ap_cpu_id = unsafe { ap_ready.offset(1) };
                                    let ap_page_table = unsafe { ap_ready.offset(2) };
                                    let ap_stack_start = unsafe { ap_ready.offset(3) };
                                    let ap_stack_end = unsafe { ap_ready.offset(4) };
                                    let ap_code = unsafe { ap_ready.offset(5) };

                                    // Set the ap_ready to 0, volatile
                                    unsafe { atomic_store(ap_ready, 0) };
                                    unsafe { atomic_store(ap_cpu_id, ap_local_apic.id as u64) };
                                    unsafe {
                                        atomic_store(
                                            ap_page_table,
                                            crate::memory::phys_addr_of(active_table).as_u64(),
                                        )
                                    };
                                    unsafe { atomic_store(ap_stack_start, stack_start as u64) };
                                    unsafe { atomic_store(ap_stack_end, stack_end as u64) };
                                    unsafe { atomic_store(ap_code, kstart_ap as u64) };
                                    AP_READY.store(false, Ordering::SeqCst);

                                    serial_print!("        AP {}:", ap_local_apic.id);

                                    // Send INIT IPI
                                    {
                                        let mut icr = 0x4500;
                                        if local_apic.x2 {
                                            icr |= (ap_local_apic.id as u64) << 32;
                                        } else {
                                            icr |= (ap_local_apic.id as u64) << 56;
                                        }
                                        serial_print!(" IPI...");
                                        local_apic.set_icr(icr);
                                    }

                                    // Send START IPI
                                    {
                                        //Start at 0x0800:0000 => 0x8000. Hopefully the bootloader code is still there
                                        let ap_segment = (TRAMPOLINE >> 12) & 0xFF;
                                        let mut icr = 0x4600 | ap_segment as u64;

                                        if local_apic.x2 {
                                            icr |= (ap_local_apic.id as u64) << 32;
                                        } else {
                                            icr |= (ap_local_apic.id as u64) << 56;
                                        }

                                        serial_print!(" SIPI...");
                                        local_apic.set_icr(icr);
                                    }

                                    // Wait for trampoline ready
                                    serial_print!(" Wait...");
                                    while unsafe { atomic_load(ap_ready) } == 0 {
                                        unsafe { core::arch::x86_64::_mm_pause() };
                                    }
                                    serial_print!(" Trampoline...");
                                    while !AP_READY.load(Ordering::SeqCst) {
                                        unsafe { core::arch::x86_64::_mm_pause() };
                                    }
                                    serial_println!(" Ready");

                                    // active_table.flush_all();
                                    x86_64::instructions::tlb::flush_all();
                                } else {
                                    serial_println!("        CPU Disabled");
                                }
                            }
                        }
                        _ => (),
                    }
                }

                // Unmap trampoline
                let (_frame, result) = active_table.unmap(trampoline_page).unwrap();
                result.flush();
            }
        }
    }

    pub fn new(sdt: &'static Sdt) -> Option<Madt> {
        if &sdt.signature == b"APIC" && sdt.data_len() >= 8 {
            //Not valid if no local address and flags
            let local_address = unsafe { *(sdt.data_address() as *const u32) };
            let flags = unsafe { *(sdt.data_address() as *const u32).offset(1) };

            Some(Madt {
                sdt: sdt,
                local_address: local_address,
                flags: flags,
            })
        } else {
            None
        }
    }

    pub fn iter(&self) -> MadtIter {
        MadtIter {
            sdt: self.sdt,
            i: 8, // Skip local controller address and flags
        }
    }
}

/// MADT Local APIC
#[derive(Clone, Copy, Debug)]
#[repr(packed)]
pub struct MadtLocalApic {
    /// Processor ID
    pub processor: u8,
    /// Local APIC ID
    pub id: u8,
    /// Flags. 1 means that the processor is enabled
    pub flags: u32,
}

/// MADT I/O APIC
#[derive(Clone, Copy, Debug)]
#[repr(packed)]
pub struct MadtIoApic {
    /// I/O APIC ID
    pub id: u8,
    /// reserved
    reserved: u8,
    /// I/O APIC address
    pub address: u32,
    /// Global system interrupt base
    pub gsi_base: u32,
}

/// MADT Interrupt Source Override
#[derive(Clone, Copy, Debug)]
#[repr(packed)]
pub struct MadtIntSrcOverride {
    /// Bus Source
    pub bus_source: u8,
    /// IRQ Source
    pub irq_source: u8,
    /// Global system interrupt base
    pub gsi_base: u32,
    /// Flags
    pub flags: u16,
}

/// MADT Entries
#[derive(Debug)]
pub enum MadtEntry {
    LocalApic(&'static MadtLocalApic),
    InvalidLocalApic(usize),
    IoApic(&'static MadtIoApic),
    InvalidIoApic(usize),
    IntSrcOverride(&'static MadtIntSrcOverride),
    InvalidIntSrcOverride(usize),
    Unknown(u8),
}

pub struct MadtIter {
    sdt: &'static Sdt,
    i: usize,
}

impl Iterator for MadtIter {
    type Item = MadtEntry;
    fn next(&mut self) -> Option<Self::Item> {
        if self.i + 1 < self.sdt.data_len() {
            let entry_type = unsafe { *(self.sdt.data_address() as *const u8).add(self.i) };
            let entry_len =
                unsafe { *(self.sdt.data_address() as *const u8).add(self.i + 1) } as usize;

            if self.i + entry_len <= self.sdt.data_len() {
                let item = match entry_type {
                    0 => {
                        if entry_len == mem::size_of::<MadtLocalApic>() + 2 {
                            MadtEntry::LocalApic(unsafe {
                                &*((self.sdt.data_address() + self.i + 2) as *const MadtLocalApic)
                            })
                        } else {
                            MadtEntry::InvalidLocalApic(entry_len)
                        }
                    }
                    1 => {
                        if entry_len == mem::size_of::<MadtIoApic>() + 2 {
                            MadtEntry::IoApic(unsafe {
                                &*((self.sdt.data_address() + self.i + 2) as *const MadtIoApic)
                            })
                        } else {
                            MadtEntry::InvalidIoApic(entry_len)
                        }
                    }
                    2 => {
                        if entry_len == mem::size_of::<MadtIntSrcOverride>() + 2 {
                            MadtEntry::IntSrcOverride(unsafe {
                                &*((self.sdt.data_address() + self.i + 2)
                                    as *const MadtIntSrcOverride)
                            })
                        } else {
                            MadtEntry::InvalidIntSrcOverride(entry_len)
                        }
                    }
                    _ => MadtEntry::Unknown(entry_type),
                };

                self.i += entry_len;

                Some(item)
            } else {
                None
            }
        } else {
            None
        }
    }
}
