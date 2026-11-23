// SPDX-License-Identifier: GPL-2.0-only
#include <linux/aperture.h>
#include <linux/device.h>
#include <linux/dma-mapping.h>
#include <linux/interrupt.h>
#include <linux/io.h>
#include <linux/pci.h>
#include <linux/scatterlist.h>
#include <linux/slab.h>
#include <ferrion/linux.h>
#include <ferrion/pool.h>

#define FERRION_PCI_DRIVERS_MAX 4
#define FERRION_IRQ_INTX 32
#define FERRION_IRQ_ACTIONS_MAX 8
#define FERRION_PCI_CAP_WALK_MAX 48

struct irq_action {
	unsigned int irq;
	irq_handler_t handler;
	void *dev;
	const char *name;
};

static struct pci_driver *pci_drivers[FERRION_PCI_DRIVERS_MAX];
static struct irq_action irq_actions[FERRION_IRQ_ACTIONS_MAX];
static bool irq_started;
static void __iomem *bar_maps[PCI_STD_NUM_BARS];
static struct pci_bus ferrion_bus;
static struct pci_dev *ferrion_pdev;

int __pci_register_driver(struct pci_driver *drv, struct module *owner, const char *mod_name)
{
	unsigned int i;

	for (i = 0; i < FERRION_PCI_DRIVERS_MAX; i++) {
		if (!pci_drivers[i]) {
			pci_drivers[i] = drv;
			drv->driver.name = drv->name;
			drv->driver.bus = &pci_bus_type;
			return 0;
		}
	}
	return -ENOSPC;
}

void pci_unregister_driver(struct pci_driver *drv)
{
	unsigned int i;

	for (i = 0; i < FERRION_PCI_DRIVERS_MAX; i++)
		if (pci_drivers[i] == drv)
			pci_drivers[i] = NULL;
}

int pci_read_config_dword(const struct pci_dev *dev, int where, u32 *val)
{
	if (where < 0 || where > PCI_CFG_SPACE_EXP_SIZE - 4 || (where & 3)) {
		*val = ~0U;
		return PCIBIOS_BAD_REGISTER_NUMBER;
	}
	*val = ferrion_pci_read_config(where);
	return PCIBIOS_SUCCESSFUL;
}

int pci_read_config_word(const struct pci_dev *dev, int where, u16 *val)
{
	u32 v;

	if (where & 1) {
		*val = ~0;
		return PCIBIOS_BAD_REGISTER_NUMBER;
	}
	pci_read_config_dword(dev, where & ~3, &v);
	*val = v >> ((where & 3) * 8);
	return PCIBIOS_SUCCESSFUL;
}

int pci_read_config_byte(const struct pci_dev *dev, int where, u8 *val)
{
	u32 v;

	pci_read_config_dword(dev, where & ~3, &v);
	*val = v >> ((where & 3) * 8);
	return PCIBIOS_SUCCESSFUL;
}

static int write_config(int where, unsigned int bytes, u32 val)
{
	unsigned int shift = (where & 3) * 8;
	u32 mask = bytes == 4 ? ~0U : ((1U << (bytes * 8)) - 1) << shift;
	u32 old;

	if (where < 0 || where > PCI_CFG_SPACE_EXP_SIZE - 4 || (where & (bytes - 1)))
		return PCIBIOS_BAD_REGISTER_NUMBER;
	old = bytes == 4 ? 0 : ferrion_pci_read_config(where & ~3);
	ferrion_pci_write_config(where & ~3, (old & ~mask) | ((val << shift) & mask));
	return PCIBIOS_SUCCESSFUL;
}

int pci_write_config_dword(const struct pci_dev *dev, int where, u32 val)
{
	return write_config(where, 4, val);
}

int pci_write_config_word(const struct pci_dev *dev, int where, u16 val)
{
	return write_config(where, 2, val);
}

int pci_write_config_byte(const struct pci_dev *dev, int where, u8 val)
{
	return write_config(where, 1, val);
}

u8 pci_find_next_capability(struct pci_dev *dev, u8 pos, int cap)
{
	unsigned int n;
	u8 next;

	pci_read_config_byte(dev, pos, &next);
	for (n = 0; n < FERRION_PCI_CAP_WALK_MAX && next >= PCI_STD_HEADER_SIZEOF; n++) {
		u8 id;

		pos = next & ~3;
		pci_read_config_byte(dev, pos + PCI_CAP_LIST_ID, &id);
		if (id == 0xff)
			break;
		if (id == cap)
			return pos;
		pci_read_config_byte(dev, pos + PCI_CAP_LIST_NEXT, &next);
	}
	return 0;
}

u8 pci_find_capability(struct pci_dev *dev, int cap)
{
	u16 status;

	pci_read_config_word(dev, PCI_STATUS, &status);
	if (!(status & PCI_STATUS_CAP_LIST))
		return 0;
	return pci_find_next_capability(dev, PCI_CAPABILITY_LIST, cap);
}

u16 pci_find_ext_capability(struct pci_dev *dev, int cap)
{
	int pos = PCI_CFG_SPACE_SIZE;
	unsigned int n;
	u32 header;

	if (dev->cfg_size <= PCI_CFG_SPACE_SIZE)
		return 0;
	for (n = 0; n < (PCI_CFG_SPACE_EXP_SIZE - PCI_CFG_SPACE_SIZE) / 8; n++) {
		pci_read_config_dword(dev, pos, &header);
		if (header == 0 || header == ~0U)
			return 0;
		if (PCI_EXT_CAP_ID(header) == cap)
			return pos;
		pos = PCI_EXT_CAP_NEXT(header);
		if (pos < PCI_CFG_SPACE_SIZE)
			return 0;
	}
	return 0;
}

int pci_enable_device(struct pci_dev *dev)
{
	u16 cmd;

	pci_read_config_word(dev, PCI_COMMAND, &cmd);
	pci_write_config_word(dev, PCI_COMMAND, cmd | PCI_COMMAND_MEMORY | PCI_COMMAND_IO);
	atomic_inc(&dev->enable_cnt);
	return 0;
}

void pci_disable_device(struct pci_dev *dev)
{
	atomic_dec(&dev->enable_cnt);
}

void pci_set_master(struct pci_dev *dev)
{
	u16 cmd;

	pci_read_config_word(dev, PCI_COMMAND, &cmd);
	pci_write_config_word(dev, PCI_COMMAND, cmd | PCI_COMMAND_MASTER);
	dev->is_busmaster = 1;
}

bool pci_device_is_present(struct pci_dev *pdev)
{
	u32 v;

	pci_read_config_dword(pdev, PCI_VENDOR_ID, &v);
	return (v & 0xffff) != 0xffff;
}

int pci_request_selected_regions(struct pci_dev *pdev, int bars, const char *res_name)
{
	return 0;
}

void pci_release_selected_regions(struct pci_dev *pdev, int bars)
{
}

void __iomem *pci_iomap_range(struct pci_dev *dev, int bar, unsigned long offset, unsigned long maxlen)
{
	resource_size_t len = pci_resource_len(dev, bar);

	if (bar < 0 || bar >= PCI_STD_NUM_BARS || !len || offset >= len)
		return NULL;
	if (pci_resource_flags(dev, bar) & IORESOURCE_IO)
		return NULL;
	if (!bar_maps[bar])
		bar_maps[bar] = (void __iomem *)ferrion_pci_map_bar(bar);
	if (!bar_maps[bar])
		return NULL;
	return bar_maps[bar] + offset;
}

void pci_iounmap(struct pci_dev *dev, void __iomem *addr)
{
}

unsigned int ioread8(const void __iomem *addr)
{
	return readb(addr);
}

unsigned int ioread16(const void __iomem *addr)
{
	return readw(addr);
}

unsigned int ioread32(const void __iomem *addr)
{
	return readl(addr);
}

void iowrite8(u8 value, void __iomem *addr)
{
	writeb(value, addr);
}

void iowrite16(u16 value, void __iomem *addr)
{
	writew(value, addr);
}

void iowrite32(u32 value, void __iomem *addr)
{
	writel(value, addr);
}

int request_threaded_irq(unsigned int irq, irq_handler_t handler, irq_handler_t thread_fn, unsigned long irqflags,
			 const char *devname, void *dev_id)
{
	unsigned int i;

	if (thread_fn)
		ferrion_refuse("request_threaded_irq with a thread function");
	if (irq != FERRION_IRQ_INTX || !handler)
		return -EINVAL;
	for (i = 0; i < FERRION_IRQ_ACTIONS_MAX; i++) {
		if (!irq_actions[i].handler) {
			irq_actions[i] = (struct irq_action){ irq, handler, dev_id, devname };
			if (!irq_started) {
				int ret = ferrion_pci_irq_start(irq);

				if (ret) {
					irq_actions[i] = (struct irq_action){ 0 };
					return ret;
				}
				irq_started = true;
			}
			return 0;
		}
	}
	return -ENOSPC;
}

const void *free_irq(unsigned int irq, void *dev_id)
{
	unsigned int i;

	for (i = 0; i < FERRION_IRQ_ACTIONS_MAX; i++) {
		if (irq_actions[i].handler && irq_actions[i].irq == irq && irq_actions[i].dev == dev_id) {
			const char *name = irq_actions[i].name;

			irq_actions[i] = (struct irq_action){ 0 };
			return name;
		}
	}
	return NULL;
}

void synchronize_irq(unsigned int irq)
{
}

void ferrion_irq_fire(unsigned int irq)
{
	unsigned long flags;
	unsigned int i;

	ferrion_enter();
	local_irq_save(flags);
	preempt_count_add(HARDIRQ_OFFSET);
	for (i = 0; i < FERRION_IRQ_ACTIONS_MAX; i++)
		if (irq_actions[i].handler && irq_actions[i].irq == irq)
			irq_actions[i].handler(irq, irq_actions[i].dev);
	preempt_count_sub(HARDIRQ_OFFSET);
	local_irq_restore(flags);
	ferrion_leave();
}

dma_addr_t dma_map_page_attrs(struct device *dev, struct page *page, size_t offset, size_t size,
			      enum dma_data_direction dir, unsigned long attrs)
{
	return page_to_phys(page) + offset;
}

void dma_unmap_page_attrs(struct device *dev, dma_addr_t addr, size_t size, enum dma_data_direction dir,
			  unsigned long attrs)
{
}

int dma_map_sgtable(struct device *dev, struct sg_table *sgt, enum dma_data_direction dir, unsigned long attrs)
{
	struct scatterlist *sg;
	int i;

	for_each_sgtable_sg(sgt, sg, i) {
		sg->dma_address = sg_phys(sg);
		sg_dma_len(sg) = sg->length;
	}
	sgt->nents = sgt->orig_nents;
	return 0;
}

void dma_unmap_sg_attrs(struct device *dev, struct scatterlist *sg, int nents, enum dma_data_direction dir,
			unsigned long attrs)
{
}

dma_addr_t dma_map_resource(struct device *dev, phys_addr_t phys_addr, size_t size, enum dma_data_direction dir,
			    unsigned long attrs)
{
	return phys_addr;
}

void dma_unmap_resource(struct device *dev, dma_addr_t addr, size_t size, enum dma_data_direction dir,
			unsigned long attrs)
{
}

void *dma_alloc_attrs(struct device *dev, size_t size, dma_addr_t *dma_handle, gfp_t flag, unsigned long attrs)
{
	return ferrion_dma_alloc(size, dma_handle);
}

void dma_free_attrs(struct device *dev, size_t size, void *cpu_addr, dma_addr_t dma_handle, unsigned long attrs)
{
	if (cpu_addr)
		ferrion_dma_free(cpu_addr, size);
}

int dma_set_mask(struct device *dev, u64 mask)
{
	if (!dev->dma_mask)
		return -EIO;
	*dev->dma_mask = mask;
	return 0;
}

int dma_set_coherent_mask(struct device *dev, u64 mask)
{
	dev->coherent_dma_mask = mask;
	return 0;
}

size_t dma_max_mapping_size(struct device *dev)
{
	return SIZE_MAX;
}

bool __dma_need_sync(struct device *dev, dma_addr_t dma_addr)
{
	return false;
}

void __dma_sync_single_for_cpu(struct device *dev, dma_addr_t addr, size_t size, enum dma_data_direction dir)
{
}

void __dma_sync_single_for_device(struct device *dev, dma_addr_t addr, size_t size, enum dma_data_direction dir)
{
}

void __dma_sync_sg_for_cpu(struct device *dev, struct scatterlist *sg, int nelems, enum dma_data_direction dir)
{
}

void __dma_sync_sg_for_device(struct device *dev, struct scatterlist *sg, int nelems, enum dma_data_direction dir)
{
}

static const struct pci_device_id *match_id(const struct pci_device_id *ids, struct pci_dev *dev)
{
	for (; ids && (ids->vendor || ids->subvendor || ids->class_mask); ids++) {
		if ((ids->vendor == PCI_ANY_ID || ids->vendor == dev->vendor) &&
		    (ids->device == PCI_ANY_ID || ids->device == dev->device) &&
		    (ids->subvendor == PCI_ANY_ID || ids->subvendor == dev->subsystem_vendor) &&
		    (ids->subdevice == PCI_ANY_ID || ids->subdevice == dev->subsystem_device) &&
		    !((ids->class ^ dev->class) & ids->class_mask))
			return ids;
	}
	return NULL;
}

static void pdev_release(struct device *dev)
{
	kfree(to_pci_dev(dev));
}

static struct pci_dev *pdev_new(const struct ferrion_pci_info *info)
{
	struct pci_dev *pdev = kzalloc(sizeof(*pdev), GFP_KERNEL);
	unsigned int i;
	u8 hdr;
	u32 ext;

	if (!pdev)
		return NULL;
	ferrion_bus.number = info->bus;
	pdev->bus = &ferrion_bus;
	pdev->devfn = info->devfn;
	pdev->vendor = info->vendor;
	pdev->device = info->device;
	pdev->subsystem_vendor = info->subsystem_vendor;
	pdev->subsystem_device = info->subsystem_device;
	pdev->class = info->class;
	pdev->revision = info->revision;
	pdev->irq = info->has_irq ? FERRION_IRQ_INTX : 0;
	pdev->current_state = PCI_D0;
	pdev->dma_mask = DMA_BIT_MASK(32);
	pdev->dev.dma_mask = &pdev->dma_mask;
	pdev->dev.coherent_dma_mask = DMA_BIT_MASK(32);
	pdev->dev.bus = &pci_bus_type;
	pdev->dev.release = pdev_release;
	device_initialize(&pdev->dev);
	pci_read_config_byte(pdev, PCI_HEADER_TYPE, &hdr);
	pdev->hdr_type = hdr & PCI_HEADER_TYPE_MASK;
	pdev->multifunction = !!(hdr & PCI_HEADER_TYPE_MFD);
	pci_read_config_dword(pdev, PCI_CFG_SPACE_SIZE, &ext);
	pdev->cfg_size = ext != ~0U && ext != 0 ? PCI_CFG_SPACE_EXP_SIZE : PCI_CFG_SPACE_SIZE;
	for (i = 0; i < PCI_STD_NUM_BARS; i++) {
		const struct ferrion_pci_bar *bar = &info->bars[i];
		struct resource *res = &pdev->resource[i];

		if (!bar->size)
			continue;
		res->name = "ferrion-bar";
		res->start = bar->phys;
		res->end = bar->phys + bar->size - 1;
		res->flags = bar->flags & FERRION_PCI_BAR_IO ? IORESOURCE_IO : IORESOURCE_MEM;
		if (bar->flags & FERRION_PCI_BAR_64)
			res->flags |= IORESOURCE_MEM_64;
		if (bar->flags & FERRION_PCI_BAR_PREFETCH)
			res->flags |= IORESOURCE_PREFETCH;
	}
	if (dev_set_name(&pdev->dev, "0000:%02x:%02x.%d", info->bus, PCI_SLOT(info->devfn), PCI_FUNC(info->devfn))) {
		kfree(pdev);
		return NULL;
	}
	return pdev;
}

int ferrion_pci_attach(void)
{
	struct ferrion_pci_info info = { 0 };
	struct pci_dev *pdev;
	unsigned int i;
	int ret = -ENODEV;

	if (ferrion_pci_info(&info))
		return -ENODEV;
	ferrion_enter();
	pdev = pdev_new(&info);
	if (!pdev) {
		ferrion_leave();
		return -ENOMEM;
	}
	for (i = 0; i < FERRION_PCI_DRIVERS_MAX && ret == -ENODEV; i++) {
		struct pci_driver *drv = pci_drivers[i];
		const struct pci_device_id *id = drv ? match_id(drv->id_table, pdev) : NULL;

		if (!id)
			continue;
		pdev->driver = drv;
		pdev->dev.driver = &drv->driver;
		ret = drv->probe(pdev, id);
		if (ret) {
			pdev->driver = NULL;
			pdev->dev.driver = NULL;
			dev_err(&pdev->dev, "probe by %s failed: %d\n", drv->name, ret);
		} else {
			dev_info(&pdev->dev, "bound to %s\n", drv->name);
		}
	}
	if (!ret) {
		ferrion_pdev = pdev;
		get_device(&pdev->dev);
	}
	ferrion_leave();
	return ret;
}

struct pci_dev *pci_get_device(unsigned int vendor, unsigned int device, struct pci_dev *from)
{
	struct pci_dev *pdev = ferrion_pdev;

	if (from)
		pci_dev_put(from);
	if (!pdev || from == pdev)
		return NULL;
	if ((vendor != PCI_ANY_ID && vendor != pdev->vendor) || (device != PCI_ANY_ID && device != pdev->device))
		return NULL;
	get_device(&pdev->dev);
	return pdev;
}

void pci_dev_put(struct pci_dev *dev)
{
	if (dev)
		put_device(&dev->dev);
}

int aperture_remove_conflicting_pci_devices(struct pci_dev *pdev, const char *name)
{
	return 0;
}

struct resource *__devm_request_region(struct device *dev, struct resource *parent, resource_size_t start,
				       resource_size_t n, const char *name)
{
	struct resource *res = devm_kzalloc(dev, sizeof(*res), GFP_KERNEL);

	if (!res)
		return NULL;
	res->name = name;
	res->start = start;
	res->end = start + n - 1;
	res->flags = parent ? parent->flags : IORESOURCE_MEM;
	res->parent = parent;
	return res;
}
