use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use x86_64::structures::paging::OffsetPageTable;

pub static CPU_COUNT: AtomicUsize = AtomicUsize::new(0);
pub static AP_READY: AtomicBool = AtomicBool::new(false);
pub static BSP_READY: AtomicBool = AtomicBool::new(false);

pub fn init_aps(active_table: &mut OffsetPageTable) {
    unsafe {
        crate::acpi::init(active_table, None);
        crate::device::init_after_acpi(active_table);
    }

    BSP_READY.store(true, Ordering::SeqCst);
}

#[repr(packed)]
pub struct KernelArgsAp {
    pub cpu_id: u64,
    pub page_table: u64,
    pub stack_start: u64,
    pub stack_end: u64,
}
