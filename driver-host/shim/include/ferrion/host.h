/* SPDX-License-Identifier: GPL-2.0-only */
#ifndef FERRION_HOST_H
#define FERRION_HOST_H

struct ferrion_thread;

struct ferrion_pci_bar {
	unsigned long long phys;
	unsigned long long size;
	unsigned int flags;
};

#define FERRION_PCI_BAR_IO 1U
#define FERRION_PCI_BAR_64 2U
#define FERRION_PCI_BAR_PREFETCH 4U

struct ferrion_pci_info {
	unsigned short vendor;
	unsigned short device;
	unsigned short subsystem_vendor;
	unsigned short subsystem_device;
	unsigned int class;
	unsigned char revision;
	unsigned char bus;
	unsigned char devfn;
	unsigned char has_irq;
	struct ferrion_pci_bar bars[6];
};

void ferrion_log(int level, const char *text, unsigned long len);
__attribute__((noreturn)) void ferrion_refuse(const char *name);
__attribute__((noreturn)) void ferrion_abort(const char *what);

void *ferrion_alloc(unsigned long size, unsigned long align);
void ferrion_free(void *ptr);
unsigned long ferrion_alloc_usable(const void *ptr);

void *ferrion_phys_alloc(unsigned long size, unsigned long *phys);
void ferrion_phys_free(void *va, unsigned long size);
int ferrion_anon_map(void *addr, unsigned long size);

int ferrion_pci_info(struct ferrion_pci_info *info);
unsigned int ferrion_pci_read_config(unsigned int offset);
void ferrion_pci_write_config(unsigned int offset, unsigned int value);
void *ferrion_pci_map_bar(unsigned int bar);
int ferrion_pci_irq_start(unsigned int irq);

void ferrion_uevent(const char *name, int action);

unsigned long long ferrion_now_ns(void);
unsigned int ferrion_random_u32(void);

void ferrion_lock(void);
void ferrion_unlock(void);
int ferrion_lock_waiters(void);
void ferrion_yield(void);

struct ferrion_thread *ferrion_thread_self(void);
void *ferrion_thread_context(struct ferrion_thread *thread);
void ferrion_thread_set_context(struct ferrion_thread *thread, void *context);
int ferrion_thread_wait(struct ferrion_thread *thread, unsigned long long deadline_ns);
void ferrion_thread_wake(struct ferrion_thread *thread);
int ferrion_thread_spawn(void (*entry)(void *), void *arg);
int ferrion_tick_start(void (*tick)(void), unsigned long long period_ns);

__attribute__((noreturn)) void ferrion_driver_host_main(void);
int ferrion_linux_start(void);
int ferrion_pci_attach(void);
void ferrion_irq_fire(unsigned int irq);

void *ferrion_drm_open(void);
int ferrion_drm_ioctl(void *file, unsigned int cmd, void *arg);
int ferrion_drm_vmap(void *file, unsigned int handle, void **va, unsigned long *size);
void ferrion_drm_vunmap(void *file, unsigned int handle, void *va);
long ferrion_drm_edid(void *file, unsigned int connector_id, void *buf, unsigned long len);

#endif
