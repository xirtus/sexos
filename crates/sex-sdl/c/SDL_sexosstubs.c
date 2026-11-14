/* SPDX-License-Identifier: GPL-3.0-only */
/* What SDL's pthread backend asks of POSIX beyond sex-libc: signal masks and
 * scheduling. Nothing sends a SexOS program signals, and its workers have one
 * priority. These answer for SDL alone, here in the SDL library. */
#include <pthread.h>
#include <sched.h>
#include <signal.h>
#include <string.h>

int sigemptyset(sigset_t *set)
{
    memset(set, 0, sizeof(*set));
    return 0;
}

int sigaddset(sigset_t *set, int sig)
{
    (void)set;
    (void)sig;
    return 0;
}

int pthread_sigmask(int how, const sigset_t *set, sigset_t *old)
{
    (void)how;
    (void)set;
    if (old) {
        memset(old, 0, sizeof(*old));
    }
    return 0;
}

int pthread_setcanceltype(int type, int *old)
{
    (void)type;
    if (old) {
        *old = 0;
    }
    return 0;
}

int pthread_getschedparam(pthread_t thread, int *policy, struct sched_param *param)
{
    (void)thread;
    *policy = SCHED_OTHER;
    memset(param, 0, sizeof(*param));
    return 0;
}

int pthread_setschedparam(pthread_t thread, int policy, const struct sched_param *param)
{
    (void)thread;
    (void)policy;
    (void)param;
    return 0;
}

int sched_get_priority_min(int policy)
{
    (void)policy;
    return 0;
}

int sched_get_priority_max(int policy)
{
    (void)policy;
    return 0;
}
