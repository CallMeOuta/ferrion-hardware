// SPDX-License-Identifier: GPL-2.0-only
#define _GNU_SOURCE
#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <sys/mman.h>
#include <unistd.h>
#include <ferrion/address.h>
#include <ferrion/host.h>

#define FERRION_SIM_PAGE_SIZE 4096UL
#define FERRION_SIM_BYTES_MAX (512UL << 20)
#define FERRION_SIM_PAGES_MAX (FERRION_SIM_BYTES_MAX / FERRION_SIM_PAGE_SIZE)
#define FERRION_SIM_CONTINUATION UINT32_MAX
#define FERRION_SIM_LOG_MAX 1024UL

static uint32_t allocated_pages[FERRION_SIM_PAGES_MAX];
static int linear_reserved;

void ferrion_log(int level, const char *text, unsigned long len)
{
	(void)level;
	if (len > FERRION_SIM_LOG_MAX)
		len = FERRION_SIM_LOG_MAX;
	fwrite(text, 1, len, stderr);
	fputc('\n', stderr);
	fflush(stderr);
}

int ferrion_reserve(void *addr, unsigned long size, int writable)
{
	int prot = writable ? PROT_READ | PROT_WRITE : PROT_NONE;
	void *mapped = mmap(addr, size, prot, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);

	if (mapped == MAP_FAILED)
		return -errno;
	if (mapped != addr) {
		munmap(mapped, size);
		return -EADDRNOTAVAIL;
	}
	if ((uintptr_t)addr == FERRION_PAGE_OFFSET && size == FERRION_PHYS_LIMIT)
		linear_reserved = 1;
	return 0;
}

void *ferrion_phys_alloc(unsigned long size, unsigned long *phys)
{
	unsigned long pages, start = 0, run = 0, i;
	void *mapped, *addr;

	if (!linear_reserved || !phys || !size || size > FERRION_SIM_BYTES_MAX ||
	    size % FERRION_SIM_PAGE_SIZE)
		return NULL;
	pages = size / FERRION_SIM_PAGE_SIZE;
	for (i = 0; i < FERRION_SIM_PAGES_MAX; i++) {
		if (allocated_pages[i]) {
			run = 0;
			continue;
		}
		if (!run)
			start = i;
		if (++run == pages)
			break;
	}
	if (run != pages)
		return NULL;
	addr = (void *)(FERRION_PAGE_OFFSET + start * FERRION_SIM_PAGE_SIZE);
	mapped = mmap(addr, size, PROT_READ | PROT_WRITE,
		      MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED, -1, 0);
	if (mapped == MAP_FAILED)
		return NULL;
	if (mapped != addr)
		ferrion_abort("simulator allocation moved outside its reservation");
	allocated_pages[start] = pages;
	for (i = start + 1; i < start + pages; i++)
		allocated_pages[i] = FERRION_SIM_CONTINUATION;
	*phys = start * FERRION_SIM_PAGE_SIZE;
	return addr;
}

void ferrion_phys_free(void *addr, unsigned long size)
{
	unsigned long offset = (uintptr_t)addr - FERRION_PAGE_OFFSET;
	unsigned long start = offset / FERRION_SIM_PAGE_SIZE;
	unsigned long pages = size / FERRION_SIM_PAGE_SIZE, i;
	void *mapped;

	if (offset >= FERRION_SIM_BYTES_MAX || offset % FERRION_SIM_PAGE_SIZE ||
	    !size || size % FERRION_SIM_PAGE_SIZE || pages > FERRION_SIM_PAGES_MAX - start ||
	    allocated_pages[start] != pages)
		ferrion_abort("invalid simulator allocation free");
	mapped = mmap(addr, size, PROT_NONE, MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED, -1, 0);
	if (mapped != addr)
		ferrion_abort("simulator allocation could not be revoked");
	for (i = start; i < start + pages; i++)
		allocated_pages[i] = 0;
}

void ferrion_driver_host_main(void)
{
	ferrion_linux_start();
	ferrion_abort("Wi-Fi init returned without a control service");
}
