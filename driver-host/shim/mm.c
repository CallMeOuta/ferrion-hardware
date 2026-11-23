// SPDX-License-Identifier: GPL-2.0-only
#include <linux/err.h>
#include <linux/gfp.h>
#include <linux/ctype.h>
#include <linux/io.h>
#include <linux/log2.h>
#include <linux/mm.h>
#include <linux/random.h>
#include <linux/slab.h>
#include <linux/string.h>
#include <linux/uaccess.h>
#include <ferrion/linux.h>
#include <ferrion/pool.h>

#define FERRION_KMALLOC_MINALIGN 16
#define FERRION_STRNDUP_USER_MAX (PAGE_SIZE * 32)

struct kmem_cache {
	const char *name;
	unsigned int size;
	unsigned int align;
	void (*ctor)(void *);
};

kmem_buckets kmalloc_caches[NR_KMALLOC_TYPES];

static void *kmalloc_any(size_t size, gfp_t flags)
{
	return ferrion_kmalloc(size, FERRION_KMALLOC_MINALIGN, flags);
}

void *__kmalloc_noprof(size_t size, gfp_t flags)
{
	return kmalloc_any(size, flags);
}

void *__kmalloc_node_noprof(size_t size, gfp_t flags, int node)
{
	return kmalloc_any(size, flags);
}

void *__kmalloc_cache_noprof(struct kmem_cache *s, gfp_t flags, size_t size)
{
	return kmalloc_any(size, flags);
}

void *__kmalloc_node_track_caller_noprof(size_t size, gfp_t flags, int node, unsigned long caller)
{
	return kmalloc_any(size, flags);
}

void *__kvmalloc_node_noprof(size_t size, unsigned long align, gfp_t flags, int node)
{
	return ferrion_kmalloc(size, max_t(unsigned long, align, FERRION_KMALLOC_MINALIGN), flags);
}

void kvfree(const void *addr)
{
	kfree(addr);
}

size_t kmalloc_size_roundup(size_t size)
{
	return size > KMALLOC_MAX_SIZE ? size : ALIGN(size, FERRION_KMALLOC_MINALIGN);
}

void *krealloc_node_align_noprof(const void *objp, size_t new_size, unsigned long align, gfp_t flags, int nid)
{
	size_t old_size = ksize(objp);
	void *ptr;

	if (!new_size) {
		kfree(objp);
		return ZERO_SIZE_PTR;
	}
	if (objp && !ZERO_OR_NULL_PTR(objp) && new_size <= old_size && IS_ALIGNED((unsigned long)objp, align))
		return (void *)objp;
	ptr = __kvmalloc_node_noprof(new_size, align, flags & ~__GFP_ZERO, nid);
	if (!ptr)
		return NULL;
	if (!ZERO_OR_NULL_PTR(objp))
		memcpy(ptr, objp, min(old_size, new_size));
	if ((flags & __GFP_ZERO) && new_size > old_size)
		memset((char *)ptr + old_size, 0, new_size - old_size);
	kfree(objp);
	return ptr;
}

void *kmemdup_noprof(const void *src, size_t len, gfp_t gfp)
{
	void *p = kmalloc_any(len, gfp);

	if (p && !ZERO_OR_NULL_PTR(p))
		memcpy(p, src, len);
	return p;
}

char *kstrdup(const char *s, gfp_t gfp)
{
	if (!s)
		return NULL;
	return kmemdup_noprof(s, strlen(s) + 1, gfp);
}

const char *kstrdup_const(const char *s, gfp_t gfp)
{
	return kstrdup(s, gfp);
}

void kfree_const(const void *x)
{
	kfree(x);
}

struct kmem_cache *__kmem_cache_create_args(const char *name, unsigned int object_size,
					    struct kmem_cache_args *args, slab_flags_t flags)
{
	struct kmem_cache *s = kzalloc(sizeof(*s), GFP_KERNEL);

	if (!s)
		return NULL;
	s->name = name;
	s->size = object_size;
	s->align = max_t(unsigned int, args ? args->align : 0, FERRION_KMALLOC_MINALIGN);
	if (flags & SLAB_HWCACHE_ALIGN)
		s->align = max_t(unsigned int, s->align, L1_CACHE_BYTES);
	s->ctor = args ? args->ctor : NULL;
	return s;
}

void kmem_cache_destroy(struct kmem_cache *s)
{
	kfree(s);
}

void *kmem_cache_alloc_noprof(struct kmem_cache *s, gfp_t flags)
{
	void *ptr = ferrion_kmalloc(s->size, s->align, s->ctor ? 0 : flags);

	if (ptr && s->ctor)
		s->ctor(ptr);
	return ptr;
}

void *kmem_cache_alloc_lru_noprof(struct kmem_cache *s, struct list_lru *lru, gfp_t gfpflags)
{
	return kmem_cache_alloc_noprof(s, gfpflags);
}

void kmem_cache_free(struct kmem_cache *s, void *objp)
{
	kfree(objp);
}

bool is_vmalloc_addr(const void *x)
{
	return x && !ferrion_linear(x);
}

int is_vmalloc_or_module_addr(const void *x)
{
	return is_vmalloc_addr(x);
}

void ferrion_user_window(const void *base, unsigned long len)
{
	struct ferrion_task *t = ferrion_current();

	t->user_base = base;
	t->user_len = len;
}

static bool user_range(const void __user *ptr, unsigned long n)
{
	struct ferrion_task *t = ferrion_current();
	unsigned long start = (unsigned long)t->user_base;
	unsigned long addr = (unsigned long)ptr;

	return t->user_len && addr >= start && n <= t->user_len && addr - start <= t->user_len - n;
}

unsigned long _copy_from_user(void *to, const void __user *from, unsigned long n)
{
	if (!user_range(from, n)) {
		memset(to, 0, n);
		return n;
	}
	memcpy(to, (const void __force *)from, n);
	return 0;
}

unsigned long _copy_to_user(void __user *to, const void *from, unsigned long n)
{
	if (!user_range(to, n))
		return n;
	memcpy((void __force *)to, from, n);
	return 0;
}

static long user_strnlen(const char __user *src, long count)
{
	struct ferrion_task *t = ferrion_current();
	unsigned long start = (unsigned long)t->user_base;
	unsigned long addr = (unsigned long)src;
	unsigned long room;

	if (!t->user_len || addr < start || addr - start >= t->user_len || count <= 0)
		return -EFAULT;
	room = min_t(unsigned long, t->user_len - (addr - start), count);
	return strnlen((const char __force *)src, room);
}

long strncpy_from_user(char *dst, const char __user *src, long count)
{
	long len = user_strnlen(src, count);

	if (len < 0)
		return len;
	memcpy(dst, (const char __force *)src, len);
	if (len < count)
		dst[len] = '\0';
	return len;
}

void *memdup_user(const void __user *src, size_t len)
{
	void *p = kmalloc_any(len, GFP_KERNEL);

	if (!p)
		return ERR_PTR(-ENOMEM);
	if (copy_from_user(p, src, len)) {
		kfree(p);
		return ERR_PTR(-EFAULT);
	}
	return p;
}

void *vmemdup_user(const void __user *src, size_t len)
{
	return memdup_user(src, len);
}

void *memdup_user_nul(const void __user *src, size_t len)
{
	char *p = kmalloc_any(len + 1, GFP_KERNEL);

	if (!p)
		return ERR_PTR(-ENOMEM);
	if (copy_from_user(p, src, len)) {
		kfree(p);
		return ERR_PTR(-EFAULT);
	}
	p[len] = '\0';
	return p;
}

char *strndup_user(const char __user *s, long n)
{
	long len = user_strnlen(s, min_t(long, n, FERRION_STRNDUP_USER_MAX));
	char *p;

	if (len < 0)
		return ERR_PTR(len);
	if (len >= n)
		return ERR_PTR(-EINVAL);
	p = memdup_user(s, len + 1);
	if (IS_ERR(p))
		return p;
	p[len] = '\0';
	return p;
}

void memcpy_fromio(void *to, const volatile void __iomem *from, size_t n)
{
	memcpy(to, (const void __force *)from, n);
}

void memcpy_toio(volatile void __iomem *to, const void *from, size_t n)
{
	memcpy((void __force *)to, from, n);
}

ssize_t sized_strscpy(char *dest, const char *src, size_t count)
{
	size_t len;

	if (count == 0 || WARN_ON_ONCE(count > INT_MAX))
		return -E2BIG;
	len = strnlen(src, count);
	if (len == count) {
		memcpy(dest, src, count - 1);
		dest[count - 1] = '\0';
		return -E2BIG;
	}
	memcpy(dest, src, len + 1);
	return len;
}

char *strnchr(const char *s, size_t count, int c)
{
	while (count--) {
		if (*s == (char)c)
			return (char *)s;
		if (*s++ == '\0')
			break;
	}
	return NULL;
}

void *memchr_inv(const void *start, int c, size_t bytes)
{
	const u8 *p = start;
	size_t i;

	for (i = 0; i < bytes; i++)
		if (p[i] != (u8)c)
			return (void *)(p + i);
	return NULL;
}

bool sysfs_streq(const char *s1, const char *s2)
{
	while (*s1 && *s1 == *s2) {
		s1++;
		s2++;
	}
	if (*s1 == *s2)
		return true;
	if (!*s1 && *s2 == '\n' && !s2[1])
		return true;
	if (*s1 == '\n' && !s1[1] && !*s2)
		return true;
	return false;
}

int match_string(const char *const *array, size_t n, const char *string)
{
	size_t index;

	for (index = 0; index < n; index++) {
		if (!array[index])
			break;
		if (!strcmp(array[index], string))
			return index;
	}
	return -EINVAL;
}

void memcpy_and_pad(void *dest, size_t dest_len, const void *src, size_t count, int pad)
{
	if (dest_len > count) {
		memcpy(dest, src, count);
		memset((char *)dest + count, pad, dest_len - count);
	} else {
		memcpy(dest, src, dest_len);
	}
}

long simple_strtol(const char *cp, char **endp, unsigned int base)
{
	bool negative = *cp == '-';
	unsigned long value = 0;

	if (negative)
		cp++;
	if (!base) {
		base = 10;
		if (cp[0] == '0') {
			base = 8;
			if (_tolower(cp[1]) == 'x' && isxdigit(cp[2])) {
				base = 16;
				cp += 2;
			}
		}
	} else if (base == 16 && cp[0] == '0' && _tolower(cp[1]) == 'x') {
		cp += 2;
	}
	for (;; cp++) {
		unsigned int digit;

		if (isdigit(*cp))
			digit = *cp - '0';
		else if (isxdigit(*cp))
			digit = _tolower(*cp) - 'a' + 10;
		else
			break;
		if (digit >= base)
			break;
		value = value * base + digit;
	}
	if (endp)
		*endp = (char *)cp;
	return negative ? -(long)value : (long)value;
}

u32 __get_random_u32_below(u32 ceil)
{
	return (u32)(((u64)ferrion_random_u32() * ceil) >> 32);
}
