#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <pthread.h>
#include <stdatomic.h>
#include <stdio.h>
#include <stdlib.h>

int pthread_create(pthread_t *thread, const pthread_attr_t *attr,
                   void *(*start_routine)(void *), void *arg) {
    static _Atomic unsigned count;
    const char *setting = getenv("MINASE_REVIEW_FAIL_PTHREAD_AT");
    unsigned nth = atomic_fetch_add(&count, 1) + 1;
    if (setting && nth == strtoul(setting, NULL, 10)) {
        fprintf(stderr, "review: injecting pthread_create EAGAIN at call %u\n", nth);
        return EAGAIN;
    }
    int (*actual)(pthread_t *, const pthread_attr_t *, void *(*)(void *), void *) =
        dlsym(RTLD_NEXT, "pthread_create");
    return actual(thread, attr, start_routine, arg);
}
