#![no_std]
#![cfg_attr(test, no_main)]
#![feature(custom_test_frameworks)]
#![feature(abi_x86_interrupt)]
#![feature(alloc_error_handler)]
#![feature(core_intrinsics)]
#![test_runner(crate::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate alloc;
use core::{ops::DerefMut, panic::PanicInfo, sync::atomic::Ordering};

pub mod acpi;
pub mod allocator;
pub mod ap_init;
pub mod device;
pub mod gdt;
pub mod interrupts;
pub mod memory;
pub mod pio;
pub mod serial;

/// Virtual address of the beginning of the physical memory map setup by the bootloader.
pub const PHYS_OFFSET: u64 = 0x0000_4000_0000_0000; // must match bootloader conf in Cargo.toml

pub fn kstart(phys_mem_offset: u64, memory_regions: &'static MemoryRegions) {
    serial_print!("Initting...");

    gdt::init();

    interrupts::init_idt();

    // unsafe { interrupts::PICS.lock().initialize() };

    assert_eq!(phys_mem_offset, PHYS_OFFSET);

    let mut active_table = unsafe { memory::init(VirtAddr::new(phys_mem_offset)) };

    unsafe {
        memory::init_frame_alloc(memory_regions);
    }

    allocator::init_heap(&mut active_table, FRAME_ALLOC.lock().deref_mut())
        .expect("heap initialization failed");

    // Reset AP variables
    ap_init::CPU_COUNT.store(1, Ordering::SeqCst);
    ap_init::AP_READY.store(false, Ordering::SeqCst);
    ap_init::BSP_READY.store(false, Ordering::SeqCst);

    // Initialize devices (pic/apic)
    unsafe { device::init(&mut active_table) };

    ap_init::init_aps(&mut active_table);

    kmain()
}

pub unsafe extern "C" fn kstart_ap(args_ptr: *const ap_init::KernelArgsAp) -> ! {
    serial_println!("stuff from an ap");

    let args = &*args_ptr;
    let cpu_id = args.cpu_id as usize;
    let bsp_table = args.page_table as usize;
    let _stack_start = args.stack_start as usize;
    let stack_end = args.stack_end as usize;

    gdt::init();

    interrupts::init_idt();

    while !ap_init::BSP_READY.load(Ordering::SeqCst) {
        core::arch::x86_64::_mm_pause()
    }

    device::init_ap();

    ap_init::AP_READY.store(true, Ordering::SeqCst);
    while !ap_init::BSP_READY.load(Ordering::SeqCst) {
        core::arch::x86_64::_mm_pause()
    }

    crate::kmain_ap(cpu_id);
}

pub fn kmain() {
    serial_println!("stuff from main bsp");
    x86_64::instructions::interrupts::enable();
    crate::hlt_loop();
}

pub fn kmain_ap(cpu_id: usize) -> ! {
    serial_println!("stuff from ap {}", cpu_id);
    x86_64::instructions::interrupts::enable();
    crate::hlt_loop();
}
pub trait Testable {
    fn run(&self) -> ();
}

impl<T> Testable for T
where
    T: Fn(),
{
    fn run(&self) {
        serial_print!("{}...\t", core::any::type_name::<T>());
        self();
        serial_println!("[ok]");
    }
}

pub fn test_runner(tests: &[&dyn Testable]) {
    serial_println!("Running {} tests", tests.len());
    for test in tests {
        test.run();
    }
    exit_qemu(QemuExitCode::Success);
}

pub fn test_panic_handler(info: &PanicInfo) -> ! {
    serial_println!("[failed]\n");
    serial_println!("Error: {}\n", info);
    exit_qemu(QemuExitCode::Failed);
    hlt_loop();
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum QemuExitCode {
    Success = 0x10,
    Failed = 0x11,
}

pub fn exit_qemu(exit_code: QemuExitCode) {
    use x86_64::instructions::port::Port;

    unsafe {
        let mut port = Port::new(0xf4);
        port.write(exit_code as u32);
    }
}

pub fn hlt_loop() -> ! {
    loop {
        x86_64::instructions::hlt();
    }
}

use bootloader::boot_info::MemoryRegions;
#[cfg(test)]
use bootloader::{entry_point, BootInfo};
use x86_64::VirtAddr;

use crate::memory::FRAME_ALLOC;

#[cfg(test)]
entry_point!(test_kernel_main);

/// Entry point for `cargo xtest`
#[cfg(test)]
fn test_kernel_main(_boot_info: &'static BootInfo) -> ! {
    init();
    test_main();
    hlt_loop();
}

#[cfg(test)]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    test_panic_handler(info)
}

#[alloc_error_handler]
fn alloc_error_handler(layout: alloc::alloc::Layout) -> ! {
    panic!("allocation error: {:?}", layout)
}
