// SPDX-License-Identifier: GPL-2.0-only
#include <linux/bitmap.h>
#include <linux/gfp.h>
#include <linux/log2.h>
#include <linux/mm.h>
#include <linux/slab.h>
#include <linux/string.h>
#include <ferrion/linux.h>
#include <ferrion/pool.h>

#define FERRION_CHUNK_PAGES 512
#define FERRION_CHUNKS_MAX 256
#define FERRION_SLAB_MIN_SHIFT 4
#define FERRION_SLAB_MAX_SHIFT 11
#define FERRION_SLAB_CLASSES (FERRION_SLAB_MAX_SHIFT - FERRION_SLAB_MIN_SHIFT + 1)
#define FERRION_KMALLOC_MAX_ORDER 8
#define FERRION_PAGE_SLAB (1UL << 62)
#define FERRION_PAGE_LARGE (1UL << 61)
#define FERRION_PAGE_KIND (FERRION_PAGE_SLAB | FERRION_PAGE_LARGE)

struct chunk {
	unsigned long pfn;
	DECLARE_BITMAP(used, FERRION_CHUNK_PAGES);
};

struct slab_object {
	struct slab_object *next;
};

static struct chunk chunks[FERRION_CHUNKS_MAX];
static unsigned int nr_chunks;
static struct slab_object *slab_free[FERRION_SLAB_CLASSES];

bool ferrion_linear(const void *addr)
{
	unsigned long a = (unsigned long)addr;

	return a >= page_offset_base && a - page_offset_base < FERRION_PHYS_LIMIT;
}

void ferrion_pool_init(void)
{
	unsigned long pages = FERRION_PHYS_LIMIT >> PAGE_SHIFT;

	if (ferrion_reserve((void *)page_offset_base, FERRION_PHYS_LIMIT, 0))
		ferrion_abort("the linear map's address range is taken");
	if (ferrion_reserve((void *)vmemmap_base, PAGE_ALIGN(pages * sizeof(struct page)), 1))
		ferrion_abort("the page array's address range is taken");
}

void *ferrion_region_alloc(unsigned long size)
{
	unsigned long phys;
	void *va;

	size = PAGE_ALIGN(size);
	va = ferrion_phys_alloc(size, &phys);
	if (!va)
		return NULL;
	if ((unsigned long)va != page_offset_base + phys)
		ferrion_abort("physical memory is not in the linear map");
	return va;
}

void ferrion_region_free(void *va, unsigned long size)
{
	ferrion_phys_free(va, PAGE_ALIGN(size));
}

static struct chunk *chunk_of(unsigned long pfn)
{
	unsigned int i;

	for (i = 0; i < nr_chunks; i++)
		if (pfn - chunks[i].pfn < FERRION_CHUNK_PAGES)
			return &chunks[i];
	return NULL;
}

static bool chunk_add(void)
{
	void *va;

	if (nr_chunks == FERRION_CHUNKS_MAX)
		return false;
	va = ferrion_region_alloc(FERRION_CHUNK_PAGES * PAGE_SIZE);
	if (!va)
		return false;
	chunks[nr_chunks].pfn = __pa(va) >> PAGE_SHIFT;
	bitmap_zero(chunks[nr_chunks].used, FERRION_CHUNK_PAGES);
	nr_chunks++;
	return true;
}

static struct page *pool_alloc(unsigned int order, gfp_t gfp)
{
	unsigned int n = 1U << order;
	unsigned int pass, i;

	if (order > ilog2(FERRION_CHUNK_PAGES))
		return NULL;
	for (pass = 0; pass < 2; pass++) {
		for (i = 0; i < nr_chunks; i++) {
			unsigned long idx = bitmap_find_next_zero_area(chunks[i].used, FERRION_CHUNK_PAGES, 0, n, n - 1);
			struct page *page;

			if (idx >= FERRION_CHUNK_PAGES)
				continue;
			bitmap_set(chunks[i].used, idx, n);
			page = pfn_to_page(chunks[i].pfn + idx);
			set_page_count(page, 1);
			page->private = 0;
			if (gfp & __GFP_ZERO)
				memset(page_address(page), 0, PAGE_SIZE << order);
			return page;
		}
		if (pass == 0 && !chunk_add())
			return NULL;
	}
	return NULL;
}

static void pool_free(struct page *page, unsigned int order)
{
	unsigned long pfn = page_to_pfn(page);
	struct chunk *c = chunk_of(pfn);

	if (!c)
		ferrion_abort("free of a page that is not in the pool");
	set_page_count(page, 0);
	page->private = 0;
	bitmap_clear(c->used, pfn - c->pfn, 1U << order);
}

unsigned long get_free_pages_noprof(gfp_t gfp_mask, unsigned int order)
{
	struct page *page = pool_alloc(order, gfp_mask);

	return page ? (unsigned long)page_address(page) : 0;
}

void free_pages(unsigned long addr, unsigned int order)
{
	if (addr)
		pool_free(virt_to_page((void *)addr), order);
}

void *alloc_pages_exact_noprof(size_t size, gfp_t gfp_mask)
{
	return (void *)get_free_pages_noprof(gfp_mask, get_order(size));
}

void free_pages_exact(void *virt, size_t size)
{
	free_pages((unsigned long)virt, get_order(size));
}

void *ferrion_dma_alloc(size_t size, dma_addr_t *handle)
{
	void *va = alloc_pages_exact_noprof(size, GFP_KERNEL | __GFP_ZERO);

	if (va)
		*handle = __pa(va);
	return va;
}

void ferrion_dma_free(void *va, size_t size)
{
	free_pages_exact(va, size);
}

static unsigned int slab_class(size_t size)
{
	unsigned int shift = size <= (1UL << FERRION_SLAB_MIN_SHIFT) ? FERRION_SLAB_MIN_SHIFT : ilog2(size - 1) + 1;

	return shift - FERRION_SLAB_MIN_SHIFT;
}

static void *slab_alloc(unsigned int class)
{
	struct slab_object *obj = slab_free[class];

	if (!obj) {
		size_t size = 1UL << (class + FERRION_SLAB_MIN_SHIFT);
		struct page *page = pool_alloc(0, 0);
		char *base;
		size_t off;

		if (!page)
			return NULL;
		page->private = FERRION_PAGE_SLAB | class;
		base = page_address(page);
		for (off = 0; off < PAGE_SIZE; off += size) {
			struct slab_object *o = (struct slab_object *)(base + off);

			o->next = slab_free[class];
			slab_free[class] = o;
		}
		obj = slab_free[class];
	}
	slab_free[class] = obj->next;
	return obj;
}

void *ferrion_kmalloc(size_t size, size_t align, gfp_t flags)
{
	void *ptr;

	if (!size)
		return ZERO_SIZE_PTR;
	if (size > KMALLOC_MAX_SIZE)
		return NULL;
	size = max(size, align);
	if (size <= (1UL << FERRION_SLAB_MAX_SHIFT)) {
		ptr = slab_alloc(slab_class(size));
	} else if (get_order(size) <= FERRION_KMALLOC_MAX_ORDER) {
		struct page *page = pool_alloc(get_order(size), 0);

		if (!page)
			return NULL;
		page->private = FERRION_PAGE_LARGE | get_order(size);
		ptr = page_address(page);
	} else {
		ptr = ferrion_alloc(size, max_t(size_t, align, PAGE_SIZE));
	}
	if (ptr && (flags & __GFP_ZERO))
		memset(ptr, 0, size);
	return ptr;
}

void kfree(const void *objp)
{
	struct page *page;

	if (ZERO_OR_NULL_PTR(objp))
		return;
	if (!ferrion_linear(objp)) {
		ferrion_free((void *)objp);
		return;
	}
	page = virt_to_page(objp);
	if (page->private & FERRION_PAGE_SLAB) {
		struct slab_object *o = (struct slab_object *)objp;
		unsigned int class = page->private & ~FERRION_PAGE_KIND;

		o->next = slab_free[class];
		slab_free[class] = o;
	} else if (page->private & FERRION_PAGE_LARGE) {
		pool_free(page, page->private & ~FERRION_PAGE_KIND);
	} else {
		ferrion_abort("kfree of memory kmalloc did not hand out");
	}
}

size_t ksize(const void *objp)
{
	struct page *page;

	if (ZERO_OR_NULL_PTR(objp))
		return 0;
	if (!ferrion_linear(objp))
		return ferrion_alloc_usable(objp);
	page = virt_to_page(objp);
	if (page->private & FERRION_PAGE_SLAB)
		return 1UL << ((page->private & ~FERRION_PAGE_KIND) + FERRION_SLAB_MIN_SHIFT);
	return PAGE_SIZE << (page->private & ~FERRION_PAGE_KIND);
}

void __folio_put(struct folio *folio)
{
	ferrion_abort("__folio_put: a page the shim holds lost its last reference");
}
