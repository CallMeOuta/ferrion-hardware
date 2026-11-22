// SPDX-License-Identifier: GPL-2.0-only
#include <linux/hrtimer.h>
#include <linux/irq_work.h>
#include <linux/jiffies.h>
#include <linux/sched.h>
#include <linux/time64.h>
#include <linux/timekeeping.h>
#include <linux/timer.h>
#include <linux/wait.h>
#include <ferrion/linux.h>

#define FERRION_TIMERS_MAX 1024
#define FERRION_HRTIMERS_MAX 64
#define FERRION_NS_PER_JIFFY (NSEC_PER_SEC / HZ)

unsigned long volatile __cacheline_aligned_in_smp __jiffy_arch_data jiffies = INITIAL_JIFFIES;

static unsigned long long boot_ns;
static HLIST_HEAD(timers);
static unsigned int timer_count;
static struct timer_list *running_timer;
static struct hrtimer *hrtimers[FERRION_HRTIMERS_MAX];
static unsigned int hrtimer_count;
static struct hrtimer *running_hrtimer;
static struct irq_work *irq_works;
static DECLARE_WAIT_QUEUE_HEAD(timer_done);
static struct ferrion_task *timer_task;
static bool kicked;

void ferrion_tick(void)
{
	unsigned long long now = ferrion_now_ns();

	if (!boot_ns)
		boot_ns = now;
	jiffies = INITIAL_JIFFIES + (unsigned long)((now - boot_ns) / FERRION_NS_PER_JIFFY);
}

unsigned long long ferrion_jiffies_to_deadline(unsigned long expires)
{
	long delta;

	ferrion_tick();
	delta = (long)(expires - jiffies);
	if (delta < 0)
		delta = 0;
	return ferrion_now_ns() + (unsigned long long)delta * FERRION_NS_PER_JIFFY;
}

static void kick(void)
{
	kicked = true;
	if (timer_task && timer_task->thread && timer_task != ferrion_current())
		ferrion_thread_wake(timer_task->thread);
}

ktime_t ktime_get(void)
{
	return (ktime_t)ferrion_now_ns();
}

ktime_t ktime_add_safe(const ktime_t lhs, const ktime_t rhs)
{
	ktime_t res = ktime_add_unsafe(lhs, rhs);

	if (res < 0 || res < lhs || res < rhs)
		res = ktime_set(KTIME_SEC_MAX, 0);
	return res;
}

unsigned long __msecs_to_jiffies(const unsigned int m)
{
	if ((int)m < 0)
		return MAX_JIFFY_OFFSET;
	return _msecs_to_jiffies(m);
}

u64 nsecs_to_jiffies64(u64 n)
{
	return div_u64(n, FERRION_NS_PER_JIFFY);
}

struct timespec64 ns_to_timespec64(s64 nsec)
{
	struct timespec64 ts = { 0, 0 };
	s32 rem;

	if (!nsec)
		return ts;
	ts.tv_sec = div_s64_rem(nsec, NSEC_PER_SEC, &rem);
	if (rem < 0) {
		ts.tv_sec--;
		rem += NSEC_PER_SEC;
	}
	ts.tv_nsec = rem;
	return ts;
}

void timer_init_key(struct timer_list *timer, void (*func)(struct timer_list *), unsigned int flags,
		    const char *name, struct lock_class_key *key)
{
	timer->entry.pprev = NULL;
	timer->entry.next = NULL;
	timer->function = func;
	timer->flags = flags;
}

static void timer_detach(struct timer_list *timer)
{
	hlist_del_init(&timer->entry);
	timer_count--;
}

int mod_timer(struct timer_list *timer, unsigned long expires)
{
	int pending = timer_pending(timer);

	if (pending && timer->expires == expires)
		return 1;
	if (pending)
		timer_detach(timer);
	if (timer_count >= FERRION_TIMERS_MAX)
		ferrion_abort("mod_timer: FERRION_TIMERS_MAX timers are pending");
	timer->expires = expires;
	hlist_add_head(&timer->entry, &timers);
	timer_count++;
	kick();
	return pending;
}

int timer_delete_sync(struct timer_list *timer)
{
	int pending = timer_pending(timer);

	if (pending)
		timer_detach(timer);
	if (ferrion_current() != timer_task)
		wait_event(timer_done, READ_ONCE(running_timer) != timer);
	return pending;
}

static int hrtimer_slot(const struct hrtimer *timer)
{
	unsigned int i;

	for (i = 0; i < hrtimer_count; i++)
		if (hrtimers[i] == timer)
			return i;
	return -1;
}

static bool hrtimer_dequeue(struct hrtimer *timer)
{
	int i = hrtimer_slot(timer);

	if (i < 0)
		return false;
	hrtimers[i] = hrtimers[--hrtimer_count];
	timer->state = HRTIMER_STATE_INACTIVE;
	return true;
}

static void hrtimer_enqueue(struct hrtimer *timer)
{
	if (hrtimer_slot(timer) < 0) {
		if (hrtimer_count >= FERRION_HRTIMERS_MAX)
			ferrion_abort("hrtimer_start: FERRION_HRTIMERS_MAX timers are armed");
		hrtimers[hrtimer_count++] = timer;
	}
	timer->state = HRTIMER_STATE_ENQUEUED;
	kick();
}

void hrtimer_setup(struct hrtimer *timer, enum hrtimer_restart (*function)(struct hrtimer *),
		   clockid_t clock_id, enum hrtimer_mode mode)
{
	if (clock_id != CLOCK_MONOTONIC && clock_id != CLOCK_BOOTTIME)
		ferrion_abort("hrtimer_setup: only the monotonic clock exists here");
	memset(timer, 0, sizeof(*timer));
	ACCESS_PRIVATE(timer, function) = function;
	timer->is_rel = !!(mode & HRTIMER_MODE_REL);
	timer->is_soft = !!(mode & HRTIMER_MODE_SOFT);
	timer->is_hard = !!(mode & HRTIMER_MODE_HARD);
}

void hrtimer_start_range_ns(struct hrtimer *timer, ktime_t tim, u64 range_ns, const enum hrtimer_mode mode)
{
	if (mode & HRTIMER_MODE_REL)
		tim = ktime_add_safe(ktime_get(), tim);
	hrtimer_set_expires_range_ns(timer, tim, range_ns);
	hrtimer_enqueue(timer);
}

int hrtimer_try_to_cancel(struct hrtimer *timer)
{
	if (hrtimer_dequeue(timer))
		return 1;
	return READ_ONCE(running_hrtimer) == timer ? -1 : 0;
}

int hrtimer_cancel(struct hrtimer *timer)
{
	int ret = hrtimer_dequeue(timer);

	if (ferrion_current() != timer_task)
		wait_event(timer_done, READ_ONCE(running_hrtimer) != timer);
	return ret;
}

bool hrtimer_active(const struct hrtimer *timer)
{
	return timer->state != HRTIMER_STATE_INACTIVE || READ_ONCE(running_hrtimer) == timer;
}

ktime_t hrtimer_cb_get_time(const struct hrtimer *timer)
{
	return ktime_get();
}

u64 hrtimer_forward(struct hrtimer *timer, ktime_t now, ktime_t interval)
{
	s64 delta = ktime_sub(now, hrtimer_get_expires(timer));
	u64 orun = 1;

	if (delta < 0)
		return 0;
	if (interval < 1)
		interval = 1;
	if (delta >= interval) {
		orun = ktime_divns(delta, interval);
		hrtimer_add_expires_ns(timer, interval * orun);
		if (hrtimer_get_expires(timer) > now)
			return orun;
		orun++;
	}
	hrtimer_add_expires(timer, interval);
	return orun;
}

bool irq_work_queue(struct irq_work *work)
{
	if (atomic_read(&work->node.a_flags) & IRQ_WORK_PENDING)
		return false;
	atomic_or(IRQ_WORK_CLAIMED | CSD_TYPE_IRQ_WORK, &work->node.a_flags);
	work->node.llist.next = (struct llist_node *)irq_works;
	irq_works = work;
	kick();
	return true;
}

static void run_irq_works(void)
{
	while (irq_works) {
		struct irq_work *work = irq_works;

		irq_works = (struct irq_work *)work->node.llist.next;
		work->node.llist.next = NULL;
		atomic_andnot(IRQ_WORK_PENDING, &work->node.a_flags);
		preempt_count_add(HARDIRQ_OFFSET);
		work->func(work);
		preempt_count_sub(HARDIRQ_OFFSET);
		atomic_andnot(IRQ_WORK_BUSY, &work->node.a_flags);
	}
}

static struct timer_list *first_expired_timer(void)
{
	struct timer_list *timer;

	hlist_for_each_entry(timer, &timers, entry)
		if (time_after_eq(jiffies, timer->expires))
			return timer;
	return NULL;
}

static void run_timers(void)
{
	unsigned int n;

	ferrion_tick();
	for (n = 0; n < FERRION_TIMERS_MAX; n++) {
		struct timer_list *timer = first_expired_timer();

		if (!timer)
			break;
		timer_detach(timer);
		running_timer = timer;
		preempt_count_add(SOFTIRQ_OFFSET);
		timer->function(timer);
		preempt_count_sub(SOFTIRQ_OFFSET);
		running_timer = NULL;
		wake_up_all(&timer_done);
	}
}

static struct hrtimer *first_expired_hrtimer(ktime_t now)
{
	unsigned int i;

	for (i = 0; i < hrtimer_count; i++)
		if (hrtimer_get_softexpires(hrtimers[i]) <= now)
			return hrtimers[i];
	return NULL;
}

static void run_hrtimers(void)
{
	unsigned int n;

	for (n = 0; n < FERRION_HRTIMERS_MAX; n++) {
		struct hrtimer *timer = first_expired_hrtimer(ktime_get());
		enum hrtimer_restart restart;

		if (!timer)
			break;
		hrtimer_dequeue(timer);
		running_hrtimer = timer;
		preempt_count_add(HARDIRQ_OFFSET);
		restart = ACCESS_PRIVATE(timer, function)(timer);
		preempt_count_sub(HARDIRQ_OFFSET);
		running_hrtimer = NULL;
		if (restart == HRTIMER_RESTART && timer->state == HRTIMER_STATE_INACTIVE)
			hrtimer_enqueue(timer);
		wake_up_all(&timer_done);
	}
}

static unsigned long long next_deadline(void)
{
	unsigned long long deadline = 0;
	struct timer_list *timer;
	unsigned int i;

	hlist_for_each_entry(timer, &timers, entry) {
		unsigned long long at = ferrion_jiffies_to_deadline(timer->expires);

		if (!deadline || at < deadline)
			deadline = at;
	}
	for (i = 0; i < hrtimer_count; i++) {
		unsigned long long at = (unsigned long long)hrtimer_get_softexpires(hrtimers[i]);

		if (!deadline || at < deadline)
			deadline = at;
	}
	return deadline;
}

static void timer_main(void *arg)
{
	struct ferrion_task *self = ferrion_current();

	for (;;) {
		kicked = false;
		run_irq_works();
		run_timers();
		run_hrtimers();
		if (!kicked)
			ferrion_task_sleep(self, next_deadline());
	}
}

void ferrion_time_init(void)
{
	ferrion_tick();
	timer_task = ferrion_task_new("ferrion-timers", timer_main, NULL);
	if (ferrion_task_start(timer_task))
		ferrion_abort("ferrion_time_init: no thread for timers");
}
