// SPDX-License-Identifier: GPL-2.0-only
#include <linux/ctype.h>
#include <linux/device.h>
#include <linux/kernel.h>
#include <linux/panic.h>
#include <linux/printk.h>
#include <linux/ratelimit.h>
#include <linux/sprintf.h>
#include <linux/sysfs.h>
#include <ferrion/linux.h>

#define FERRION_LINE_MAX 1024
#define FERRION_HEX_MAX 64
#define FERRION_VA_DEPTH_MAX 4

int oops_in_progress;

struct out {
	char *buf;
	size_t size;
	size_t len;
};

struct spec {
	bool left;
	bool plus;
	bool space;
	bool alt;
	bool zero;
	int width;
	int precision;
};

static void put(struct out *o, char c)
{
	if (o->len + 1 < o->size)
		o->buf[o->len] = c;
	o->len++;
}

static void pad(struct out *o, char c, int n)
{
	while (n-- > 0)
		put(o, c);
}

static void put_str(struct out *o, const char *s, const struct spec *sp)
{
	int len = 0;

	if (!s)
		s = "(null)";
	while ((sp->precision < 0 || len < sp->precision) && s[len])
		len++;
	if (!sp->left)
		pad(o, ' ', sp->width - len);
	for (int i = 0; i < len; i++)
		put(o, s[i]);
	if (sp->left)
		pad(o, ' ', sp->width - len);
}

static void put_num(struct out *o, unsigned long long v, bool negative, unsigned int base, bool upper,
		    const struct spec *sp)
{
	const char *digits = upper ? "0123456789ABCDEF" : "0123456789abcdef";
	char tmp[24];
	int n = 0, sign = 0, prefix = 0, zeros, total;

	do {
		tmp[n++] = digits[v % base];
		v /= base;
	} while (v);
	if (sp->precision >= 0 && sp->precision == 0 && n == 1 && tmp[0] == '0')
		n = 0;
	if (negative || sp->plus || sp->space)
		sign = 1;
	if (sp->alt && base == 16)
		prefix = 2;
	else if (sp->alt && base == 8)
		prefix = 1;
	zeros = sp->precision > n ? sp->precision - n : 0;
	total = sign + prefix + zeros + n;
	if (sp->zero && !sp->left && sp->precision < 0 && sp->width > total) {
		zeros += sp->width - total;
		total = sp->width;
	}
	if (!sp->left)
		pad(o, ' ', sp->width - total);
	if (sign)
		put(o, negative ? '-' : sp->plus ? '+' : ' ');
	if (prefix) {
		put(o, '0');
		if (prefix == 2)
			put(o, upper ? 'X' : 'x');
	}
	pad(o, '0', zeros);
	while (n)
		put(o, tmp[--n]);
	if (sp->left)
		pad(o, ' ', sp->width - total);
}

static void format(struct out *o, const char *fmt, va_list *ap, int depth);

static void put_hex(struct out *o, const u8 *p, int len, char sep)
{
	struct spec sp = { .precision = -1, .width = 2, .zero = true };

	if (len < 0 || len > FERRION_HEX_MAX)
		len = FERRION_HEX_MAX;
	for (int i = 0; i < len; i++) {
		if (i && sep)
			put(o, sep);
		put_num(o, p[i], false, 16, false, &sp);
	}
}

static const char *put_pointer(struct out *o, const char *fmt, void *ptr, struct spec *sp, va_list *ap, int depth)
{
	struct spec hex = { .precision = -1, .alt = true };

	switch (*fmt) {
	case 'V':
		if (ptr && depth < FERRION_VA_DEPTH_MAX) {
			struct va_format *vaf = ptr;
			va_list copy;

			va_copy(copy, *vaf->va);
			format(o, vaf->fmt, &copy, depth + 1);
			va_end(copy);
		}
		return fmt + 1;
	case 'e':
		if (IS_ERR(ptr)) {
			struct spec dec = { .precision = -1 };

			put_num(o, -PTR_ERR(ptr), true, 10, false, &dec);
			return fmt + 1;
		}
		break;
	case '4':
		if (fmt[1] == 'c' && fmt[2] == 'c' && ptr) {
			u32 cc = *(u32 *)ptr;

			for (int i = 0; i < 4; i++) {
				char c = (cc >> (8 * i)) & 0x7f;

				put(o, isascii(c) && isprint(c) ? c : '.');
			}
			put_str(o, cc & BIT(31) ? " big-endian (0x" : " little-endian (0x",
				&(struct spec){ .precision = -1 });
			put_num(o, cc, false, 16, false, &(struct spec){ .precision = 8, .width = 0 });
			put(o, ')');
			return fmt + 3;
		}
		break;
	case 'h':
		if (ptr) {
			char sep = fmt[1] == 'C' ? ':' : fmt[1] == 'D' ? '-' : fmt[1] == 'N' ? 0 : ' ';

			put_hex(o, ptr, sp->width < 0 ? 1 : sp->width, sep);
		}
		while (isalnum(*fmt))
			fmt++;
		return fmt;
	case 'a':
		if (ptr) {
			put_num(o, fmt[1] == 'd' ? *(dma_addr_t *)ptr : *(phys_addr_t *)ptr, false, 16, false, &hex);
			while (isalnum(*fmt))
				fmt++;
			return fmt;
		}
		break;
	default:
		break;
	}
	put_num(o, (unsigned long)ptr, false, 16, false, &hex);
	while (isalnum(*fmt))
		fmt++;
	return fmt;
}

static void format(struct out *o, const char *fmt, va_list *ap, int depth)
{
	while (*fmt) {
		struct spec sp = { .precision = -1 };
		int length = 0;
		unsigned long long v;
		bool negative;

		if (*fmt != '%') {
			put(o, *fmt++);
			continue;
		}
		fmt++;
		for (;; fmt++) {
			if (*fmt == '-')
				sp.left = true;
			else if (*fmt == '+')
				sp.plus = true;
			else if (*fmt == ' ')
				sp.space = true;
			else if (*fmt == '#')
				sp.alt = true;
			else if (*fmt == '0')
				sp.zero = true;
			else
				break;
		}
		if (*fmt == '*') {
			sp.width = va_arg(*ap, int);
			if (sp.width < 0) {
				sp.left = true;
				sp.width = -sp.width;
			}
			fmt++;
		} else {
			sp.width = -1;
			while (isdigit(*fmt))
				sp.width = (sp.width < 0 ? 0 : sp.width * 10) + (*fmt++ - '0');
		}
		if (*fmt == '.') {
			fmt++;
			sp.precision = 0;
			if (*fmt == '*') {
				sp.precision = va_arg(*ap, int);
				fmt++;
			} else {
				while (isdigit(*fmt))
					sp.precision = sp.precision * 10 + (*fmt++ - '0');
			}
		}
		while (*fmt == 'h' || *fmt == 'l' || *fmt == 'z' || *fmt == 't' || *fmt == 'j' || *fmt == 'L' ||
		       *fmt == 'q') {
			length += *fmt == 'h' ? -1 : 1;
			if (*fmt == 'z' || *fmt == 't' || *fmt == 'j' || *fmt == 'q' || *fmt == 'L')
				length = 2;
			fmt++;
		}
		switch (*fmt) {
		case 'd':
		case 'i':
			if (length >= 2)
				v = va_arg(*ap, long long);
			else if (length == 1)
				v = va_arg(*ap, long);
			else
				v = va_arg(*ap, int);
			if (length == -1)
				v = (short)v;
			else if (length <= -2)
				v = (signed char)v;
			negative = (long long)v < 0;
			put_num(o, negative ? -(long long)v : v, negative, 10, false, &sp);
			break;
		case 'u':
		case 'x':
		case 'X':
		case 'o':
			if (length >= 2)
				v = va_arg(*ap, unsigned long long);
			else if (length == 1)
				v = va_arg(*ap, unsigned long);
			else
				v = va_arg(*ap, unsigned int);
			if (length == -1)
				v = (unsigned short)v;
			else if (length <= -2)
				v = (unsigned char)v;
			put_num(o, v, false, *fmt == 'u' ? 10 : *fmt == 'o' ? 8 : 16, *fmt == 'X', &sp);
			break;
		case 'c': {
			char c[2] = { (char)va_arg(*ap, int), 0 };

			put_str(o, c, &sp);
			break;
		}
		case 's':
			put_str(o, va_arg(*ap, const char *), &sp);
			break;
		case 'p':
			fmt = put_pointer(o, fmt + 1, va_arg(*ap, void *), &sp, ap, depth);
			continue;
		case '%':
			put(o, '%');
			break;
		case '\0':
			return;
		default:
			put(o, '%');
			put(o, *fmt);
			break;
		}
		fmt++;
	}
}

int ferrion_vformat(char *buf, size_t size, const char *fmt, va_list args)
{
	struct out o = { .buf = buf, .size = size };
	va_list copy;

	va_copy(copy, args);
	format(&o, fmt, &copy, 0);
	va_end(copy);
	if (size)
		buf[min(o.len, size - 1)] = '\0';
	return o.len;
}

static int level_of(const char **fmt, int fallback)
{
	int level = fallback;

	while ((*fmt)[0] == KERN_SOH_ASCII && (*fmt)[1]) {
		char c = (*fmt)[1];

		if (c >= '0' && c <= '7')
			level = c - '0';
		*fmt += 2;
	}
	return level;
}

void ferrion_vlog(int level, const char *prefix, const char *fmt, va_list args)
{
	char line[FERRION_LINE_MAX];
	struct out o = { .buf = line, .size = sizeof(line) };
	va_list copy;

	level = level_of(&fmt, level);
	if (prefix)
		while (*prefix)
			put(&o, *prefix++);
	va_copy(copy, args);
	format(&o, fmt, &copy, 0);
	va_end(copy);
	ferrion_log(level, line, min(o.len, sizeof(line) - 1));
}

int _printk(const char *fmt, ...)
{
	va_list args;

	va_start(args, fmt);
	ferrion_vlog(MESSAGE_LOGLEVEL_DEFAULT, NULL, fmt, args);
	va_end(args);
	return 0;
}

static void dev_vlog(int level, const struct device *dev, const char *fmt, va_list args)
{
	char prefix[FERRION_LINE_MAX / 4];

	scnprintf(prefix, sizeof(prefix), "%s %s: ", dev ? dev_driver_string(dev) : "(NULL device)",
		  dev ? dev_name(dev) : "");
	ferrion_vlog(level, prefix, fmt, args);
}

void _dev_printk(const char *level, const struct device *dev, const char *fmt, ...)
{
	va_list args;
	const char *l = level;

	va_start(args, fmt);
	dev_vlog(level_of(&l, MESSAGE_LOGLEVEL_DEFAULT), dev, fmt, args);
	va_end(args);
}

void _dev_err(const struct device *dev, const char *fmt, ...)
{
	va_list args;

	va_start(args, fmt);
	dev_vlog(LOGLEVEL_ERR, dev, fmt, args);
	va_end(args);
}

void _dev_warn(const struct device *dev, const char *fmt, ...)
{
	va_list args;

	va_start(args, fmt);
	dev_vlog(LOGLEVEL_WARNING, dev, fmt, args);
	va_end(args);
}

void _dev_info(const struct device *dev, const char *fmt, ...)
{
	va_list args;

	va_start(args, fmt);
	dev_vlog(LOGLEVEL_INFO, dev, fmt, args);
	va_end(args);
}

int dev_err_probe(const struct device *dev, int err, const char *fmt, ...)
{
	va_list args;

	if (err == -EPROBE_DEFER)
		return err;
	va_start(args, fmt);
	dev_vlog(LOGLEVEL_ERR, dev, fmt, args);
	va_end(args);
	return err;
}

void warn_slowpath_fmt(const char *file, const int line, unsigned taint, const char *fmt, ...)
{
	va_list args;

	pr_warn("WARNING: at %s:%d\n", file, line);
	if (!fmt)
		return;
	va_start(args, fmt);
	ferrion_vlog(LOGLEVEL_WARNING, NULL, fmt, args);
	va_end(args);
}

void panic(const char *fmt, ...)
{
	va_list args;

	oops_in_progress = 1;
	va_start(args, fmt);
	ferrion_vlog(LOGLEVEL_EMERG, "panic: ", fmt, args);
	va_end(args);
	ferrion_abort("panic");
}

int ___ratelimit(struct ratelimit_state *rs, const char *func)
{
	if (rs->interval <= 0 || rs->burst <= 0)
		return rs->burst > 0 || rs->interval <= 0;
	if (!rs->begin || time_is_before_jiffies(rs->begin + rs->interval)) {
		int missed = atomic_xchg(&rs->missed, 0);

		if (missed)
			pr_warn("%s: %d callbacks suppressed\n", func, missed);
		rs->begin = jiffies;
		atomic_set(&rs->rs_n_left, rs->burst);
	}
	if (atomic_dec_if_positive(&rs->rs_n_left) >= 0)
		return 1;
	atomic_inc(&rs->missed);
	return 0;
}

int scnprintf(char *buf, size_t size, const char *fmt, ...)
{
	va_list args;
	int len;

	if (!size)
		return 0;
	va_start(args, fmt);
	len = ferrion_vformat(buf, size, fmt, args);
	va_end(args);
	return min_t(int, len, size - 1);
}

int sysfs_emit(char *buf, const char *fmt, ...)
{
	va_list args;
	int len;

	va_start(args, fmt);
	len = ferrion_vformat(buf, PAGE_SIZE, fmt, args);
	va_end(args);
	return min_t(int, len, PAGE_SIZE - 1);
}
