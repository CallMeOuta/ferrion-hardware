// SPDX-License-Identifier: GPL-2.0-only
#include <linux/fs.h>
#include <linux/file.h>
#include <linux/mm.h>
#include <linux/mount.h>
#include <linux/pagemap.h>
#include <linux/pagevec.h>
#include <linux/shmem_fs.h>
#include <linux/slab.h>
#include <linux/swap.h>
#include <linux/vmalloc.h>
#include <ferrion/linux.h>
#include <ferrion/pool.h>

struct ferrion_mount {
	struct vfsmount mnt;
	struct super_block sb;
};

struct ferrion_shmem {
	struct file file;
	struct inode inode;
	void *va;
	unsigned long size;
};

static const struct file_operations ferrion_shmem_fops;
static unsigned long next_ino = 1;

struct vfsmount *kern_mount(struct file_system_type *type)
{
	struct ferrion_mount *m = kzalloc(sizeof(*m), GFP_KERNEL);

	if (!m)
		return ERR_PTR(-ENOMEM);
	m->sb.s_type = type;
	m->sb.s_flags = SB_KERNMOUNT;
	m->mnt.mnt_sb = &m->sb;
	m->mnt.mnt_root = NULL;
	return &m->mnt;
}

void kern_unmount(struct vfsmount *mnt)
{
	if (!IS_ERR_OR_NULL(mnt))
		kfree(container_of(mnt, struct ferrion_mount, mnt));
}

int simple_pin_fs(struct file_system_type *type, struct vfsmount **mount, int *count)
{
	if (!*mount) {
		struct vfsmount *mnt = kern_mount(type);

		if (IS_ERR(mnt))
			return PTR_ERR(mnt);
		*mount = mnt;
	}
	++*count;
	return 0;
}

void simple_release_fs(struct vfsmount **mount, int *count)
{
	if (--*count == 0) {
		kern_unmount(*mount);
		*mount = NULL;
	}
}

static void inode_setup(struct inode *inode, struct super_block *sb, umode_t mode)
{
	struct address_space *mapping = &inode->i_data;

	inode->i_sb = sb;
	inode->i_ino = next_ino++;
	inode->i_mode = mode;
	inode->i_mapping = mapping;
	atomic_set(&inode->i_count, 1);
	mapping->host = inode;
	xa_init_flags(&mapping->i_pages, XA_FLAGS_LOCK_IRQ);
	mapping->i_mmap = RB_ROOT_CACHED;
	mapping_set_gfp_mask(mapping, GFP_HIGHUSER_MOVABLE);
}

struct inode *alloc_anon_inode(struct super_block *sb)
{
	struct inode *inode = kzalloc(sizeof(*inode), GFP_KERNEL);

	if (!inode)
		return ERR_PTR(-ENOMEM);
	inode_setup(inode, sb, S_IFREG | S_IRUSR | S_IWUSR);
	return inode;
}

void iput(struct inode *inode)
{
	if (inode && atomic_dec_and_test(&inode->i_count))
		kfree(inode);
}

static struct ferrion_shmem *shmem_of_mapping(struct address_space *mapping)
{
	return container_of(mapping, struct ferrion_shmem, inode.i_data);
}

struct file *shmem_file_setup(const char *name, loff_t size, unsigned long flags)
{
	struct ferrion_shmem *sh;
	unsigned long pages, i;

	if (size <= 0 || size > FERRION_PHYS_LIMIT)
		return ERR_PTR(-EINVAL);
	sh = kzalloc(sizeof(*sh), GFP_KERNEL);
	if (!sh)
		return ERR_PTR(-ENOMEM);
	sh->size = PAGE_ALIGN(size);
	sh->va = ferrion_region_alloc(sh->size);
	if (!sh->va) {
		kfree(sh);
		return ERR_PTR(-ENOMEM);
	}
	inode_setup(&sh->inode, NULL, S_IFREG | S_IRUSR | S_IWUSR);
	i_size_write(&sh->inode, size);
	pages = sh->size >> PAGE_SHIFT;
	for (i = 0; i < pages; i++) {
		struct page *page = virt_to_page((char *)sh->va + i * PAGE_SIZE);

		set_page_count(page, 1);
		page->mapping = &sh->inode.i_data;
		page->__folio_index = i;
	}
	file_ref_init(&sh->file.f_ref, 1);
	sh->file.f_op = &ferrion_shmem_fops;
	sh->file.f_inode = &sh->inode;
	sh->file.f_mapping = &sh->inode.i_data;
	sh->file.f_mode = FMODE_READ | FMODE_WRITE;
	return &sh->file;
}

struct file *shmem_file_setup_with_mnt(struct vfsmount *mnt, const char *name, loff_t size, unsigned long flags)
{
	return shmem_file_setup(name, size, flags);
}

struct folio *shmem_read_folio_gfp(struct address_space *mapping, pgoff_t index, gfp_t gfp)
{
	struct ferrion_shmem *sh = shmem_of_mapping(mapping);
	struct folio *folio;

	if (index >= sh->size >> PAGE_SHIFT)
		return ERR_PTR(-EINVAL);
	folio = page_folio(virt_to_page((char *)sh->va + index * PAGE_SIZE));
	folio_get(folio);
	return folio;
}

void shmem_truncate_range(struct inode *inode, loff_t start, loff_t end)
{
}

static void shmem_release(struct ferrion_shmem *sh)
{
	unsigned long pages = sh->size >> PAGE_SHIFT, i;

	for (i = 0; i < pages; i++) {
		struct page *page = virt_to_page((char *)sh->va + i * PAGE_SIZE);

		if (page_count(page) != 1)
			ferrion_abort("shmem file released while its pages are held");
		set_page_count(page, 0);
		page->mapping = NULL;
	}
	ferrion_region_free(sh->va, sh->size);
	kfree(sh);
}

void fput(struct file *file)
{
	if (atomic_long_dec_return(&file->f_ref.refcnt) >= 0)
		return;
	if (file->f_op == &ferrion_shmem_fops) {
		shmem_release(container_of(file, struct ferrion_shmem, file));
		return;
	}
	if (file->f_op && file->f_op->release)
		file->f_op->release(file->f_inode, file);
	iput(file->f_inode);
	kfree(file);
}

void __folio_batch_release(struct folio_batch *fbatch)
{
	unsigned int i;

	for (i = 0; i < folio_batch_count(fbatch); i++)
		folio_put(fbatch->folios[i]);
	folio_batch_reinit(fbatch);
}

void folio_mark_accessed(struct folio *folio)
{
}

bool folio_mark_dirty(struct folio *folio)
{
	return !folio_test_set_dirty(folio);
}

void check_move_unevictable_folios(struct folio_batch *fbatch)
{
}

unsigned long invalidate_mapping_pages(struct address_space *mapping, pgoff_t start, pgoff_t end)
{
	return 0;
}

void unmap_mapping_range(struct address_space *mapping, loff_t const holebegin, loff_t const holelen,
			 int even_cows)
{
}

void *vmap(struct page **pages, unsigned int count, unsigned long flags, pgprot_t prot)
{
	unsigned long pfn = page_to_pfn(pages[0]);
	unsigned int i;

	for (i = 1; i < count; i++)
		if (page_to_pfn(pages[i]) != pfn + i)
			ferrion_refuse("vmap of pages that are not one run");
	return page_address(pages[0]);
}

void vunmap(const void *addr)
{
}

struct page *vmalloc_to_page(const void *addr)
{
	if (!ferrion_linear(addr))
		ferrion_refuse("vmalloc_to_page outside the linear map");
	return virt_to_page(addr);
}
