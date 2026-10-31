TARGET := x86_64-unknown-none
PROFILE := debug
CARGO_PROFILE := $(if $(filter debug,$(PROFILE)),dev,$(PROFILE))
FEATURES ?=
FEATURE_FLAGS := $(if $(strip $(FEATURES)),--features "$(FEATURES)")

BUILD_DIR := build
ISO_ROOT := $(BUILD_DIR)/iso_root
ISO := $(BUILD_DIR)/kryo.iso

CARGO_TARGET_DIR ?= target
KERNEL := $(CARGO_TARGET_DIR)/$(TARGET)/$(PROFILE)/kryo
LIMINE := /usr/share/limine

QEMU := qemu-system-x86_64
QEMU_FLAGS := -M q35 -m 512M -serial stdio -no-reboot -debugcon "file:$(BUILD_DIR)/debug.log"

.PHONY: all kernel iso run check test smoke clean

all: iso

kernel:
	cargo build --locked -p kryo --target $(TARGET) --profile $(CARGO_PROFILE) $(FEATURE_FLAGS)

iso: kernel
	rm -rf "$(ISO_ROOT)"
	mkdir -p "$(ISO_ROOT)/boot/limine"
	mkdir -p "$(ISO_ROOT)/EFI/BOOT"

	cp "$(KERNEL)" "$(ISO_ROOT)/boot/kryo"
	cp boot/limine.conf "$(ISO_ROOT)/boot/limine/"

	cp "$(LIMINE)/limine-bios-cd.bin" "$(ISO_ROOT)/boot/limine/"
	cp "$(LIMINE)/limine-uefi-cd.bin" "$(ISO_ROOT)/boot/limine/"
	cp "$(LIMINE)/limine-bios.sys" "$(ISO_ROOT)/boot/limine/"
	cp "$(LIMINE)/BOOTX64.EFI" "$(ISO_ROOT)/EFI/BOOT/"

	mkdir -p "$(BUILD_DIR)"

	xorriso -as mkisofs -R -r -J \
		-b boot/limine/limine-bios-cd.bin \
		-no-emul-boot \
		-boot-load-size 4 \
		-boot-info-table \
		--efi-boot boot/limine/limine-uefi-cd.bin \
		--efi-boot-part \
		--efi-boot-image \
		--protective-msdos-label \
		"$(ISO_ROOT)" -o "$(ISO)"

	limine bios-install "$(ISO)"


run: iso
	"$(QEMU)" $(QEMU_FLAGS) -cdrom "$(ISO)" -boot d

check:
	cargo check --workspace --locked --offline

test:
	cargo test -p kryo-core -p kryo-hal -p kryo-interfaces --target x86_64-unknown-linux-gnu --locked --offline

smoke:
	python3 tools/boot_smoke.py --profile "$(PROFILE)"

clean:
	cargo clean
	rm -rf "$(BUILD_DIR)"
