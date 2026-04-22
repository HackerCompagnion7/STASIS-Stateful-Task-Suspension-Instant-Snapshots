/*
 * test_freeze_auto.c - STASIS freeze test con auto-trigger
 *
 * El proceso se congela automaticamente tras 3 segundos
 * enviandose SIGUSR2 (senal 12) a si mismo.
 *
 * Uso:
 *   LD_PRELOAD=./target/release/liblibstasis.so ./test_freeze_auto_bin
 *
 * Resultado esperado:
 *   - Imprime "corriendo..." durante 3 segundos
 *   - [STASIS] >>> FREEZE GLOBAL INICIADO <<<
 *   - [STASIS FREEZE] Thread congelado  (por cada thread)
 *   - Proceso detenido, CPU ~0%
 *   - Mata con: kill -9 <pid>
 */

#include <unistd.h>
#include <pthread.h>
#include <stdio.h>
#include <signal.h>

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

    printf("Creando 3 threads...\n");
    fflush(stdout);

    pthread_create(&t1, NULL, thread_func, (void*)1);
    pthread_create(&t2, NULL, thread_func, (void*)2);
    pthread_create(&t3, NULL, thread_func, (void*)3);

    for (int i = 0; i < 3; i++) {
        printf("Main corriendo... (%d/3)\n", i + 1);
        fflush(stdout);
        sleep(1);
    }

    printf(">>> Auto-freeze en 3... 2... 1... <<<\n");
    fflush(stdout);

    // Enviarse SIGUSR2 (senal 12) = trigger freeze global
    kill(getpid(), 12);

    // Nunca deberia llegar aqui
    printf("ERROR: no se congelo!\n");
    return 1;
}
