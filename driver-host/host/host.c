/* SPDX-License-Identifier: GPL-2.0-only */
#define _GNU_SOURCE
#include <errno.h>
#include <malloc.h>
#include <pthread.h>
#include <sched.h>
#include <stdatomic.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/random.h>
#include <time.h>
#include <unistd.h>

#include <ferrion/host.h>

#define FERRION_ALLOC_BUDGET (512UL << 20)
#define FERRION_THREADS_MAX 64
#define FERRION_LOG_LINE_MAX 1024
#define FERRION_REFUSED_EXIT 70

struct ferrion_thread {
	pthread_cond_t cond;
	int woken;
	void *context;
	void (*entry)(void *);
	void *arg;
};

static pthread_mutex_t big_lock = PTHREAD_MUTEX_INITIALIZER;
static atomic_int lock_waiters;
static atomic_ulong allocated;
static atomic_int threads;
static __thread struct ferrion_thread *self;

void ferrion_refuse(const char *name)
{
	char line[FERRION_LOG_LINE_MAX];
	int len = snprintf(line, sizeof(line), "refused: %s is not provided by the shim", name);

	ferrion_log(2, line, (unsigned long)len);
	_exit(FERRION_REFUSED_EXIT);
}

void ferrion_abort(const char *what)
{
	char line[FERRION_LOG_LINE_MAX];
	int len = snprintf(line, sizeof(line), "abort: %s", what);

	ferrion_log(0, line, (unsigned long)len);
	abort();
}

void *ferrion_alloc(unsigned long size, unsigned long align)
{
	void *ptr = NULL;

	if (align < sizeof(void *))
		align = sizeof(void *);
	if (atomic_load(&allocated) + size > FERRION_ALLOC_BUDGET)
		return NULL;
	if (posix_memalign(&ptr, align, size ? size : 1) != 0)
		return NULL;
	atomic_fetch_add(&allocated, malloc_usable_size(ptr));
	return ptr;
}

void ferrion_free(void *ptr)
{
	if (!ptr)
		return;
	atomic_fetch_sub(&allocated, malloc_usable_size(ptr));
	free(ptr);
}

unsigned long ferrion_alloc_usable(const void *ptr)
{
	return malloc_usable_size((void *)ptr);
}

unsigned long long ferrion_now_ns(void)
{
	struct timespec ts;

	if (clock_gettime(CLOCK_MONOTONIC, &ts) != 0)
		ferrion_abort("clock_gettime(CLOCK_MONOTONIC)");
	return (unsigned long long)ts.tv_sec * 1000000000ULL + (unsigned long long)ts.tv_nsec;
}

unsigned int ferrion_random_u32(void)
{
	unsigned int value;

	if (getrandom(&value, sizeof(value), 0) != (ssize_t)sizeof(value))
		ferrion_abort("getrandom");
	return value;
}

void ferrion_lock(void)
{
	atomic_fetch_add(&lock_waiters, 1);
	if (pthread_mutex_lock(&big_lock) != 0)
		ferrion_abort("pthread_mutex_lock");
	atomic_fetch_sub(&lock_waiters, 1);
}

void ferrion_unlock(void)
{
	if (pthread_mutex_unlock(&big_lock) != 0)
		ferrion_abort("pthread_mutex_unlock");
}

int ferrion_lock_waiters(void)
{
	return atomic_load(&lock_waiters);
}

void ferrion_yield(void)
{
	ferrion_unlock();
	sched_yield();
	ferrion_lock();
}

static struct ferrion_thread *thread_new(void (*entry)(void *), void *arg)
{
	struct ferrion_thread *thread = calloc(1, sizeof(*thread));
	pthread_condattr_t attr;

	if (!thread)
		ferrion_abort("thread record allocation");
	if (pthread_condattr_init(&attr) != 0 || pthread_condattr_setclock(&attr, CLOCK_MONOTONIC) != 0 ||
	    pthread_cond_init(&thread->cond, &attr) != 0)
		ferrion_abort("pthread_cond_init");
	pthread_condattr_destroy(&attr);
	thread->entry = entry;
	thread->arg = arg;
	return thread;
}

struct ferrion_thread *ferrion_thread_self(void)
{
	if (!self)
		self = thread_new(NULL, NULL);
	return self;
}

void *ferrion_thread_context(struct ferrion_thread *thread)
{
	return thread->context;
}

void ferrion_thread_set_context(struct ferrion_thread *thread, void *context)
{
	thread->context = context;
}

int ferrion_thread_wait(struct ferrion_thread *thread, unsigned long long deadline_ns)
{
	int timed_out = 0;

	while (!thread->woken && !timed_out) {
		if (deadline_ns == 0) {
			if (pthread_cond_wait(&thread->cond, &big_lock) != 0)
				ferrion_abort("pthread_cond_wait");
		} else {
			struct timespec ts = {
				.tv_sec = (time_t)(deadline_ns / 1000000000ULL),
				.tv_nsec = (long)(deadline_ns % 1000000000ULL),
			};
			int err = pthread_cond_timedwait(&thread->cond, &big_lock, &ts);

			if (err == ETIMEDOUT)
				timed_out = 1;
			else if (err != 0)
				ferrion_abort("pthread_cond_timedwait");
		}
	}
	timed_out = !thread->woken;
	thread->woken = 0;
	return timed_out;
}

void ferrion_thread_wake(struct ferrion_thread *thread)
{
	thread->woken = 1;
	if (pthread_cond_signal(&thread->cond) != 0)
		ferrion_abort("pthread_cond_signal");
}

static void *thread_start(void *arg)
{
	struct ferrion_thread *thread = arg;

	self = thread;
	thread->entry(thread->arg);
	pthread_cond_destroy(&thread->cond);
	free(thread);
	atomic_fetch_sub(&threads, 1);
	return NULL;
}

int ferrion_thread_spawn(void (*entry)(void *), void *arg)
{
	struct ferrion_thread *thread;
	pthread_attr_t attr;
	pthread_t id;
	int err;

	if (atomic_fetch_add(&threads, 1) >= FERRION_THREADS_MAX) {
		atomic_fetch_sub(&threads, 1);
		return -EAGAIN;
	}
	thread = thread_new(entry, arg);
	pthread_attr_init(&attr);
	pthread_attr_setdetachstate(&attr, PTHREAD_CREATE_DETACHED);
	err = pthread_create(&id, &attr, thread_start, thread);
	pthread_attr_destroy(&attr);
	if (err != 0) {
		pthread_cond_destroy(&thread->cond);
		free(thread);
		atomic_fetch_sub(&threads, 1);
		return -err;
	}
	return 0;
}

struct tick {
	void (*fn)(void);
	struct timespec period;
};

static void *tick_main(void *arg)
{
	struct tick *tick = arg;

	for (;;) {
		nanosleep(&tick->period, NULL);
		tick->fn();
	}
	return NULL;
}

int ferrion_tick_start(void (*fn)(void), unsigned long long period_ns)
{
	static struct tick tick;
	pthread_attr_t attr;
	pthread_t id;
	int err;

	if (tick.fn)
		return -EBUSY;
	tick.fn = fn;
	tick.period.tv_sec = (time_t)(period_ns / 1000000000ULL);
	tick.period.tv_nsec = (long)(period_ns % 1000000000ULL);
	pthread_attr_init(&attr);
	pthread_attr_setdetachstate(&attr, PTHREAD_CREATE_DETACHED);
	err = pthread_create(&id, &attr, tick_main, &tick);
	pthread_attr_destroy(&attr);
	return -err;
}

int main(void)
{
	ferrion_driver_host_main();
}
