// SPDX-License-Identifier: GPL-2.0-only
#include <linux/capability.h>
#include <linux/completion.h>
#include <linux/mutex.h>
#include <linux/pid.h>
#include <linux/rcupdate.h>
#include <linux/sched.h>
#include <linux/sched/signal.h>
#include <linux/slab.h>
#include <linux/srcu.h>
#include <linux/string.h>
#include <linux/swait.h>
#include <linux/wait.h>
#include <linux/ww_mutex.h>
#include <asm/fpu/api.h>
#include <ferrion/linux.h>

#define FERRION_RCU_DRAIN_MAX 4096

DEFINE_PER_CPU_CACHE_HOT(struct task_struct *, current_task);
DEFINE_PER_CPU_CACHE_HOT(int, __preempt_count) = INIT_PREEMPT_COUNT;
unsigned long ferrion_irq_flags = X86_EFLAGS_IF;

static struct signal_struct ferrion_signal;
static struct rcu_head *rcu_callbacks;
static struct rcu_head *rcu_frees;

struct ferrion_lock_waiter {
	struct list_head list;
	struct task_struct *task;
};

static struct ferrion_task *task_of(struct task_struct *p)
{
	return container_of(p, struct ferrion_task, task);
}

struct ferrion_task *ferrion_current(void)
{
	return task_of(current);
}

static void resume(struct ferrion_task *t)
{
	this_cpu_write(current_task, &t->task);
	raw_cpu_write_4(__preempt_count, t->preempt_count);
	WRITE_ONCE(ferrion_irq_flags, t->irq_flags);
	ferrion_tick();
}

static void suspend(struct ferrion_task *t)
{
	ferrion_rcu_drain();
	t->preempt_count = raw_cpu_read_4(__preempt_count);
	t->irq_flags = READ_ONCE(ferrion_irq_flags);
}

struct ferrion_task *ferrion_task_new(const char *name, void (*entry)(void *arg), void *arg)
{
	struct ferrion_task *t = kzalloc(sizeof(*t), GFP_KERNEL);

	if (!t)
		ferrion_abort("task allocation");
	t->task.__state = TASK_RUNNING;
	t->task.signal = &ferrion_signal;
	t->task.group_leader = &t->task;
	t->task.flags = entry ? PF_KTHREAD : 0;
	strscpy(t->task.comm, name, sizeof(t->task.comm));
	t->preempt_count = INIT_PREEMPT_COUNT & ~PREEMPT_NEED_RESCHED;
	t->irq_flags = X86_EFLAGS_IF;
	t->entry = entry;
	t->arg = arg;
	return t;
}

static void task_trampoline(void *arg)
{
	struct ferrion_task *t = arg;
	struct ferrion_thread *thread = ferrion_thread_self();

	ferrion_lock();
	t->thread = thread;
	ferrion_thread_set_context(thread, t);
	resume(t);
	t->entry(t->arg);
	ferrion_exit_task();
}

int ferrion_task_start(struct ferrion_task *t)
{
	return ferrion_thread_spawn(task_trampoline, t);
}

void ferrion_enter(void)
{
	struct ferrion_thread *thread = ferrion_thread_self();
	struct ferrion_task *t;

	ferrion_lock();
	t = ferrion_thread_context(thread);
	if (!t) {
		t = ferrion_task_new("ferrion-host", NULL, NULL);
		t->thread = thread;
		ferrion_thread_set_context(thread, t);
	}
	resume(t);
}

void ferrion_leave(void)
{
	suspend(ferrion_current());
	ferrion_unlock();
}

void ferrion_exit_task(void)
{
	struct ferrion_task *t = ferrion_current();

	suspend(t);
	ferrion_thread_set_context(t->thread, NULL);
	this_cpu_write(current_task, NULL);
	kfree(t);
	ferrion_unlock();
}

void ferrion_task_sleep(struct ferrion_task *t, unsigned long long deadline_ns)
{
	suspend(t);
	ferrion_thread_wait(t->thread, deadline_ns);
	resume(t);
}

static int wake_task(struct task_struct *p, unsigned int state)
{
	struct ferrion_task *t = task_of(p);

	if (!(READ_ONCE(p->__state) & state))
		return 0;
	WRITE_ONCE(p->__state, TASK_RUNNING);
	if (t->thread && t != ferrion_current())
		ferrion_thread_wake(t->thread);
	return 1;
}

int wake_up_process(struct task_struct *p)
{
	return wake_task(p, TASK_NORMAL);
}

int wake_up_state(struct task_struct *p, unsigned int state)
{
	return wake_task(p, state);
}

asmlinkage __visible void schedule(void)
{
	struct ferrion_task *t = ferrion_current();

	while (READ_ONCE(t->task.__state) != TASK_RUNNING)
		ferrion_task_sleep(t, 0);
}

signed long schedule_timeout(signed long timeout)
{
	struct ferrion_task *t = ferrion_current();
	unsigned long expire;
	unsigned long long deadline;
	long left;

	if (timeout == MAX_SCHEDULE_TIMEOUT) {
		schedule();
		return timeout;
	}
	if (timeout < 0) {
		__set_current_state(TASK_RUNNING);
		return 0;
	}
	expire = jiffies + timeout;
	deadline = ferrion_jiffies_to_deadline(expire);
	while (READ_ONCE(t->task.__state) != TASK_RUNNING) {
		if (ferrion_now_ns() >= deadline) {
			__set_current_state(TASK_RUNNING);
			break;
		}
		ferrion_task_sleep(t, deadline);
	}
	ferrion_tick();
	left = (long)(expire - jiffies);
	return left < 0 ? 0 : left;
}

signed long schedule_timeout_uninterruptible(signed long timeout)
{
	__set_current_state(TASK_UNINTERRUPTIBLE);
	return schedule_timeout(timeout);
}

int __cond_resched(void)
{
	struct ferrion_task *t;

	if (ferrion_lock_waiters() == 0)
		return 0;
	t = ferrion_current();
	suspend(t);
	ferrion_yield();
	resume(t);
	return 1;
}

void sched_set_fifo(struct task_struct *p)
{
}

static int autoremove_wake(struct wait_queue_entry *wq_entry, unsigned int mode, int sync, void *key)
{
	int woken = wake_task(wq_entry->private, mode);

	if (woken)
		list_del_init_careful(&wq_entry->entry);
	return woken;
}

void __init_waitqueue_head(struct wait_queue_head *wq_head, const char *name, struct lock_class_key *key)
{
	spin_lock_init(&wq_head->lock);
	INIT_LIST_HEAD(&wq_head->head);
}

void init_wait_entry(struct wait_queue_entry *wq_entry, int flags)
{
	wq_entry->flags = flags;
	wq_entry->private = current;
	wq_entry->func = autoremove_wake;
	INIT_LIST_HEAD(&wq_entry->entry);
}

long prepare_to_wait_event(struct wait_queue_head *wq_head, struct wait_queue_entry *wq_entry, int state)
{
	if (signal_pending_state(state, current)) {
		list_del_init(&wq_entry->entry);
		return -ERESTARTSYS;
	}
	if (list_empty(&wq_entry->entry)) {
		if (wq_entry->flags & WQ_FLAG_EXCLUSIVE)
			list_add_tail(&wq_entry->entry, &wq_head->head);
		else
			list_add(&wq_entry->entry, &wq_head->head);
	}
	set_current_state(state);
	return 0;
}

void finish_wait(struct wait_queue_head *wq_head, struct wait_queue_entry *wq_entry)
{
	__set_current_state(TASK_RUNNING);
	if (!list_empty_careful(&wq_entry->entry))
		list_del_init(&wq_entry->entry);
}

static int wake_common(struct wait_queue_head *wq_head, unsigned int mode, int nr_exclusive, void *key)
{
	struct wait_queue_entry *curr, *next;
	int woken = 0;

	list_for_each_entry_safe(curr, next, &wq_head->head, entry) {
		unsigned int flags = curr->flags;
		int ret = curr->func(curr, mode, 0, key);

		if (ret < 0)
			break;
		if (ret)
			woken++;
		if (ret && (flags & WQ_FLAG_EXCLUSIVE) && !--nr_exclusive)
			break;
	}
	return woken;
}

int __wake_up(struct wait_queue_head *wq_head, unsigned int mode, int nr_exclusive, void *key)
{
	return wake_common(wq_head, mode, nr_exclusive, key);
}

void __wake_up_locked_key(struct wait_queue_head *wq_head, unsigned int mode, void *key)
{
	wake_common(wq_head, mode, 1, key);
}

void __init_swait_queue_head(struct swait_queue_head *q, const char *name, struct lock_class_key *key)
{
	raw_spin_lock_init(&q->lock);
	INIT_LIST_HEAD(&q->task_list);
}

static void swake_all(struct swait_queue_head *q)
{
	struct swait_queue *curr, *next;

	list_for_each_entry_safe(curr, next, &q->task_list, task_list) {
		wake_up_process(curr->task);
		list_del_init(&curr->task_list);
	}
}

static long swait_timeout(struct swait_queue_head *q, bool (*done)(void *), void *arg, long timeout,
			  unsigned int state)
{
	struct swait_queue wait = { .task = current };

	INIT_LIST_HEAD(&wait.task_list);
	while (!done(arg) && timeout) {
		if (signal_pending_state(state, current)) {
			timeout = -ERESTARTSYS;
			break;
		}
		if (list_empty(&wait.task_list))
			list_add_tail(&wait.task_list, &q->task_list);
		set_current_state(state);
		timeout = schedule_timeout(timeout);
	}
	__set_current_state(TASK_RUNNING);
	list_del_init(&wait.task_list);
	return timeout;
}

static bool completion_done_now(void *arg)
{
	return READ_ONCE(((struct completion *)arg)->done) != 0;
}

static long wait_completion(struct completion *x, long timeout, unsigned int state)
{
	if (!x->done)
		timeout = swait_timeout(&x->wait, completion_done_now, x, timeout, state);
	if (!x->done)
		return timeout;
	if (x->done != UINT_MAX)
		x->done--;
	return timeout ?: 1;
}

void complete_all(struct completion *x)
{
	x->done = UINT_MAX;
	swake_all(&x->wait);
}

void complete(struct completion *x)
{
	struct swait_queue *first;

	if (x->done != UINT_MAX)
		x->done++;
	first = list_first_entry_or_null(&x->wait.task_list, struct swait_queue, task_list);
	if (first) {
		wake_up_process(first->task);
		list_del_init(&first->task_list);
	}
}

void wait_for_completion(struct completion *x)
{
	wait_completion(x, MAX_SCHEDULE_TIMEOUT, TASK_UNINTERRUPTIBLE);
}

bool try_wait_for_completion(struct completion *x)
{
	if (!READ_ONCE(x->done))
		return false;
	if (x->done != UINT_MAX)
		x->done--;
	return true;
}

unsigned long wait_for_completion_timeout(struct completion *x, unsigned long timeout)
{
	return wait_completion(x, timeout, TASK_UNINTERRUPTIBLE);
}

int wait_for_completion_interruptible(struct completion *x)
{
	long t = wait_completion(x, MAX_SCHEDULE_TIMEOUT, TASK_INTERRUPTIBLE);

	return t == -ERESTARTSYS ? t : 0;
}

long wait_for_completion_interruptible_timeout(struct completion *x, unsigned long timeout)
{
	return wait_completion(x, timeout, TASK_INTERRUPTIBLE);
}

void __mutex_init(struct mutex *lock, const char *name, struct lock_class_key *key)
{
	atomic_long_set(&lock->owner, 0);
	raw_spin_lock_init(&lock->wait_lock);
	INIT_LIST_HEAD(&lock->wait_list);
}

bool mutex_is_locked(struct mutex *lock)
{
	return atomic_long_read(&lock->owner) != 0;
}

static int mutex_wait(struct mutex *lock, unsigned int state)
{
	struct ferrion_lock_waiter waiter = { .task = current };

	if (atomic_long_read(&lock->owner) == (long)current)
		ferrion_abort("mutex_lock: the owner locks again");
	if (signal_pending_state(state, current))
		return -EINTR;
	list_add_tail(&waiter.list, &lock->wait_list);
	set_current_state(state);
	schedule();
	list_del(&waiter.list);
	return 0;
}

static int mutex_lock_state(struct mutex *lock, unsigned int state)
{
	while (mutex_is_locked(lock)) {
		int ret = mutex_wait(lock, state);

		if (ret)
			return ret;
	}
	atomic_long_set(&lock->owner, (long)current);
	return 0;
}

void mutex_lock(struct mutex *lock)
{
	mutex_lock_state(lock, TASK_UNINTERRUPTIBLE);
}

int mutex_lock_interruptible(struct mutex *lock)
{
	return mutex_lock_state(lock, TASK_INTERRUPTIBLE);
}

int mutex_trylock(struct mutex *lock)
{
	if (mutex_is_locked(lock))
		return 0;
	atomic_long_set(&lock->owner, (long)current);
	return 1;
}

void mutex_unlock(struct mutex *lock)
{
	struct ferrion_lock_waiter *first;

	if (atomic_long_read(&lock->owner) != (long)current)
		ferrion_abort("mutex_unlock: not the owner");
	atomic_long_set(&lock->owner, 0);
	first = list_first_entry_or_null(&lock->wait_list, struct ferrion_lock_waiter, list);
	if (first)
		wake_up_process(first->task);
}

int atomic_dec_and_mutex_lock(atomic_t *cnt, struct mutex *lock)
{
	if (atomic_add_unless(cnt, -1, 1))
		return 0;
	mutex_lock(lock);
	if (!atomic_dec_and_test(cnt)) {
		mutex_unlock(lock);
		return 0;
	}
	return 1;
}

static bool ww_younger(struct ww_acquire_ctx *a, struct ww_acquire_ctx *b)
{
	return (long)(a->stamp - b->stamp) > 0;
}

static int ww_lock_state(struct ww_mutex *lock, struct ww_acquire_ctx *ctx, unsigned int state)
{
	if (ctx && READ_ONCE(lock->ctx) == ctx)
		return -EALREADY;
	while (mutex_is_locked(&lock->base)) {
		int ret;

		if (ctx && ctx->acquired > 0 && lock->ctx && ww_younger(ctx, lock->ctx))
			return -EDEADLK;
		ret = mutex_wait(&lock->base, state);
		if (ret)
			return ret;
	}
	atomic_long_set(&lock->base.owner, (long)current);
	lock->ctx = ctx;
	if (ctx)
		ctx->acquired++;
	return 0;
}

int ww_mutex_lock(struct ww_mutex *lock, struct ww_acquire_ctx *ctx)
{
	return ww_lock_state(lock, ctx, TASK_UNINTERRUPTIBLE);
}

int ww_mutex_lock_interruptible(struct ww_mutex *lock, struct ww_acquire_ctx *ctx)
{
	return ww_lock_state(lock, ctx, TASK_INTERRUPTIBLE);
}

int ww_mutex_trylock(struct ww_mutex *lock, struct ww_acquire_ctx *ctx)
{
	if (mutex_is_locked(&lock->base))
		return 0;
	atomic_long_set(&lock->base.owner, (long)current);
	lock->ctx = ctx;
	if (ctx)
		ctx->acquired++;
	return 1;
}

void ww_mutex_unlock(struct ww_mutex *lock)
{
	if (lock->ctx)
		lock->ctx->acquired--;
	lock->ctx = NULL;
	mutex_unlock(&lock->base);
}

void ferrion_rcu_drain(void)
{
	unsigned int n;

	for (n = 0; n < FERRION_RCU_DRAIN_MAX && (rcu_callbacks || rcu_frees); n++) {
		struct rcu_head *head;

		if (rcu_frees) {
			head = rcu_frees;
			rcu_frees = head->next;
			kvfree((void *)head->func);
			continue;
		}
		head = rcu_callbacks;
		rcu_callbacks = head->next;
		head->func(head);
	}
}

void call_rcu(struct rcu_head *head, rcu_callback_t func)
{
	head->func = func;
	head->next = rcu_callbacks;
	rcu_callbacks = head;
}

void kvfree_call_rcu(struct rcu_head *head, void *ptr)
{
	if (!head) {
		synchronize_rcu();
		kvfree(ptr);
		return;
	}
	head->func = (rcu_callback_t)ptr;
	head->next = rcu_frees;
	rcu_frees = head;
}

void synchronize_rcu(void)
{
	ferrion_rcu_drain();
}

struct srcu_drain {
	struct srcu_struct *ssp;
	int idx;
};

static bool srcu_readers_gone(void *arg)
{
	struct srcu_drain *drain = arg;

	return READ_ONCE(drain->ssp->srcu_lock_nesting[drain->idx]) == 0;
}

void __srcu_read_unlock(struct srcu_struct *ssp, int idx)
{
	int nesting = READ_ONCE(ssp->srcu_lock_nesting[idx]) - 1;

	WRITE_ONCE(ssp->srcu_lock_nesting[idx], nesting);
	if (!nesting && READ_ONCE(ssp->srcu_gp_waiting))
		swake_all(&ssp->srcu_wq);
}

void synchronize_srcu(struct srcu_struct *ssp)
{
	struct srcu_drain drain = { .ssp = ssp, .idx = ((READ_ONCE(ssp->srcu_idx) + 1) & 0x2) >> 1 };

	WRITE_ONCE(ssp->srcu_idx, ssp->srcu_idx + 1);
	WRITE_ONCE(ssp->srcu_gp_waiting, true);
	swait_timeout(&ssp->srcu_wq, srcu_readers_gone, &drain, MAX_SCHEDULE_TIMEOUT, TASK_UNINTERRUPTIBLE);
	WRITE_ONCE(ssp->srcu_gp_waiting, false);
	WRITE_ONCE(ssp->srcu_idx, ssp->srcu_idx + 1);
	ferrion_rcu_drain();
}

void __local_bh_enable_ip(unsigned long ip, unsigned int cnt)
{
	__preempt_count_sub(cnt);
}

pid_t __task_pid_nr_ns(struct task_struct *task, enum pid_type type, struct pid_namespace *ns)
{
	return 0;
}

struct task_struct *pid_task(struct pid *pid, enum pid_type type)
{
	if (pid)
		ferrion_abort("pid_task: the shim creates no struct pid");
	return NULL;
}

void put_pid(struct pid *pid)
{
	if (pid)
		ferrion_abort("put_pid: the shim creates no struct pid");
}

#ifdef CONFIG_MULTIUSER
bool capable(int cap)
{
	return false;
}
#endif

void kernel_fpu_begin_mask(unsigned int kfpu_mask)
{
}

void kernel_fpu_end(void)
{
}
