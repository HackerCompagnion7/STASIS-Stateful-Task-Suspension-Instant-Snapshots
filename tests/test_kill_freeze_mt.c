/*
 * test_kill_freeze_mt.c - Test freeze multithread via fork()
 *
 * Padre crea 3 threads, hijo envía SIGUSR1 tras 3s.
 * Todos los threads deberían congelarse.
 */

#include <unistd.h>
#include <stdio.h>
#include <stdlib.h>
#include <pthread.h>
#include <sys/wait.h>

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

    // Fork: hijo envía SIGUSR1 tras 3 segundos
    pid_t child = fork();
    if (child == 0) {
        sleep(3);
        printf("[HIJO] Enviando SIGUSR1 al padre PID=%d\n", getppid());
        kill(getppid(), 10);
        sleep(5);
        printf("[HIJO] Matando padre con kill -9\n");
        kill(getppid(), 9);
        _exit(0);
    }

    // Padre: loop hasta congelarse
    while (1) {
        printf("Main corriendo...\n");
        fflush(stdout);
        sleep(1);
    }

    return 0;
}
