/*
 * test_kill_freeze.c - Test freeze via kill() desde proceso hijo
 *
 * El padre corre con LD_PRELOAD. El hijo envia SIGUSR2 (senal 12)
 * al padre tras 3 segundos.
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
        sleep(3);
        printf("[HIJO] Enviando SIGUSR2 (kill -12) al padre PID=%d\n", getppid());
        kill(getppid(), 12);
        sleep(5);
        printf("[HIJO] Padre deberia estar congelado. Matando con kill -9\n");
        kill(getppid(), 9);
        _exit(0);
    }

    while (1) {
        printf("Padre corriendo... PID=%d\n", getpid());
        fflush(stdout);
        sleep(1);
    }

    return 0;
}
