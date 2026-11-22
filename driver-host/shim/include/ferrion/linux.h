/* SPDX-License-Identifier: GPL-2.0-only */
#ifndef FERRION_LINUX_H
#define FERRION_LINUX_H

#include <linux/sched.h>
#include <linux/stdarg.h>
#include <linux/workqueue.h>
#include <ferrion/host.h>

#define FERRION_TASK_NAME_MAX 16

struct ferrion_task {
	struct task_struct task;
	struct ferrion_thread *thread;
	int preempt_count;
	unsigned long irq_flags;
	const void *user_base;
	unsigned long user_len;
	struct work_struct *current_work;
	void (*entry)(void *arg);
	void *arg;
};

struct ferrion_task *ferrion_current(void);
struct ferrion_task *ferrion_task_new(const char *name, void (*entry)(void *arg), void *arg);
int ferrion_task_start(struct ferrion_task *t);
void ferrion_task_sleep(struct ferrion_task *t, unsigned long long deadline_ns);
void ferrion_enter(void);
void ferrion_leave(void);
void ferrion_exit_task(void);

void ferrion_tick(void);
unsigned long long ferrion_jiffies_to_deadline(unsigned long expires);
void ferrion_time_init(void);
void ferrion_workqueue_init(void);
void ferrion_rcu_drain(void);

int ferrion_vformat(char *buf, size_t size, const char *fmt, va_list args);
void ferrion_vlog(int level, const char *prefix, const char *fmt, va_list args);

void ferrion_user_window(const void *base, unsigned long len);

#endif
