pub mod core_local;
pub mod interrupts;
#[cfg(all(not(feature = "pci"), any(feature = "tcp", feature = "udp")))]
pub mod mmio;
#[cfg(feature = "pci")]
pub mod pci;
pub mod processor;
pub mod scheduler;
pub mod serial;
#[cfg(target_os = "none")]
mod start;
pub mod switch;
pub mod systemtime;

use alloc::alloc::{Layout, alloc};
use core::arch::global_asm;
use core::sync::atomic::{AtomicPtr, AtomicU32, Ordering};
use core::task::Waker;
use core::{ptr, str};

use memory_addresses::arch::aarch64::{PhysAddr, VirtAddr};

use crate::arch::aarch64::kernel::core_local::*;
use crate::arch::aarch64::kernel::serial::SerialPort;
use crate::arch::aarch64::mm::paging::{BasePageSize, PageSize};
use crate::config::*;
use crate::env;

const SERIAL_PORT_BAUDRATE: u32 = 115_200;

pub(crate) struct Console {
	serial_port: SerialPort,
}

impl Console {
	pub fn new() -> Self {
		CoreLocal::install();

		let base = env::boot_info()
			.hardware_info
			.serial_port_base
			.map(|uartport| uartport.get())
			.unwrap_or_default()
			.try_into()
			.unwrap();

		let serial_port = SerialPort::new(base);

		serial_port.init(SERIAL_PORT_BAUDRATE);

		Self { serial_port }
	}

	pub fn write(&mut self, buf: &[u8]) {
		self.serial_port.write_buf(buf);
	}

	pub fn read(&mut self) -> Option<u8> {
		None
	}

	pub fn is_empty(&self) -> bool {
		true
	}

	pub fn register_waker(&mut self, _waker: &Waker) {}
}

impl Default for Console {
	fn default() -> Self {
		Self::new()
	}
}

#[repr(align(8))]
pub(crate) struct AlignedAtomicU32(AtomicU32);

/// `CPU_ONLINE` is the count of CPUs that finished initialization.
///
/// It also synchronizes initialization of CPU cores.
pub(crate) static CPU_ONLINE: AlignedAtomicU32 = AlignedAtomicU32(AtomicU32::new(0));

pub(crate) static CURRENT_STACK_ADDRESS: AtomicPtr<u8> = AtomicPtr::new(ptr::null_mut());

#[cfg(target_os = "none")]
global_asm!(include_str!("start.s"));

pub fn is_uhyve_with_pci() -> bool {
	false
}

pub fn get_ram_address() -> PhysAddr {
	PhysAddr::new(env::boot_info().hardware_info.phys_addr_range.start)
}

pub fn get_base_address() -> VirtAddr {
	VirtAddr::new(env::boot_info().load_info.kernel_image_addr_range.start)
}

pub fn get_image_size() -> usize {
	let range = &env::boot_info().load_info.kernel_image_addr_range;
	(range.end - range.start) as usize
}

pub fn get_limit() -> usize {
	env::boot_info().hardware_info.phys_addr_range.end as usize
}

#[cfg(feature = "smp")]
pub fn get_possible_cpus() -> u32 {
	use hermit_dtb::Dtb;

	let dtb = unsafe {
		Dtb::from_raw(core::ptr::with_exposed_provenance(
			env::boot_info().hardware_info.device_tree.unwrap().get() as usize,
		))
		.expect(".dtb file has invalid header")
	};

	dtb.enum_subnodes("/cpus")
		.filter(|name| name.contains("cpu@"))
		.count()
		.try_into()
		.unwrap()
}

#[cfg(feature = "smp")]
pub fn get_processor_count() -> u32 {
	CPU_ONLINE.0.load(Ordering::Acquire)
}

#[cfg(not(feature = "smp"))]
pub fn get_processor_count() -> u32 {
	1
}

pub fn args() -> Option<&'static str> {
	None
}

/// Real Boot Processor initialization as soon as we have put the first Welcome message on the screen.
#[cfg(target_os = "none")]
pub fn boot_processor_init() {
	if !crate::env::is_uhyve() {
		processor::configure();
	}

	crate::mm::init();
	crate::mm::print_information();
	CoreLocal::get().add_irq_counter();
	env::init();
	interrupts::init();
	processor::detect_frequency();
	processor::print_information();
	systemtime::init();
	#[cfg(feature = "pci")]
	pci::init();

	finish_processor_init();
}

/// Application Processor initialization
#[allow(dead_code)]
pub fn application_processor_init() {
	CoreLocal::install();
	finish_processor_init();
}

fn finish_processor_init() {
	debug!("Initialized Processor");

	// Allocate stack for the CPU and pass the addresses.
	let layout = Layout::from_size_align(KERNEL_STACK_SIZE, BasePageSize::SIZE as usize).unwrap();
	let stack = unsafe { alloc(layout) };
	assert!(!stack.is_null());
	CURRENT_STACK_ADDRESS.store(stack, Ordering::Relaxed);
}

pub fn boot_next_processor() {
	CPU_ONLINE.0.fetch_add(1, Ordering::Release);
}

pub fn print_statistics() {
	interrupts::print_statistics();
}
