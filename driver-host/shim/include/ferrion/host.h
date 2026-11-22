/* SPDX-License-Identifier: GPL-2.0-only */
#ifndef FERRION_HOST_H
#define FERRION_HOST_H

struct ferrion_thread;

void ferrion_log(int level, const char *text, unsigned long len);
__attribute__((noreturn)) void ferrion_refuse(const char *name);
__attribute__((noreturn)) void ferrion_abort(const char *what);

void *ferrion_alloc(unsigned long size, unsigned long align);
void ferrion_free(void *ptr);
unsigned long ferrion_alloc_usable(const void *ptr);

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

int ferrion_linux_main(void);

#endif
