/*
 * test_kill_freeze.c - Test freeze via kill() desde proceso hijo
 *
 * El padre corre con LD_PRELOAD (handler instalado por stasis_init).
 * El hijo envía SIGUSR1 (senal 10) al padre tras 3 segundos.
 * Si el handler funciona → el padre se congela (CPU~0%).
 * Si no funciona → el padre sigue imprimiendo.
 */

#include <unistd.h>
#include <stdio.h>
#include <stdlib.h>
#include <signal.h>
#include <sys/wait.h>

int main() {
    pid_t pid = fork();

    if (pid < 0) {
        perror("fork");
        return 1;
    }

    if (pid == 0) {
        // HIJO: esperar 3s, enviar SIGUSR1 al padre, esperar, matar
        sleep(3);
        printf("[HIJO] Enviando SIGUSR1 (kill -10) al padre PID=%d\n", getppid());
        kill(getppid(), 10);
        sleep(5);
        printf("[HIJO] Padre deberia estar congelado. Matando con kill -9\n");
        kill(getppid(), 9);
        _exit(0);
    }

    // PADRE: correr hasta que llegue la senal
    while (1) {
        printf("Padre corriendo... PID=%d\n", getpid());
        fflush(stdout);
        sleep(1);
    }

    return 0;
}
