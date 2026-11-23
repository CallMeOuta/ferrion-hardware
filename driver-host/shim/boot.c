// SPDX-License-Identifier: GPL-2.0-only
#include <linux/cpuhotplug.h>
#include <linux/init.h>
#include <linux/jump_label.h>
#include <linux/printk.h>
#include <linux/radix-tree.h>
#include <asm/cpufeature.h>
#include <asm/cpuid/api.h>
#include <asm/page.h>
#include <asm/pgtable_types.h>
#include <asm/processor.h>
#include <ferrion/linux.h>
#include <ferrion/pool.h>

struct cpuinfo_x86 boot_cpu_data;
unsigned long page_offset_base = FERRION_PAGE_OFFSET;
unsigned long vmemmap_base = FERRION_VMEMMAP_BASE;
unsigned long phys_base;
pteval_t __default_kernel_pte_mask = ~0;
bool static_key_initialized = true;
struct cpumask __cpu_possible_mask = { CPU_BITS_CPU0 };

extern initcall_entry_t __ferrion_initcall_start[];
extern initcall_entry_t __ferrion_initcall_end[];

static void boot_cpu_init(void)
{
	struct cpuinfo_x86 *c = &boot_cpu_data;
	u32 eax, ebx, ecx, edx;

	cpuid(0, &eax, &ebx, &ecx, &edx);
	c->cpuid_level = eax;
	cpuid(1, &eax, &ebx, &ecx, &edx);
	c->x86_capability[CPUID_1_ECX] = ecx;
	c->x86_capability[CPUID_1_EDX] = edx;
	c->x86_clflush_size = ((ebx >> 8) & 0xff) * 8;
	c->x86_cache_alignment = c->x86_clflush_size;
	if (c->cpuid_level >= 7) {
		cpuid_count(7, 0, &eax, &ebx, &ecx, &edx);
		c->x86_capability[CPUID_7_0_EBX] = ebx;
		c->x86_capability[CPUID_7_ECX] = ecx;
		c->x86_capability[CPUID_7_EDX] = edx;
	}
	cpuid(0x80000000, &eax, &ebx, &ecx, &edx);
	c->extended_cpuid_level = eax;
	if (c->extended_cpuid_level >= 0x80000001) {
		cpuid(0x80000001, &eax, &ebx, &ecx, &edx);
		c->x86_capability[CPUID_8000_0001_ECX] = ecx;
		c->x86_capability[CPUID_8000_0001_EDX] = edx;
	}
	if (c->extended_cpuid_level >= 0x80000008) {
		cpuid(0x80000008, &eax, &ebx, &ecx, &edx);
		c->x86_phys_bits = eax & 0xff;
		c->x86_virt_bits = (eax >> 8) & 0xff;
	}
}

int __cpuhp_setup_state(enum cpuhp_state state, const char *name, bool invoke, int (*startup)(unsigned int cpu),
			int (*teardown)(unsigned int cpu), bool multi_instance)
{
	if (invoke && startup) {
		int ret = startup(0);

		if (ret)
			return ret;
	}
	return state == CPUHP_AP_ONLINE_DYN || state == CPUHP_BP_PREPARE_DYN ? state : 0;
}

static void run_initcalls(void)
{
	initcall_entry_t *entry;

	for (entry = __ferrion_initcall_start; entry < __ferrion_initcall_end; entry++) {
		initcall_t fn = initcall_from_entry(entry);
		int ret = fn();

		if (ret)
			pr_err("ferrion: initcall %ps returned %d\n", fn, ret);
	}
}

int ferrion_linux_start(void)
{
	ferrion_enter();
	boot_cpu_init();
	radix_tree_init();
	ferrion_time_init();
	ferrion_workqueue_init();
	run_initcalls();
	pr_info("ferrion: initcalls done\n");
	ferrion_leave();
	return 0;
}
