TARGET := x86_64-unknown-none
PROFILE := debug
CARGO_PROFILE := $(if $(filter debug,$(PROFILE)),dev,$(PROFILE))
FEATURES ?=
BLOCK_PROBE ?= 0
ABI_PROBE ?= 0
LIBC_PROBE ?= 0
FEATURE_FLAGS := $(if $(strip $(FEATURES)),--features "$(FEATURES)")

BUILD_DIR := build
ISO_ROOT := $(BUILD_DIR)/iso_root
ISO := $(BUILD_DIR)/kryo.iso
USER_ELF := $(BUILD_DIR)/native-smoke.elf
INIT_ELF := $(BUILD_DIR)/init.elf
WORKER_ELF := $(BUILD_DIR)/bootstrap-worker.elf
DEVICE_ELF := $(BUILD_DIR)/device-probe.elf
SCALE_ELF := $(BUILD_DIR)/scale-probe.elf
SERVICE_ELF := $(BUILD_DIR)/service-probe.elf
PEER_ELF := $(BUILD_DIR)/service-peer.elf
SUPERVISOR_ELF := $(BUILD_DIR)/supervisor.elf
ECHO_ELF := $(BUILD_DIR)/echo-service.elf
CLIENT_ELF := $(BUILD_DIR)/registry-client.elf
BLOCK_ELF := $(BUILD_DIR)/virtio-block.elf
BLOCK_SERVICE_ELF := $(BUILD_DIR)/block-service.elf
BLOCK_CLIENT_ELF := $(BUILD_DIR)/block-client.elf
ABI_ELF := $(BUILD_DIR)/abi-probe.elf
LIBC_ELF := $(BUILD_DIR)/libc-probe.elf

CARGO_TARGET_DIR ?= target
KERNEL := $(CARGO_TARGET_DIR)/$(TARGET)/$(PROFILE)/kryo
LIMINE := /usr/share/limine

CPUS ?= 4
CPU ?= max
MEMORY ?= 512M
QEMU := qemu-system-x86_64
QEMU_FLAGS := -M q35,memory-backend=ram -object memory-backend-ram,id=ram,size=$(MEMORY),prealloc=off,reserve=off -overcommit mem-lock=off -global hpet.msi=on -m $(MEMORY) -smp $(CPUS) -cpu $(CPU) -serial stdio -no-reboot -debugcon "file:$(BUILD_DIR)/debug.log"

.PHONY: all kernel user abi libc iso run check test smoke clean

all: iso

kernel:
	cargo build --locked -p kryo --target $(TARGET) --profile $(CARGO_PROFILE) $(FEATURE_FLAGS)

abi:
	python3 tools/build_abi.py --profile "$(PROFILE)" --output "$(ABI_ELF)"

libc:
	python3 tools/build_libc.py --profile "$(PROFILE)" --build-dir "$(BUILD_DIR)/libc" --output "$(LIBC_ELF)"

user: $(if $(filter 1,$(ABI_PROBE)),abi) $(if $(filter 1,$(LIBC_PROBE)),libc)
	python3 tools/build_user.py --profile "$(PROFILE)" --output "$(USER_ELF)"
	python3 tools/build_user.py --program init --profile "$(PROFILE)" --output "$(INIT_ELF)"
	python3 tools/build_user.py --program bootstrap-worker --profile "$(PROFILE)" --output "$(WORKER_ELF)"
	python3 tools/build_user.py --program device-probe --profile "$(PROFILE)" --output "$(DEVICE_ELF)"
	python3 tools/build_user.py --program scale-probe --profile "$(PROFILE)" --output "$(SCALE_ELF)"
	python3 tools/build_user.py --program service-probe --profile "$(PROFILE)" --output "$(SERVICE_ELF)"
	python3 tools/build_user.py --program service-peer --profile "$(PROFILE)" --output "$(PEER_ELF)"
	python3 tools/build_user.py --program supervisor --profile "$(PROFILE)" --output "$(SUPERVISOR_ELF)"
	python3 tools/build_user.py --program echo-service --profile "$(PROFILE)" --output "$(ECHO_ELF)"
	python3 tools/build_user.py --program registry-client --profile "$(PROFILE)" --output "$(CLIENT_ELF)"
	python3 tools/build_user.py --program virtio-block --profile "$(PROFILE)" --output "$(BLOCK_ELF)"
	python3 tools/build_user.py --program block-service --profile "$(PROFILE)" --output "$(BLOCK_SERVICE_ELF)"
	python3 tools/build_user.py --program block-client --profile "$(PROFILE)" --output "$(BLOCK_CLIENT_ELF)"

iso: kernel user
	rm -rf "$(ISO_ROOT)"
	mkdir -p "$(ISO_ROOT)/boot/limine"
	mkdir -p "$(ISO_ROOT)/EFI/BOOT"

	cp "$(KERNEL)" "$(ISO_ROOT)/boot/kryo"
	cp "$(USER_ELF)" "$(ISO_ROOT)/boot/native-smoke.elf"
	cp "$(INIT_ELF)" "$(ISO_ROOT)/boot/init.elf"
	cp "$(WORKER_ELF)" "$(ISO_ROOT)/boot/bootstrap-worker.elf"
	cp "$(DEVICE_ELF)" "$(ISO_ROOT)/boot/device-probe.elf"
	cp "$(SCALE_ELF)" "$(ISO_ROOT)/boot/scale-probe.elf"
	cp "$(SERVICE_ELF)" "$(ISO_ROOT)/boot/service-probe.elf"
	cp "$(PEER_ELF)" "$(ISO_ROOT)/boot/service-peer.elf"
	cp "$(SUPERVISOR_ELF)" "$(ISO_ROOT)/boot/supervisor.elf"
	cp "$(ECHO_ELF)" "$(ISO_ROOT)/boot/echo-service.elf"
	cp "$(CLIENT_ELF)" "$(ISO_ROOT)/boot/registry-client.elf"
	cp "$(BLOCK_ELF)" "$(ISO_ROOT)/boot/virtio-block.elf"
	cp "$(BLOCK_SERVICE_ELF)" "$(ISO_ROOT)/boot/block-service.elf"
	cp "$(BLOCK_CLIENT_ELF)" "$(ISO_ROOT)/boot/block-client.elf"
	cp boot/limine.conf "$(ISO_ROOT)/boot/limine/"
ifeq ($(ABI_PROBE),1)
	cp "$(ABI_ELF)" "$(ISO_ROOT)/boot/abi-probe.elf"
	python3 -c 'from pathlib import Path; c=Path("$(ISO_ROOT)/boot/limine/limine.conf"); c.write_text(c.read_text()+"\tmodule_path: boot():/boot/abi-probe.elf\n\tmodule_cmdline: kryo-abi-probe\n")'
endif
ifeq ($(LIBC_PROBE),1)
	cp "$(LIBC_ELF)" "$(ISO_ROOT)/boot/libc-probe.elf"
	python3 -c 'from pathlib import Path; c=Path("$(ISO_ROOT)/boot/limine/limine.conf"); c.write_text(c.read_text()+"\tmodule_path: boot():/boot/libc-probe.elf\n\tmodule_cmdline: kryo-libc-probe\n")'
endif
ifeq ($(BLOCK_PROBE),1)
	python3 -c 'from pathlib import Path; p=Path("$(ISO_ROOT)/boot"); (p/"block-probe.enabled").write_bytes(b"KRYO-BLOCK-PROBE-V1\n"); c=p/"limine/limine.conf"; c.write_text(c.read_text()+"\tmodule_path: boot():/boot/block-probe.enabled\n\tmodule_cmdline: kryo-block-probe-enabled\n")'
endif

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
