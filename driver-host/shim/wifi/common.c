// SPDX-License-Identifier: GPL-2.0-only
#include <linux/gfp.h>
#include <linux/init.h>
#include <linux/timekeeping.h>
#include <linux/jiffies.h>
#include <linux/mm.h>
#include <linux/percpu.h>
#include <linux/random.h>
#include <linux/slab.h>
#include <linux/string.h>
#include <ferrion/linux.h>
#include <ferrion/pool.h>

#define FERRION_RANDOM_BYTES_MAX (1U << 20)
#define FERRION_STRING_MAX (1U << 20)
struct workqueue_struct *system_freezable_wq;
struct workqueue_struct *system_power_efficient_wq;

static int wifi_workqueues_init(void)
{
	system_freezable_wq = system_wq;
	system_power_efficient_wq = system_wq;
	return 0;
}
early_initcall(wifi_workqueues_init);


void get_random_bytes(void *buf, size_t len)
{
	u8 *out = buf;
	size_t offset;

	if (len > FERRION_RANDOM_BYTES_MAX)
		ferrion_abort("get_random_bytes: byte bound");
	for (offset = 0; offset < len; offset += sizeof(u32)) {
		u32 word = ferrion_random_u32();
		size_t count = min_t(size_t, sizeof(word), len - offset);

		memcpy(out + offset, &word, count);
	}
}

u8 get_random_u8(void)
{
	return (u8)ferrion_random_u32();
}

u16 get_random_u16(void)
{
	return (u16)ferrion_random_u32();
}

void kfree_sensitive(const void *ptr)
{
	if (!ZERO_OR_NULL_PTR(ptr))
		memzero_explicit((void *)ptr, ksize(ptr));
	kfree(ptr);
}

void *kmemdup_array(const void *src, size_t count, size_t size, gfp_t flags)
{
	size_t bytes;

	if (check_mul_overflow(count, size, &bytes))
		return NULL;
	return kmemdup(src, bytes, flags);
}

char *kstrndup(const char *src, size_t max, gfp_t flags)
{
	size_t len;
	char *copy;

	if (!src || max > FERRION_STRING_MAX)
		return NULL;
	len = strnlen(src, max);
	copy = kmalloc(len + 1, flags);
	if (copy) {
		memcpy(copy, src, len);
		copy[len] = 0;
	}
	return copy;
}

void *__kmalloc_large_noprof(size_t size, gfp_t flags)
{
	return ferrion_kmalloc(size, PAGE_SIZE, flags);
}

void __percpu *pcpu_alloc_noprof(size_t size, size_t align, bool reserved, gfp_t flags)
{
	return ferrion_kmalloc(size, max_t(size_t, align, sizeof(void *)), flags | __GFP_ZERO);
}

void free_percpu(void __percpu *ptr)
{
	kfree(ptr);
}

struct page *__alloc_pages_noprof(gfp_t flags, unsigned int order, int node, nodemask_t *mask)
{
	unsigned long addr;

	if (order > MAX_PAGE_ORDER)
		return NULL;
	addr = get_free_pages_noprof(flags, order);
	return addr ? virt_to_page((void *)addr) : NULL;
}

void __free_pages(struct page *page, unsigned int order)
{
	if (page && put_page_testzero(page))
		free_pages((unsigned long)page_address(page), order);
}

unsigned long __usecs_to_jiffies(const unsigned int value)
{
	return _usecs_to_jiffies(value);
}

unsigned int jiffies_to_msecs(const unsigned long value)
{
	return (u64)value * MSEC_PER_SEC / HZ;
}

signed long schedule_timeout_interruptible(signed long timeout)
{
	__set_current_state(TASK_INTERRUPTIBLE);
	return schedule_timeout(timeout);
}

void rcu_barrier(void)
{
	ferrion_rcu_drain();
}

time64_t ktime_get_seconds(void)
{
	return div_u64(ferrion_now_ns(), NSEC_PER_SEC);
}

u32 get_random_u32(void)
{
	return ferrion_random_u32();
}

int wait_for_random_bytes(void)
{
	ferrion_random_u32();
	return 0;
}
