/* SPDX-License-Identifier: GPL-2.0-only */
#ifndef FERRION_POOL_H
#define FERRION_POOL_H

#include <linux/types.h>

#define FERRION_PAGE_OFFSET 0x100000000000UL
#define FERRION_VMEMMAP_BASE 0x200000000000UL
#define FERRION_PHYS_LIMIT (1UL << 40)

void ferrion_pool_init(void);
bool ferrion_linear(const void *addr);
void *ferrion_region_alloc(unsigned long size);
void ferrion_region_free(void *va, unsigned long size);
void *ferrion_kmalloc(size_t size, size_t align, gfp_t flags);
void *ferrion_dma_alloc(size_t size, dma_addr_t *handle);
void ferrion_dma_free(void *va, size_t size);

#endif
