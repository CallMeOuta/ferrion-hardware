/* SPDX-License-Identifier: GPL-2.0-only */
SECTIONS
{
	.ferrion_initcalls : ALIGN(8)
	{
		__ferrion_initcall_start = .;
		KEEP(*(.initcallearly.init))
		KEEP(*(.initcall0.init)) KEEP(*(.initcall0s.init))
		KEEP(*(.initcall1.init)) KEEP(*(.initcall1s.init))
		KEEP(*(.initcall2.init)) KEEP(*(.initcall2s.init))
		KEEP(*(.initcall3.init)) KEEP(*(.initcall3s.init))
		KEEP(*(.initcall4.init)) KEEP(*(.initcall4s.init))
		KEEP(*(.initcall5.init)) KEEP(*(.initcall5s.init))
		KEEP(*(.initcallrootfs.init))
		KEEP(*(.initcall6.init)) KEEP(*(.initcall6s.init))
		KEEP(*(.initcall7.init)) KEEP(*(.initcall7s.init))
		__ferrion_initcall_end = .;
	}
}
INSERT AFTER .rodata;
const_current_task = current_task;
