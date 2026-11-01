use super::cpu::{MAX_CPUS, current_cpu, init_boot_cpu};
use super::tss::local_tss;
use spin::Once;
use x86_64::instructions::tables::load_tss;
use x86_64::registers::segmentation::{CS, DS, ES, FS, GS, SS, Segment};
use x86_64::structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector};

struct Selectors {
    code: SegmentSelector,
    data: SegmentSelector,
    tss: SegmentSelector,
    user_code: SegmentSelector,
    user_data: SegmentSelector,
}

static GDTS: [Once<(GlobalDescriptorTable, Selectors)>; MAX_CPUS] =
    [const { Once::new() }; MAX_CPUS];
static LOADED: [Once<()>; MAX_CPUS] = [const { Once::new() }; MAX_CPUS];

pub fn init_gdt() {
    assert!(
        !x86_64::instructions::interrupts::are_enabled(),
        "GDT initialization requires IRQs off"
    );
    let cpu =
        current_cpu().unwrap_or_else(|| unsafe { init_boot_cpu() }.expect("Unregistered CPU"));
    let (gdt, selectors) = GDTS[cpu.index()].call_once(|| {
        let mut gdt = GlobalDescriptorTable::new();

        let code = gdt.append(Descriptor::kernel_code_segment());
        let data = gdt.append(Descriptor::kernel_data_segment());
        let user_data = gdt.append(Descriptor::user_data_segment());
        let user_code = gdt.append(Descriptor::user_code_segment());
        // This CPU's TSS stays fixed; RSP0 uses CPU-local interior mutability.
        let tss = gdt.append(unsafe { Descriptor::tss_segment_unchecked(local_tss(cpu)) });

        (
            gdt,
            Selectors {
                code,
                data,
                tss,
                user_code,
                user_data,
            },
        )
    });

    LOADED[cpu.index()].call_once(|| {
        gdt.load();

        unsafe {
            CS::set_reg(selectors.code);
            SS::set_reg(selectors.data);
            DS::set_reg(selectors.data);
            ES::set_reg(selectors.data);
            FS::set_reg(SegmentSelector(0));
            GS::set_reg(SegmentSelector(0));
            load_tss(selectors.tss);
        }
    });
}

pub(super) fn user_selectors() -> Option<(u16, u16)> {
    let (_, selectors) = GDTS[current_cpu()?.index()].get()?;
    Some((selectors.user_code.0, selectors.user_data.0))
}
