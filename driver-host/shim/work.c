// SPDX-License-Identifier: GPL-2.0-only
#include <linux/kthread.h>
#include <linux/sched.h>
#include <linux/slab.h>
#include <linux/timer.h>
#include <linux/wait.h>
#include <linux/workqueue.h>
#include <ferrion/linux.h>

#define FERRION_WQ_WORKERS 4
#define FERRION_WQ_NAME_MAX 24

struct workqueue_struct {
	char name[FERRION_WQ_NAME_MAX];
	struct list_head works;
	struct ferrion_task *workers[FERRION_WQ_WORKERS];
	struct work_struct *running[FERRION_WQ_WORKERS];
	unsigned int nr_workers;
	unsigned int nr_exited;
	bool dying;
	wait_queue_head_t idle;
} __aligned(1 << WORK_STRUCT_PWQ_SHIFT);

struct wq_worker {
	struct workqueue_struct *wq;
	unsigned int index;
};

struct ferrion_kworker {
	struct kthread_worker worker;
	struct ferrion_task *task;
	bool stop;
	bool exited;
	wait_queue_head_t idle;
};

struct workqueue_struct *system_wq;
struct workqueue_struct *system_percpu_wq;
struct workqueue_struct *system_long_wq;
struct workqueue_struct *system_unbound_wq;

static struct workqueue_struct *work_wq(struct work_struct *work)
{
	return (struct workqueue_struct *)(atomic_long_read(&work->data) & WORK_STRUCT_PWQ_MASK);
}

static bool work_running(struct workqueue_struct *wq, struct work_struct *work)
{
	unsigned int i;

	for (i = 0; i < wq->nr_workers; i++)
		if (wq->running[i] == work)
			return true;
	return false;
}

static void wake_workers(struct workqueue_struct *wq)
{
	unsigned int i;

	for (i = 0; i < wq->nr_workers; i++)
		if (wq->workers[i])
			wake_up_process(&wq->workers[i]->task);
}

static void insert_work(struct workqueue_struct *wq, struct work_struct *work)
{
	atomic_long_set(&work->data, (unsigned long)wq | WORK_STRUCT_PENDING | WORK_STRUCT_PWQ);
	list_add_tail(&work->entry, &wq->works);
	wake_workers(wq);
}

static void worker_main(void *arg)
{
	struct wq_worker *w = arg;
	struct workqueue_struct *wq = w->wq;
	unsigned int index = w->index;
	struct ferrion_task *self = ferrion_current();

	kfree(w);
	for (;;) {
		struct work_struct *work;

		set_current_state(TASK_IDLE);
		if (list_empty(&wq->works)) {
			if (wq->dying)
				break;
			schedule();
			continue;
		}
		__set_current_state(TASK_RUNNING);
		work = list_first_entry(&wq->works, struct work_struct, entry);
		list_del_init(&work->entry);
		atomic_long_andnot(WORK_STRUCT_PENDING, &work->data);
		wq->running[index] = work;
		self->current_work = work;
		work->func(work);
		self->current_work = NULL;
		wq->running[index] = NULL;
		wake_up_all(&wq->idle);
	}
	__set_current_state(TASK_RUNNING);
	wq->workers[index] = NULL;
	wq->nr_exited++;
	wake_up_all(&wq->idle);
}

static struct workqueue_struct *wq_create(const char *name, unsigned int workers)
{
	struct workqueue_struct *wq = kzalloc(sizeof(*wq), GFP_KERNEL);
	unsigned int i;

	if (!wq)
		return NULL;
	strscpy(wq->name, name, sizeof(wq->name));
	INIT_LIST_HEAD(&wq->works);
	init_waitqueue_head(&wq->idle);
	wq->nr_workers = workers;
	for (i = 0; i < workers; i++) {
		struct wq_worker *w = kzalloc(sizeof(*w), GFP_KERNEL);

		if (!w)
			ferrion_abort("workqueue worker allocation");
		w->wq = wq;
		w->index = i;
		wq->workers[i] = ferrion_task_new(name, worker_main, w);
		if (ferrion_task_start(wq->workers[i]))
			ferrion_abort("workqueue worker thread");
	}
	return wq;
}

void destroy_workqueue(struct workqueue_struct *wq)
{
	wait_event(wq->idle, list_empty(&wq->works));
	wq->dying = true;
	wake_workers(wq);
	wait_event(wq->idle, wq->nr_exited == wq->nr_workers);
	kfree(wq);
}

bool queue_work_on(int cpu, struct workqueue_struct *wq, struct work_struct *work)
{
	if (test_and_set_bit(WORK_STRUCT_PENDING_BIT, work_data_bits(work)))
		return false;
	insert_work(wq, work);
	return true;
}

void delayed_work_timer_fn(struct timer_list *t)
{
	struct delayed_work *dwork = timer_container_of(dwork, t, timer);

	insert_work(dwork->wq, &dwork->work);
}

static void queue_delayed(int cpu, struct workqueue_struct *wq, struct delayed_work *dwork, unsigned long delay)
{
	dwork->wq = wq;
	dwork->cpu = cpu;
	if (!delay) {
		insert_work(wq, &dwork->work);
		return;
	}
	atomic_long_set(&dwork->work.data, (unsigned long)wq | WORK_STRUCT_PENDING | WORK_STRUCT_PWQ);
	mod_timer(&dwork->timer, jiffies + delay);
}

bool queue_delayed_work_on(int cpu, struct workqueue_struct *wq, struct delayed_work *dwork, unsigned long delay)
{
	if (test_and_set_bit(WORK_STRUCT_PENDING_BIT, work_data_bits(&dwork->work)))
		return false;
	queue_delayed(cpu, wq, dwork, delay);
	return true;
}

static bool grab_delayed(struct delayed_work *dwork)
{
	bool pending = timer_delete_sync(&dwork->timer);

	if (!list_empty(&dwork->work.entry)) {
		list_del_init(&dwork->work.entry);
		pending = true;
	}
	atomic_long_andnot(WORK_STRUCT_PENDING, &dwork->work.data);
	return pending;
}

bool mod_delayed_work_on(int cpu, struct workqueue_struct *wq, struct delayed_work *dwork, unsigned long delay)
{
	bool pending = grab_delayed(dwork);

	set_bit(WORK_STRUCT_PENDING_BIT, work_data_bits(&dwork->work));
	queue_delayed(cpu, wq, dwork, delay);
	return pending;
}

bool flush_work(struct work_struct *work)
{
	struct workqueue_struct *wq = work_wq(work);

	if (!wq || (!work_pending(work) && !work_running(wq, work)))
		return false;
	if (ferrion_current()->current_work == work)
		ferrion_abort("flush_work: a work item flushes itself");
	wait_event(wq->idle, !work_pending(work) && !work_running(wq, work));
	return true;
}

bool cancel_delayed_work_sync(struct delayed_work *dwork)
{
	bool pending = grab_delayed(dwork);
	struct workqueue_struct *wq = work_wq(&dwork->work);

	if (wq && ferrion_current()->current_work != &dwork->work)
		wait_event(wq->idle, !work_running(wq, &dwork->work));
	return pending;
}

struct work_struct *current_work(void)
{
	return ferrion_current()->current_work;
}

void ferrion_workqueue_init(void)
{
	system_percpu_wq = wq_create("events", FERRION_WQ_WORKERS);
	system_long_wq = wq_create("events_long", 1);
	system_unbound_wq = wq_create("events_unbound", FERRION_WQ_WORKERS);
	if (!system_percpu_wq || !system_long_wq || !system_unbound_wq)
		ferrion_abort("system workqueue allocation");
	system_wq = system_percpu_wq;
}

static void kworker_main(void *arg)
{
	struct ferrion_kworker *kw = arg;
	struct kthread_worker *worker = &kw->worker;

	for (;;) {
		struct kthread_work *work;

		set_current_state(TASK_IDLE);
		if (list_empty(&worker->work_list)) {
			if (kw->stop)
				break;
			schedule();
			continue;
		}
		__set_current_state(TASK_RUNNING);
		work = list_first_entry(&worker->work_list, struct kthread_work, node);
		list_del_init(&work->node);
		worker->current_work = work;
		work->func(work);
		worker->current_work = NULL;
		wake_up_all(&kw->idle);
	}
	__set_current_state(TASK_RUNNING);
	kw->exited = true;
	wake_up_all(&kw->idle);
}

struct kthread_worker *kthread_create_worker_on_node(unsigned int flags, int node, const char namefmt[], ...)
{
	struct ferrion_kworker *kw = kzalloc(sizeof(*kw), GFP_KERNEL);
	char name[FERRION_TASK_NAME_MAX];
	va_list args;

	if (!kw)
		return ERR_PTR(-ENOMEM);
	va_start(args, namefmt);
	ferrion_vformat(name, sizeof(name), namefmt, args);
	va_end(args);
	kw->worker.flags = flags;
	raw_spin_lock_init(&kw->worker.lock);
	INIT_LIST_HEAD(&kw->worker.work_list);
	INIT_LIST_HEAD(&kw->worker.delayed_work_list);
	init_waitqueue_head(&kw->idle);
	kw->task = ferrion_task_new(name, kworker_main, kw);
	kw->worker.task = &kw->task->task;
	if (ferrion_task_start(kw->task)) {
		kfree(kw->task);
		kfree(kw);
		return ERR_PTR(-EAGAIN);
	}
	return &kw->worker;
}

static struct ferrion_kworker *kworker_of(struct kthread_worker *worker)
{
	return container_of(worker, struct ferrion_kworker, worker);
}

bool kthread_queue_work(struct kthread_worker *worker, struct kthread_work *work)
{
	if (!list_empty(&work->node) || work->canceling)
		return false;
	list_add_tail(&work->node, &worker->work_list);
	work->worker = worker;
	wake_up_process(worker->task);
	return true;
}

void kthread_flush_work(struct kthread_work *work)
{
	struct kthread_worker *worker = work->worker;

	if (!worker)
		return;
	wait_event(kworker_of(worker)->idle, list_empty(&work->node) && worker->current_work != work);
}

void kthread_flush_worker(struct kthread_worker *worker)
{
	wait_event(kworker_of(worker)->idle, list_empty(&worker->work_list) && !worker->current_work);
}

bool kthread_cancel_work_sync(struct kthread_work *work)
{
	struct kthread_worker *worker = work->worker;
	bool pending;

	if (!worker)
		return false;
	work->canceling++;
	pending = !list_empty(&work->node);
	list_del_init(&work->node);
	wait_event(kworker_of(worker)->idle, worker->current_work != work);
	work->canceling--;
	return pending;
}

void kthread_destroy_worker(struct kthread_worker *worker)
{
	struct ferrion_kworker *kw = kworker_of(worker);

	kthread_flush_worker(worker);
	kw->stop = true;
	wake_up_process(worker->task);
	wait_event(kw->idle, kw->exited);
	kfree(kw);
}
