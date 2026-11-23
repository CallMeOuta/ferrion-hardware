// SPDX-License-Identifier: GPL-2.0-only
#include <linux/net.h>
#include <linux/netdevice.h>
#include <linux/ratelimit.h>
#include <ferrion/linux.h>

static DEFINE_RATELIMIT_STATE(wifi_log_limit, DEFAULT_RATELIMIT_INTERVAL, DEFAULT_RATELIMIT_BURST);

int net_ratelimit(void)
{
	return __ratelimit(&wifi_log_limit);
}

void netdev_err(const struct net_device *dev, const char *format, ...)
{
	va_list args;
	char prefix[IFNAMSIZ + 3];

	scnprintf(prefix, sizeof(prefix), "%s: ", dev ? dev->name : "wifi");
	va_start(args, format);
	ferrion_vlog(LOGLEVEL_ERR, prefix, format, args);
	va_end(args);
}

void netdev_info(const struct net_device *dev, const char *format, ...)
{
	va_list args;
	char prefix[IFNAMSIZ + 3];

	scnprintf(prefix, sizeof(prefix), "%s: ", dev ? dev->name : "wifi");
	va_start(args, format);
	ferrion_vlog(LOGLEVEL_INFO, prefix, format, args);
	va_end(args);
}
