# SPDX-License-Identifier: GPL-2.0-only

CROSS_COMPILE ?= x86_64-unknown-redox-
CC := $(CROSS_COMPILE)gcc
LD := $(CROSS_COMPILE)ld
NM := $(CROSS_COMPILE)nm
O ?= out

LINUX := linux
GENERATED := generated
include linux-objects.mk

LINUX_ABI_FLAGS := -mcmodel=kernel -mpreferred-stack-boundary=3 -mskip-rax-setup
LINUX_CFLAGS := $(filter-out $(LINUX_ABI_FLAGS),$(LINUX_KBUILD_CFLAGS))
LINUX_INCLUDES := -Ishim/include \
	-I$(LINUX)/arch/x86/include -I$(GENERATED)/arch/x86/include/generated \
	-I$(LINUX)/include -I$(GENERATED)/include \
	-I$(LINUX)/arch/x86/include/uapi -I$(GENERATED)/arch/x86/include/generated/uapi \
	-I$(LINUX)/include/uapi -I$(GENERATED)/include/generated/uapi \
	-include $(LINUX)/include/linux/compiler-version.h \
	-include $(LINUX)/include/linux/kconfig.h \
	-include $(LINUX)/include/linux/compiler_types.h

SHIM_FLAGS := -Wall -Werror=implicit-function-declaration '-DKBUILD_MODNAME="ferrion_shim"' -D__KBUILD_MODNAME=kmod_ferrion_shim
HOST_CFLAGS := -O2 -Wall -Werror -std=gnu11 -Ishim/include

LINUX_OBJS := $(LINUX_SOURCES:%=$(O)/linux/%.o)
SHIM_SOURCES := $(sort $(wildcard shim/*.c))
SHIM_OBJS := $(SHIM_SOURCES:%.c=$(O)/%.o)
HOST_SOURCES := $(sort $(wildcard host/*.c))
HOST_OBJS := $(HOST_SOURCES:%.c=$(O)/%.o)
REFUSAL_OBJ := $(O)/shim/refusals.o
TARGET ?= x86_64-unknown-redox
CARGO_BUILD ?= cargo build --target $(TARGET)
FRONTEND_LIB := $(O)/cargo/$(TARGET)/release/libferrion_frontend.a
LIBS := $(shell $(CC) -print-file-name=libc.a) $(shell $(CC) -print-libgcc-file-name) $(FRONTEND_LIB)
HOST := $(O)/ferrion-driver-host

.PHONY: all census check-link clean links
all: check-link $(HOST)

$(O)/linux/%.o: $(LINUX)/%.c
	@mkdir -p $(@D)
	@echo "  CC      $<"
	@$(CC) $(LINUX_CFLAGS) -w $(LINUX_INCLUDES) $($*_FLAGS) -c $< -o $@

$(O)/shim/%.o: shim/%.c $(wildcard shim/include/*/*.h)
	@mkdir -p $(@D)
	@echo "  CC      $<"
	@$(CC) $(LINUX_CFLAGS) $(LINUX_INCLUDES) $(SHIM_FLAGS) -c $< -o $@

$(O)/shim/refusals.c: shim/refusals.tsv
	@mkdir -p $(@D)
	awk -F'\t' 'BEGIN { print "__attribute__((noreturn)) void ferrion_refuse(const char *name);" } \
		NR > 1 { printf "void %s(void);\nvoid %s(void) { ferrion_refuse(\"%s\"); }\n", $$1, $$1, $$1 }' $< > $@

$(O)/shim/refusals.o: $(O)/shim/refusals.c
	$(CC) $(HOST_CFLAGS) -c $< -o $@

$(O)/host/%.o: host/%.c shim/include/ferrion/host.h
	@mkdir -p $(@D)
	$(CC) $(HOST_CFLAGS) -c $< -o $@

$(O)/census/linux.o: $(LINUX_OBJS)
	@mkdir -p $(@D)
	$(LD) -r -o $@ $^

$(O)/census/host.o: $(LINUX_OBJS) $(SHIM_OBJS) $(REFUSAL_OBJ) $(HOST_OBJS)
	@mkdir -p $(@D)
	$(LD) -r -o $@ $^

links:
	$(if $(BASE_SRC),,$(error BASE_SRC names base's fetched source, RELIBC_SRC relibc's))
	ln -sfn $(abspath $(BASE_SRC)) base
	ln -sfn $(abspath $(RELIBC_SRC)) relibc

$(FRONTEND_LIB): frontend/Cargo.toml frontend/Cargo.lock $(wildcard frontend/src/*.rs) | links
	CARGO_TARGET_DIR=$(abspath $(O)/cargo) $(CARGO_BUILD) --release --locked --manifest-path frontend/Cargo.toml

CENSUS = sh tools/census.sh $(O)/census "$(NM)" shim/refusals.tsv initcalls.ld "$(SHIM_OBJS)" "$(LIBS)"

census: $(O)/census/linux.o $(O)/census/host.o $(FRONTEND_LIB)
	$(CENSUS)

check-link: $(O)/census/linux.o $(O)/census/host.o $(FRONTEND_LIB)
	$(CENSUS) check

$(HOST): check-link initcalls.ld
	$(CC) -static -o $@ -Wl,--gc-sections -Wl,-T,initcalls.ld $(LINUX_OBJS) $(SHIM_OBJS) $(REFUSAL_OBJ) $(HOST_OBJS) $(FRONTEND_LIB)

clean:
	rm -rf $(O)
