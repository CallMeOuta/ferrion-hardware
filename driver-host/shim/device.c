// SPDX-License-Identifier: GPL-2.0-only
#include <linux/device.h>
#include <linux/device/class.h>
#include <linux/dmi.h>
#include <linux/fs.h>
#include <linux/highuid.h>
#include <linux/ioport.h>
#include <linux/kobject.h>
#include <linux/moduleparam.h>
#include <linux/pci.h>
#include <linux/property.h>
#include <linux/slab.h>
#include <linux/sysfs.h>
#include <linux/vgaarb.h>
#include <video/cmdline.h>
#include <video/nomodeset.h>
#include <asm/video.h>
#include <ferrion/linux.h>

#define FERRION_CHRDEVS_MAX 8
#define FERRION_BUSES_MAX 4
#define FERRION_DRIVERS_MAX 16
#define FERRION_BUS_DEVICES_MAX 32

struct devres {
	struct list_head node;
	dr_release_t release;
	size_t size;
	u8 __aligned(ARCH_DMA_MINALIGN) data[];
};

struct action_devres {
	void (*action)(void *);
	void *data;
};

struct chrdev {
	unsigned int major;
	const char *name;
	const struct file_operations *fops;
};

static struct chrdev chrdevs[FERRION_CHRDEVS_MAX];

int overflowuid = DEFAULT_OVERFLOWUID;

struct resource iomem_resource = {
	.name = "PCI mem",
	.start = 0,
	.end = -1,
	.flags = IORESOURCE_MEM,
};

const struct bus_type pci_bus_type = {
	.name = "pci",
};

static struct devres *devres_of(void *res)
{
	return container_of(res, struct devres, data);
}

void *__devres_alloc_node(dr_release_t release, size_t size, gfp_t gfp, int nid, const char *name)
{
	struct devres *dr = kzalloc(sizeof(*dr) + size, gfp & ~__GFP_ZERO);

	if (!dr)
		return NULL;
	INIT_LIST_HEAD(&dr->node);
	dr->release = release;
	dr->size = size;
	return dr->data;
}

void devres_free(void *res)
{
	struct devres *dr;

	if (!res)
		return;
	dr = devres_of(res);
	if (!list_empty(&dr->node))
		ferrion_abort("devres_free: the resource is still on a device");
	kfree(dr);
}

void devres_add(struct device *dev, void *res)
{
	list_add_tail(&devres_of(res)->node, &dev->devres_head);
}

static void devm_kmalloc_release(struct device *dev, void *res)
{
}

void *devm_kmalloc(struct device *dev, size_t size, gfp_t gfp)
{
	void *res = __devres_alloc_node(devm_kmalloc_release, size, gfp | __GFP_ZERO, NUMA_NO_NODE, "devm_kmalloc");

	if (res)
		devres_add(dev, res);
	return res;
}

static void devres_release_all(struct device *dev)
{
	while (!list_empty(&dev->devres_head)) {
		struct devres *dr = list_last_entry(&dev->devres_head, struct devres, node);

		list_del_init(&dr->node);
		dr->release(dev, dr->data);
		kfree(dr);
	}
}

static void action_release(struct device *dev, void *res)
{
	struct action_devres *devres = res;

	devres->action(devres->data);
}

int __devm_add_action(struct device *dev, void (*action)(void *), void *data, const char *name)
{
	struct action_devres *devres = __devres_alloc_node(action_release, sizeof(*devres), GFP_KERNEL,
							   NUMA_NO_NODE, name);

	if (!devres)
		return -ENOMEM;
	devres->action = action;
	devres->data = data;
	devres_add(dev, devres);
	return 0;
}

void devm_release_action(struct device *dev, void (*action)(void *), void *data)
{
	struct devres *dr;

	list_for_each_entry_reverse(dr, &dev->devres_head, node) {
		struct action_devres *devres = (struct action_devres *)dr->data;

		if (dr->release == action_release && devres->action == action && devres->data == data) {
			list_del_init(&dr->node);
			action(data);
			kfree(dr);
			return;
		}
	}
	WARN(1, "devm_release_action: no such action\n");
}

void device_initialize(struct device *dev)
{
	kref_init(&dev->kobj.kref);
	INIT_LIST_HEAD(&dev->kobj.entry);
	dev->kobj.state_initialized = 1;
	mutex_init(&dev->mutex);
	spin_lock_init(&dev->devres_lock);
	INIT_LIST_HEAD(&dev->devres_head);
}

int dev_set_name(struct device *dev, const char *fmt, ...)
{
	va_list args;
	char *name;
	int len;

	va_start(args, fmt);
	len = ferrion_vformat(NULL, 0, fmt, args);
	va_end(args);
	name = kmalloc(len + 1, GFP_KERNEL);
	if (!name)
		return -ENOMEM;
	va_start(args, fmt);
	ferrion_vformat(name, len + 1, fmt, args);
	va_end(args);
	kfree_const(dev->kobj.name);
	dev->kobj.name = name;
	return 0;
}

static const struct bus_type *buses[FERRION_BUSES_MAX];
static struct device_driver *drivers[FERRION_DRIVERS_MAX];
static struct device *bus_devices[FERRION_BUS_DEVICES_MAX];

int bus_register(const struct bus_type *bus)
{
	unsigned int i;

	for (i = 0; i < FERRION_BUSES_MAX; i++) {
		if (!buses[i]) {
			buses[i] = bus;
			return 0;
		}
	}
	return -ENOSPC;
}

void bus_unregister(const struct bus_type *bus)
{
	unsigned int i;

	for (i = 0; i < FERRION_BUSES_MAX; i++)
		if (buses[i] == bus)
			buses[i] = NULL;
}

static int bind(struct device *dev, struct device_driver *drv)
{
	int ret;

	if (dev->driver || drv->bus != dev->bus)
		return -ENODEV;
	if (dev->bus->match && !dev->bus->match(dev, drv))
		return -ENODEV;
	dev->driver = drv;
	if (dev->bus->probe)
		ret = dev->bus->probe(dev);
	else
		ret = drv->probe ? drv->probe(dev) : 0;
	if (ret) {
		dev->driver = NULL;
		dev_err(dev, "probe by %s failed: %d\n", drv->name, ret);
	}
	return ret;
}

static void unbind(struct device *dev)
{
	struct device_driver *drv = dev->driver;

	if (!drv)
		return;
	if (dev->bus->remove)
		dev->bus->remove(dev);
	else if (drv->remove)
		drv->remove(dev);
	devres_release_all(dev);
	dev->driver = NULL;
}

int driver_register(struct device_driver *drv)
{
	unsigned int i;

	for (i = 0; i < FERRION_DRIVERS_MAX && drivers[i]; i++)
		;
	if (i == FERRION_DRIVERS_MAX)
		return -ENOSPC;
	drivers[i] = drv;
	for (i = 0; i < FERRION_BUS_DEVICES_MAX; i++)
		if (bus_devices[i])
			bind(bus_devices[i], drv);
	return 0;
}

void driver_unregister(struct device_driver *drv)
{
	unsigned int i;

	for (i = 0; i < FERRION_BUS_DEVICES_MAX; i++)
		if (bus_devices[i] && bus_devices[i]->driver == drv)
			unbind(bus_devices[i]);
	for (i = 0; i < FERRION_DRIVERS_MAX; i++)
		if (drivers[i] == drv)
			drivers[i] = NULL;
}

int device_add(struct device *dev)
{
	unsigned int i;

	if (dev->init_name) {
		int ret = dev_set_name(dev, "%s", dev->init_name);

		if (ret)
			return ret;
		dev->init_name = NULL;
	}
	if (!dev_name(dev))
		return -EINVAL;
	get_device(dev->parent);
	dev->kobj.state_in_sysfs = 1;
	if (!dev->bus)
		return 0;
	for (i = 0; i < FERRION_BUS_DEVICES_MAX && bus_devices[i]; i++)
		;
	if (i == FERRION_BUS_DEVICES_MAX) {
		put_device(dev->parent);
		return -ENOSPC;
	}
	bus_devices[i] = dev;
	for (i = 0; i < FERRION_DRIVERS_MAX && !dev->driver; i++)
		if (drivers[i])
			bind(dev, drivers[i]);
	return 0;
}

void device_del(struct device *dev)
{
	unsigned int i;

	if (dev->bus) {
		unbind(dev);
		for (i = 0; i < FERRION_BUS_DEVICES_MAX; i++)
			if (bus_devices[i] == dev)
				bus_devices[i] = NULL;
	}
	dev->kobj.state_in_sysfs = 0;
	put_device(dev->parent);
}

int device_register(struct device *dev)
{
	device_initialize(dev);
	return device_add(dev);
}

void device_unregister(struct device *dev)
{
	device_del(dev);
	put_device(dev);
}

struct device *get_device(struct device *dev)
{
	if (dev)
		kref_get(&dev->kobj.kref);
	return dev;
}

static void device_release(struct kref *kref)
{
	struct device *dev = container_of(kref, struct device, kobj.kref);
	const char *name = dev->kobj.name;

	devres_release_all(dev);
	if (dev->release)
		dev->release(dev);
	else if (dev->type && dev->type->release)
		dev->type->release(dev);
	else if (dev->class && dev->class->dev_release)
		dev->class->dev_release(dev);
	else
		WARN(1, "device %s has no release function\n", name);
	kfree_const(name);
}

void put_device(struct device *dev)
{
	if (dev)
		kref_put(&dev->kobj.kref, device_release);
}

const char *dev_driver_string(const struct device *dev)
{
	struct device_driver *drv = READ_ONCE(dev->driver);

	if (drv)
		return drv->name;
	if (dev->bus)
		return dev->bus->name;
	if (dev->class)
		return dev->class->name;
	return "";
}

struct class *class_create(const char *name)
{
	struct class *cls = kzalloc(sizeof(*cls), GFP_KERNEL);

	if (!cls)
		return ERR_PTR(-ENOMEM);
	cls->name = name;
	return cls;
}

void class_destroy(const struct class *cls)
{
	if (!IS_ERR_OR_NULL(cls))
		kfree(cls);
}

int class_create_file_ns(const struct class *cls, const struct class_attribute *attr, const void *ns)
{
	return 0;
}

void class_remove_file_ns(const struct class *cls, const struct class_attribute *attr, const void *ns)
{
}

int sysfs_create_link(struct kobject *kobj, struct kobject *target, const char *name)
{
	return 0;
}

void sysfs_remove_link(struct kobject *kobj, const char *name)
{
}

int __register_chrdev(unsigned int major, unsigned int baseminor, unsigned int count, const char *name,
		      const struct file_operations *fops)
{
	unsigned int i;

	if (!major)
		return -EINVAL;
	for (i = 0; i < FERRION_CHRDEVS_MAX; i++) {
		if (chrdevs[i].major == major)
			return -EBUSY;
	}
	for (i = 0; i < FERRION_CHRDEVS_MAX; i++) {
		if (!chrdevs[i].major) {
			chrdevs[i] = (struct chrdev){ .major = major, .name = name, .fops = fops };
			return 0;
		}
	}
	return -ENOSPC;
}

void __unregister_chrdev(unsigned int major, unsigned int baseminor, unsigned int count, const char *name)
{
	unsigned int i;

	for (i = 0; i < FERRION_CHRDEVS_MAX; i++)
		if (chrdevs[i].major == major)
			chrdevs[i] = (struct chrdev){ 0 };
}

const struct file_operations *ferrion_chrdev_fops(unsigned int major)
{
	unsigned int i;

	for (i = 0; i < FERRION_CHRDEVS_MAX; i++)
		if (chrdevs[i].major == major)
			return chrdevs[i].fops;
	return NULL;
}

loff_t noop_llseek(struct file *file, loff_t offset, int whence)
{
	return file->f_pos;
}

struct fwnode_handle *__dev_fwnode(struct device *dev)
{
	return dev->fwnode;
}

bool device_property_present(const struct device *dev, const char *propname)
{
	return false;
}

bool fwnode_device_is_available(const struct fwnode_handle *fwnode)
{
	return false;
}

struct fwnode_handle *fwnode_find_reference(const struct fwnode_handle *fwnode, const char *name,
					    unsigned int index)
{
	return ERR_PTR(-ENOENT);
}

const struct dmi_system_id *dmi_first_match(const struct dmi_system_id *list)
{
	return NULL;
}

const char *dmi_get_system_info(int field)
{
	return NULL;
}

bool video_firmware_drivers_only(void)
{
	return false;
}

const char *video_get_options(const char *name)
{
	return NULL;
}

bool video_is_primary_device(struct device *dev)
{
	return false;
}

int vga_get(struct pci_dev *pdev, unsigned int rsrc, int interruptible)
{
	return 0;
}

void vga_put(struct pci_dev *pdev, unsigned int rsrc)
{
}

static int param_set_bool_value(const char *val, const struct kernel_param *kp)
{
	bool *arg = kp->arg;

	if (!val || !*val || *val == '1' || *val == 'y' || *val == 'Y') {
		*arg = true;
		return 0;
	}
	if (*val == '0' || *val == 'n' || *val == 'N') {
		*arg = false;
		return 0;
	}
	return -EINVAL;
}

static int param_get_bool_value(char *buffer, const struct kernel_param *kp)
{
	return sysfs_emit(buffer, "%c\n", *(bool *)kp->arg ? 'Y' : 'N');
}

static int param_set_int_value(const char *val, const struct kernel_param *kp)
{
	char *end;
	long v;

	if (!val)
		return -EINVAL;
	v = simple_strtol(val, &end, 0);
	if (end == val || v < INT_MIN || v > INT_MAX)
		return -EINVAL;
	*(int *)kp->arg = v;
	return 0;
}

static int param_get_int_value(char *buffer, const struct kernel_param *kp)
{
	return sysfs_emit(buffer, "%d\n", *(int *)kp->arg);
}

static int param_set_ulong_value(const char *val, const struct kernel_param *kp)
{
	char *end;
	long v;

	if (!val || *val == '-')
		return -EINVAL;
	v = simple_strtol(val, &end, 0);
	if (end == val)
		return -EINVAL;
	*(unsigned long *)kp->arg = v;
	return 0;
}

static int param_get_ulong_value(char *buffer, const struct kernel_param *kp)
{
	return sysfs_emit(buffer, "%lu\n", *(unsigned long *)kp->arg);
}

const struct kernel_param_ops param_ops_bool = {
	.flags = KERNEL_PARAM_OPS_FL_NOARG,
	.set = param_set_bool_value,
	.get = param_get_bool_value,
};

const struct kernel_param_ops param_ops_int = {
	.set = param_set_int_value,
	.get = param_get_int_value,
};

const struct kernel_param_ops param_ops_ulong = {
	.set = param_set_ulong_value,
	.get = param_get_ulong_value,
};

int kobject_uevent_env(struct kobject *kobj, enum kobject_action action, char *envp_ext[])
{
	ferrion_uevent(kobject_name(kobj), action);
	return 0;
}

int add_uevent_var(struct kobj_uevent_env *env, const char *format, ...)
{
	va_list args;
	int len;

	if (env->envp_idx >= ARRAY_SIZE(env->envp))
		return -ENOMEM;
	va_start(args, format);
	len = ferrion_vformat(&env->buf[env->buflen], sizeof(env->buf) - env->buflen, format, args);
	va_end(args);
	if (len >= (int)sizeof(env->buf) - env->buflen)
		return -ENOMEM;
	env->envp[env->envp_idx++] = &env->buf[env->buflen];
	env->buflen += len + 1;
	return 0;
}

int sysfs_emit_at(char *buf, int at, const char *fmt, ...)
{
	va_list args;
	int len;

	if (at < 0 || at >= PAGE_SIZE)
		return 0;
	va_start(args, fmt);
	len = ferrion_vformat(buf + at, PAGE_SIZE - at, fmt, args);
	va_end(args);
	return min_t(int, len, PAGE_SIZE - at - 1);
}
