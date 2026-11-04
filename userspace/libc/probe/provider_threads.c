#include <errno.h>
#include <fenv.h>
#include <stdint.h>
#include <stdatomic.h>
#include <stdlib.h>
#include <threads.h>
#include <cryolite/native.h>

extern size_t __cryolite_thread_records(void);

#define TLS_INITIAL UINT64_C(0x7ac2139485b6)
#define PAYLOAD_MAGIC UINT64_C(0x53bd71ac93)
#define WAIT_NS UINT64_C(10000000000)
#define TURNS 64

static _Thread_local volatile uint64_t thread_value = TLS_INITIAL;
static _Thread_local volatile uint64_t thread_zero;
static once_flag initialize_once = ONCE_FLAG_INIT;
static unsigned once_count;
static _Atomic unsigned destructed;
static kryo_handle test_debug;
static thrd_t main_thread;
static tss_t value_key;

struct group {
    mtx_t mutex;
    cnd_t changed;
    _Atomic unsigned ready;
    unsigned open;
    unsigned alternate;
    unsigned turn;
    unsigned counter;
};

struct job {
    struct group *group;
    thrd_t thread;
    unsigned number;
    unsigned slot;
    unsigned self_detach;
    _Atomic unsigned done;
};

struct value {
    uint64_t magic;
    unsigned number;
};

static void require(int condition, unsigned stage) {
    if (!condition) {
        (void)kryo_debug_log(test_debug, 209, stage);
        kryo_exit(stage);
    }
}

static void event(unsigned number, unsigned value) {
    require(kryo_debug_log(test_debug, number, value).status == KRYO_STATUS_OK, 400);
}

static uint64_t now(void) {
    kryo_result result = kryo_clock();
    require(result.status == KRYO_STATUS_OK, 401);
    return result.value;
}

static uint64_t deadline(void) {
    uint64_t start = now();
    require(start <= UINT64_MAX - WAIT_NS, 402);
    return start + WAIT_NS;
}

static void wait_value(const _Atomic unsigned *value, unsigned expected) {
    uint64_t until = deadline();
    while (atomic_load_explicit(value, memory_order_acquire) != expected) {
        require(now() < until, 403);
        thrd_yield();
    }
}

static void wait_retirement(void) {
    uint64_t until = deadline();
    while (__cryolite_thread_records() != 0) {
        require(now() < until, 404);
        thrd_yield();
    }
    // Native backing has retired; allow deferred kernel thread cleanup before
    // the next trusted account snapshot.
    thrd_yield();
}

static int local_error(unsigned number) {
    return number & 1 ? EINVAL : EFAULT;
}

static void initialize(void) {
    ++once_count;
    thrd_yield();
}

static void destroy_value(void *pointer) {
    require(tss_get(value_key) == NULL, 450);
    struct value *value = pointer;
    require(value && value->magic == PAYLOAD_MAGIC, 405);
    require(thread_value == (TLS_INITIAL ^ value->number)
            && thread_zero == value->number, 406);
    require(errno == local_error(value->number), 407);
    value->magic = 0;
    free(value);
    atomic_fetch_add_explicit(&destructed, 1, memory_order_release);
}

static void prepare_group(struct group *group, unsigned alternate) {
    require(mtx_lock(&group->mutex) == thrd_success, 408);
    atomic_store_explicit(&group->ready, 0, memory_order_relaxed);
    group->open = 0;
    group->alternate = alternate;
    group->turn = 0;
    group->counter = 0;
    require(mtx_unlock(&group->mutex) == thrd_success, 409);
}

static void open_group(struct group *group) {
    require(mtx_lock(&group->mutex) == thrd_success, 410);
    group->open = 1;
    require(cnd_broadcast(&group->changed) == thrd_success, 411);
    require(mtx_unlock(&group->mutex) == thrd_success, 412);
}

static int run_job(void *argument) {
    struct job *job = argument;
    require(thread_value == TLS_INITIAL && thread_zero == 0, 413);
    require(!thrd_equal(thrd_current(), main_thread), 414);
    require(fegetround() == FE_DOWNWARD, 415);
    require(fesetround(FE_UPWARD) == 0, 416);
    thread_value ^= job->number;
    thread_zero = job->number;
    errno = local_error(job->number);
    call_once(&initialize_once, initialize);
    require(once_count == 1, 417);

    struct value *value = malloc(sizeof(*value));
    require(value != NULL, 418);
    value->magic = PAYLOAD_MAGIC;
    value->number = job->number;
    require(tss_set(value_key, value) == thrd_success && tss_get(value_key) == value, 419);

    if (job->self_detach) {
        require(thrd_detach(thrd_current()) == thrd_success, 420);
        atomic_store_explicit(&job->done, 1, memory_order_release);
        return 1000 + (int)job->number;
    }

    struct group *group = job->group;
    require(mtx_lock(&group->mutex) == thrd_success, 421);
    atomic_fetch_add_explicit(&group->ready, 1, memory_order_release);
    while (!group->open)
        require(cnd_wait(&group->changed, &group->mutex) == thrd_success, 422);
    require(thrd_equal(thrd_current(), job->thread), 423);
    if (group->alternate) {
        for (unsigned turn = 0; turn < TURNS; ++turn) {
            while (group->turn != job->slot)
                require(cnd_wait(&group->changed, &group->mutex) == thrd_success, 424);
            ++group->counter;
            group->turn ^= 1;
            require(cnd_broadcast(&group->changed) == thrd_success, 425);
        }
    }
    require(mtx_unlock(&group->mutex) == thrd_success, 426);
    require(thread_value == (TLS_INITIAL ^ job->number)
            && thread_zero == job->number, 427);
    require(errno == local_error(job->number) && fegetround() == FE_UPWARD, 428);
    atomic_store_explicit(&job->done, 1, memory_order_release);
    return 1000 + (int)job->number;
}

static void start_job(struct job *job, struct group *group, unsigned number,
                      unsigned slot, unsigned self_detach) {
    job->group = group;
    job->thread = NULL;
    job->number = number;
    job->slot = slot;
    job->self_detach = self_detach;
    atomic_init(&job->done, 0);
    require(thrd_create(&job->thread, run_job, job) == thrd_success, 429);
}

static void join_job(struct job *job) {
    int result = 0;
    uint64_t until = deadline();
    int status;
    while ((status = thrd_join(job->thread, &result)) != thrd_success) {
        require(status == thrd_error && now() < until, 430);
        thrd_yield();
    }
    require(result == 1000 + (int)job->number, 451);
}

void check_provider_threads(kryo_handle debug) {
    test_debug = debug;
    main_thread = thrd_current();
    int saved_error = errno;
    fenv_t saved_environment;
    require(fegetenv(&saved_environment) == 0 && fesetround(FE_DOWNWARD) == 0, 431);
    require(thrd_equal(main_thread, thrd_current()) && fegetround() == FE_DOWNWARD, 432);
    thread_value = UINT64_C(0x3185ac29);
    thread_zero = UINT64_C(0x81927);
    require(tss_create(&value_key, destroy_value) == thrd_success, 433);

    struct group group;
    require(mtx_init(&group.mutex, mtx_plain) == thrd_success, 434);
    require(cnd_init(&group.changed) == thrd_success, 435);
    atomic_init(&group.ready, 0);
    prepare_group(&group, 1);
    struct job pair[2];
    start_job(&pair[0], &group, 1, 0, 0);
    start_job(&pair[1], &group, 2, 1, 0);
    wait_value(&group.ready, 2);
    require(!thrd_equal(pair[0].thread, pair[1].thread), 436);
    errno = EOVERFLOW;
    open_group(&group);
    join_job(&pair[0]);
    join_job(&pair[1]);
    wait_retirement();
    require(group.counter == 2 * TURNS && errno == EOVERFLOW, 437);
    require(atomic_load_explicit(&destructed, memory_order_acquire) == 2, 438);

    struct job detached;
    prepare_group(&group, 0);
    start_job(&detached, &group, 3, 0, 0);
    wait_value(&group.ready, 1);
    require(thrd_detach(detached.thread) == thrd_success, 439);
    open_group(&group);
    wait_retirement();
    require(atomic_load_explicit(&destructed, memory_order_acquire) == 3, 440);

    prepare_group(&group, 0);
    start_job(&detached, &group, 4, 0, 0);
    wait_value(&group.ready, 1);
    open_group(&group);
    wait_value(&detached.done, 1);
    // This flag only precedes language exit. The reaper still has to observe
    // actual native completion before records() can report retirement.
    require(thrd_detach(detached.thread) == thrd_success, 441);
    wait_retirement();
    require(atomic_load_explicit(&destructed, memory_order_acquire) == 4, 442);

    prepare_group(&group, 0);
    start_job(&pair[0], &group, 5, 0, 0);
    start_job(&pair[1], &group, 6, 1, 0);
    wait_value(&group.ready, 2);
    event(206, 0);
    struct job failed = {.group = &group, .number = 99};
    thrd_t rejected = NULL;
    // The live-thread quota maps to EAGAIN, distinct from backing-memory OOM.
    require(thrd_create(&rejected, run_job, &failed) == thrd_error
            && rejected == NULL, 443);
    event(206, 1);
    open_group(&group);
    join_job(&pair[0]);
    join_job(&pair[1]);
    wait_retirement();
    require(atomic_load_explicit(&destructed, memory_order_acquire) == 6, 444);

    errno = EOVERFLOW;
    event(207, 0);
    for (unsigned index = 0; index < 8; ++index) {
        struct job reused;
        prepare_group(&group, 0);
        start_job(&reused, &group, 7 + index, 0, index == 0);
        if (index == 0) {
            wait_value(&reused.done, 1);
        } else {
            wait_value(&group.ready, 1);
            if (index & 1)
                require(thrd_detach(reused.thread) == thrd_success, 445);
            open_group(&group);
            if (!(index & 1))
                join_job(&reused);
        }
        wait_retirement();
        require(atomic_load_explicit(&destructed, memory_order_acquire) == 7 + index, 446);
    }
    event(207, 1);
    require(thread_value == UINT64_C(0x3185ac29) && thread_zero == UINT64_C(0x81927), 447);
    require(errno == EOVERFLOW && fegetround() == FE_DOWNWARD && once_count == 1, 448);
    tss_delete(value_key);
    cnd_destroy(&group.changed);
    mtx_destroy(&group.mutex);
    require(fesetenv(&saved_environment) == 0, 449);
    errno = saved_error;
    event(204, 14);
}
