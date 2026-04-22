/*
 * test_freeze.c - Test basico de freeze con SIGUSR2
 *
 * Uso:
 *   LD_PRELOAD=./target/release/liblibstasis.so ./test_freeze_bin
 *   Desde otra terminal: kill -12 <pid>
 */

#include <unistd.h>
#include <pthread.h>
#include <stdio.h>

void* thread_func(void* arg) {
    long id = (long)arg;
    while (1) {
        printf("Thread %ld corriendo...\n", id);
        fflush(stdout);
        usleep(500000);
    }
    return NULL;
}

int main() {
    pthread_t t1, t2, t3;

    printf("Creando 3 threads... PID=%d\n", getpid());
    fflush(stdout);

    pthread_create(&t1, NULL, thread_func, (void*)1);
    pthread_create(&t2, NULL, thread_func, (void*)2);
    pthread_create(&t3, NULL, thread_func, (void*)3);

    while (1) {
        printf("Main corriendo...\n");
        fflush(stdout);
        sleep(1);
    }

    return 0;
}
