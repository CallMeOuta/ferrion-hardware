/* SPDX-License-Identifier: GPL-2.0-only */
#ifndef _X86_IRQFLAGS_H_
#define _X86_IRQFLAGS_H_

#include <asm/processor-flags.h>

#ifndef __ASSEMBLER__

#include <asm/nospec-branch.h>
#include <linux/types.h>

extern unsigned long ferrion_irq_flags;

static __always_inline unsigned long native_save_fl(void)
{
	return READ_ONCE(ferrion_irq_flags);
}

static __always_inline void native_irq_disable(void)
{
	WRITE_ONCE(ferrion_irq_flags, 0);
}

static __always_inline void native_irq_enable(void)
{
	WRITE_ONCE(ferrion_irq_flags, X86_EFLAGS_IF);
}

static __always_inline int native_irqs_disabled_flags(unsigned long flags)
{
	return !(flags & X86_EFLAGS_IF);
}

static __always_inline unsigned long native_local_irq_save(void)
{
	unsigned long flags = native_save_fl();

	native_irq_disable();
	return flags;
}

static __always_inline void native_local_irq_restore(unsigned long flags)
{
	if (!native_irqs_disabled_flags(flags))
		native_irq_enable();
}

static __always_inline unsigned long arch_local_save_flags(void)
{
	return native_save_fl();
}

static __always_inline void arch_local_irq_disable(void)
{
	native_irq_disable();
}

static __always_inline void arch_local_irq_enable(void)
{
	native_irq_enable();
}

static __always_inline unsigned long arch_local_irq_save(void)
{
	return native_local_irq_save();
}

static __always_inline int arch_irqs_disabled_flags(unsigned long flags)
{
	return native_irqs_disabled_flags(flags);
}

static __always_inline int arch_irqs_disabled(void)
{
	return arch_irqs_disabled_flags(arch_local_save_flags());
}

static __always_inline void arch_local_irq_restore(unsigned long flags)
{
	native_local_irq_restore(flags);
}

#endif

#endif
