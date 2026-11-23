// SPDX-License-Identifier: GPL-2.0-only
#include <linux/fs.h>
#include <linux/iosys-map.h>
#include <linux/kdev_t.h>
#include <linux/slab.h>
#include <drm/drm_connector.h>
#include <drm/drm_device.h>
#include <drm/drm_file.h>
#include <drm/drm_gem.h>
#include <drm/drm_ioctl.h>
#include <drm/drm_mode_config.h>
#include <drm/drm_property.h>
#include <ferrion/linux.h>

#define FERRION_DRM_PRIMARY_MINOR 0

void *ferrion_drm_open(void)
{
	struct inode *inode;
	struct file *file;
	int ret;

	ferrion_enter();
	inode = alloc_anon_inode(NULL);
	file = kzalloc(sizeof(*file), GFP_KERNEL);
	if (IS_ERR(inode) || !file) {
		if (!IS_ERR(inode))
			iput(inode);
		kfree(file);
		ferrion_leave();
		return NULL;
	}
	inode->i_mode = S_IFCHR | S_IRUSR | S_IWUSR;
	inode->i_rdev = MKDEV(DRM_MAJOR, FERRION_DRM_PRIMARY_MINOR);
	file_ref_init(&file->f_ref, 1);
	file->f_inode = inode;
	file->f_mode = FMODE_READ | FMODE_WRITE;
	file->f_flags = O_RDWR;
	file->f_op = ferrion_chrdev_fops(DRM_MAJOR);
	ret = file->f_op && file->f_op->open ? file->f_op->open(inode, file) : -ENODEV;
	if (ret) {
		pr_err("ferrion: opening the DRM device failed: %d\n", ret);
		iput(inode);
		kfree(file);
		file = NULL;
	}
	ferrion_leave();
	return file;
}

int ferrion_drm_ioctl(void *file, unsigned int cmd, void *arg)
{
	long ret;

	ferrion_enter();
	ferrion_user_window(NULL, ULONG_MAX);
	ret = drm_ioctl(file, cmd, (unsigned long)arg);
	ferrion_user_window(NULL, 0);
	ferrion_leave();
	return ret;
}

int ferrion_drm_vmap(void *file, unsigned int handle, void **va, unsigned long *size)
{
	struct file *f = file;
	struct drm_gem_object *obj;
	struct iosys_map map;
	int ret;

	ferrion_enter();
	obj = drm_gem_object_lookup(f->private_data, handle);
	if (!obj) {
		ferrion_leave();
		return -ENOENT;
	}
	ret = drm_gem_vmap(obj, &map);
	if (!ret) {
		if (map.is_iomem) {
			drm_gem_vunmap(obj, &map);
			ret = -EINVAL;
		} else {
			*va = map.vaddr;
			*size = obj->size;
		}
	}
	drm_gem_object_put(obj);
	ferrion_leave();
	return ret;
}

void ferrion_drm_vunmap(void *file, unsigned int handle, void *va)
{
	struct file *f = file;
	struct drm_gem_object *obj;
	struct iosys_map map = IOSYS_MAP_INIT_VADDR(va);

	ferrion_enter();
	obj = drm_gem_object_lookup(f->private_data, handle);
	if (obj) {
		drm_gem_vunmap(obj, &map);
		drm_gem_object_put(obj);
	}
	ferrion_leave();
}

long ferrion_drm_edid(void *file, unsigned int connector_id, void *buf, unsigned long len)
{
	struct drm_file *priv = ((struct file *)file)->private_data;
	struct drm_device *dev = priv->minor->dev;
	struct drm_connector *connector;
	long ret = 0;

	ferrion_enter();
	connector = drm_connector_lookup(dev, priv, connector_id);
	if (!connector) {
		ferrion_leave();
		return -ENOENT;
	}
	mutex_lock(&dev->mode_config.mutex);
	if (connector->edid_blob_ptr) {
		ret = connector->edid_blob_ptr->length;
		memcpy(buf, connector->edid_blob_ptr->data, min_t(unsigned long, ret, len));
	}
	mutex_unlock(&dev->mode_config.mutex);
	drm_connector_put(connector);
	ferrion_leave();
	return ret;
}
