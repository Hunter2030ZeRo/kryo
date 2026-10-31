use super::tss::boot_tss;
use spin::Once;
use x86_64::instructions::tables::load_tss;
use x86_64::registers::segmentation::{CS, SS, Segment};
use x86_64::structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector};

struct Selectors {
    code: SegmentSelector,
    data: SegmentSelector,
    tss: SegmentSelector,
}

static GDT: Once<(GlobalDescriptorTable, Selectors)> = Once::new();
static BOOT_CPU_LOADED: Once<()> = Once::new();

pub fn init_gdt() {
    let (gdt, selectors) = GDT.call_once(|| {
        let mut gdt = GlobalDescriptorTable::new();

        let code = gdt.append(Descriptor::kernel_code_segment());
        let data = gdt.append(Descriptor::kernel_data_segment());
        let tss = gdt.append(Descriptor::tss_segment(boot_tss()));

        (gdt, Selectors { code, data, tss })
    });

    BOOT_CPU_LOADED.call_once(|| {
        gdt.load();

        unsafe {
            CS::set_reg(selectors.code);
            SS::set_reg(selectors.data);
            load_tss(selectors.tss);
        }
    });
}
