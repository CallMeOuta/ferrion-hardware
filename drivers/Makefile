# SPDX-License-Identifier: GPL-2.0-only
TARGET ?= x86_64-unknown-redox
BUILD_FLAGS ?= --release
CARGO ?= cargo
CARGO_FLAGS ?=

SRC_DIR ?= $(CURDIR)
BUILD_DIR ?= $(SRC_DIR)/target/$(TARGET)/build
DESTDIR ?= $(SRC_DIR)/sysroot
TARGET_DIR = $(BUILD_DIR)/$(TARGET)/release
INITFS_DIR = $(DESTDIR)/usr/lib/initfs/root

BINS = fbcond pcid pcid-spawner redoxerd nbdd
DRIVERS = e1000d ihdad ihdgd rtl8139d rtl8168d \
	usbctl usbhidd usbhubd usbscsid virtio-gpud virtio-netd xhcid
INITFS_BINS = acpid fbbootlogd hwd lived pcid pcid-spawner rtcd vesad
INITFS_DRIVERS = nvmed virtio-blkd

ifneq (,$(filter i586-unknown-redox i686-unknown-redox x86_64-unknown-redox,$(TARGET)))
    INITFS_BINS += ps2d
    INITFS_DRIVERS += ahcid ided
    DRIVERS += ac97d sb16d vboxd
endif

ifeq (,$(filter i586-unknown-redox i686-unknown-redox,$(TARGET)))
    DRIVERS += virtio-fsd
endif

ifeq ($(TARGET),aarch64-unknown-redox)
    INITFS_BINS += bcm2835-sdhcid
endif

CARGO_BUILD = CARGO_PROFILE_RELEASE_OPT_LEVEL=s CARGO_PROFILE_RELEASE_PANIC=abort \
	$(CARGO) build $(BUILD_FLAGS) $(CARGO_FLAGS) \
	--manifest-path "$(SRC_DIR)/Cargo.toml" --target-dir "$(BUILD_DIR)"

.PHONY: all build install list clean

all: build

list:
	@echo "bins: $(BINS)"
	@echo "drivers: $(DRIVERS)"
	@echo "initfs bins: $(INITFS_BINS)"
	@echo "initfs drivers: $(INITFS_DRIVERS)"

clean:
	rm -rf "$(SRC_DIR)/target" "$(SRC_DIR)/sysroot"

build:
	@mkdir -pv "$(BUILD_DIR)"
	$(CARGO_BUILD) $(foreach bin,$(BINS) $(DRIVERS),-p $(bin))
	$(CARGO_BUILD) $(foreach bin,$(INITFS_BINS) $(INITFS_DRIVERS),-p $(bin))
	@mkdir -pv "$(BUILD_DIR)/initfs/bin" "$(BUILD_DIR)/initfs/lib/drivers"
	@for bin in $(INITFS_BINS); do \
		cp -v "$(TARGET_DIR)/$$bin" "$(BUILD_DIR)/initfs/bin"; \
	done
	@for bin in $(INITFS_DRIVERS); do \
		cp -v "$(TARGET_DIR)/$$bin" "$(BUILD_DIR)/initfs/lib/drivers"; \
	done

install: build
	@mkdir -pv "$(DESTDIR)/usr/bin" "$(DESTDIR)/usr/lib/drivers"
	@for bin in $(BINS); do \
		cp -v "$(TARGET_DIR)/$$bin" "$(DESTDIR)/usr/bin"; \
	done
	@for bin in $(DRIVERS); do \
		cp -v "$(TARGET_DIR)/$$bin" "$(DESTDIR)/usr/lib/drivers"; \
	done
	@mkdir -pv "$(DESTDIR)/usr/lib/pcid.d" "$(DESTDIR)/usr/lib/xhcid.d"
	@find "$(SRC_DIR)" -maxdepth 3 -type f -name 'config.toml' | while read -r conf; do \
		driver=$$(basename "$$(dirname "$$conf")"); \
		cp -v "$$conf" "$(DESTDIR)/usr/lib/pcid.d/$$driver.toml"; \
	done
	@cp -v "$(SRC_DIR)/usb/xhcid/drivers.toml" "$(DESTDIR)/usr/lib/xhcid.d/"
	@mkdir -pv "$(INITFS_DIR)/lib/pcid.d"
	cp -rv "$(BUILD_DIR)/initfs/bin" "$(BUILD_DIR)/initfs/lib" "$(INITFS_DIR)/"
	cp -v "$(SRC_DIR)/initfs.toml" "$(INITFS_DIR)/lib/pcid.d/initfs.toml"
