// SPDX-License-Identifier: GPL-2.0-only
#include <linux/init.h>
#include <linux/kernel.h>
#include <linux/kobject_ns.h>
#include <linux/moduleparam.h>
#include <linux/sysfs.h>
#include <linux/utsname.h>
#include <net/net_namespace.h>
#include <ferrion/linux.h>

#define FERRION_PARAMETER_BYTES_MAX 1024

struct net init_net = {
	.passive = REFCOUNT_INIT(1),
	.ns.ns_type = CLONE_NEWNET,
	.ns.__ns_ref = REFCOUNT_INIT(1),
	.dev_base_head = LIST_HEAD_INIT(init_net.dev_base_head),
	.ptype_all = LIST_HEAD_INIT(init_net.ptype_all),
	.ptype_specific = LIST_HEAD_INIT(init_net.ptype_specific),
	.rules_ops = LIST_HEAD_INIT(init_net.rules_ops),
	.netns_ids = IDR_INIT(init_net.netns_ids),
	.dev_by_index = XARRAY_INIT(init_net.dev_by_index, 0),
};
LIST_HEAD(net_namespace_list);

struct uts_namespace init_uts_ns = {
	.ns.ns_type = CLONE_NEWUTS,
	.ns.__ns_ref = REFCOUNT_INIT(1),
	.name.sysname = "FerrionOS",
	.name.release = "driver-host",
	.name.machine = "x86_64",
};

__attribute__((noreturn)) void ferrion_wifi_namespace_sysfs(void);
__attribute__((noreturn)) void ferrion_wifi_parameters(void);

static int wifi_namespace_init(void)
{
	list_add(&init_net.list, &net_namespace_list);
	INIT_LIST_HEAD(&init_net.exit_list);
	return 0;
}
early_initcall(wifi_namespace_init);

static bool namespace_may_mount(void)
{
	return false;
}

static void *namespace_grab(void)
{
	ferrion_wifi_namespace_sysfs();
}

static const void *namespace_netlink(struct sock *sock)
{
	ferrion_wifi_namespace_sysfs();
}

static const void *namespace_initial(void)
{
	return &init_net;
}

static void namespace_drop(void *namespace)
{
	ferrion_wifi_namespace_sysfs();
}

const struct kobj_ns_type_operations net_ns_type_operations = {
	.type = KOBJ_NS_TYPE_NET,
	.current_may_mount = namespace_may_mount,
	.grab_current_ns = namespace_grab,
	.netlink_ns = namespace_netlink,
	.initial_ns = namespace_initial,
	.drop_ns = namespace_drop,
};

static int parameter_set_string(const char *value, const struct kernel_param *parameter)
{
	ferrion_wifi_parameters();
}

static int parameter_get_string(char *buffer, const struct kernel_param *parameter)
{
	const char *value = *(const char **)parameter->arg;

	if (!value)
		return scnprintf(buffer, PAGE_SIZE, "\n");
	if (strnlen(value, FERRION_PARAMETER_BYTES_MAX) == FERRION_PARAMETER_BYTES_MAX)
		return -E2BIG;
	return scnprintf(buffer, PAGE_SIZE, "%s\n", value);
}

const struct kernel_param_ops param_ops_charp = {
	.set = parameter_set_string,
	.get = parameter_get_string,
};

static int parameter_set_uint(const char *value, const struct kernel_param *parameter)
{
	return kstrtouint(value, 0, parameter->arg);
}

static int parameter_get_uint(char *buffer, const struct kernel_param *parameter)
{
	return scnprintf(buffer, PAGE_SIZE, "%u\n", *(unsigned int *)parameter->arg);
}

const struct kernel_param_ops param_ops_uint = {
	.set = parameter_set_uint,
	.get = parameter_get_uint,
};
