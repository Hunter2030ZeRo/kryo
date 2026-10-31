use super::tss::boot_tss;
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

static GDT: Once<(GlobalDescriptorTable, Selectors)> = Once::new();
static BOOT_CPU_LOADED: Once<()> = Once::new();

pub fn init_gdt() {
    let (gdt, selectors) = GDT.call_once(|| {
        let mut gdt = GlobalDescriptorTable::new();

        let code = gdt.append(Descriptor::kernel_code_segment());
        let data = gdt.append(Descriptor::kernel_data_segment());
        let user_data = gdt.append(Descriptor::user_data_segment());
        let user_code = gdt.append(Descriptor::user_code_segment());
        // The BSP TSS stays at a stable address and uses interior mutability for RSP0.
        let tss = gdt.append(unsafe { Descriptor::tss_segment_unchecked(boot_tss()) });

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

    BOOT_CPU_LOADED.call_once(|| {
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
    let (_, selectors) = GDT.get()?;
    Some((selectors.user_code.0, selectors.user_data.0))
}
