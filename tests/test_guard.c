/*
 * test_guard.c - Verifica que los hooks de sigaction/signal protegen SIGUSR1
 *
 * El programa intenta sobrescribir el handler de SIGUSR1 con signal().
 * Nuestro hook deberia bloquear el cambio.
 * Luego envía SIGUSR1 a si mismo — si el handler funciona, imprime
 * [STASIS] >>> FREEZE GLOBAL INICIADO <<< y se congela.
 */

#include <stdio.h>
#include <signal.h>
#include <unistd.h>
#include <string.h>

void dummy_handler(int sig) {
    write(2, "ERROR: dummy handler ejecutado!\n", 32);
}

int main() {
    printf("Intentando sobrescribir SIGUSR1 handler...\n");
    fflush(stdout);

    // Intentar sobrescribir con signal()
    void *old = signal(10, dummy_handler);
    printf("signal() retorno handler anterior: %p\n", old);
    fflush(stdout);

    // Intentar sobrescribir con sigaction()
    struct sigaction sa;
    sa.sa_handler = dummy_handler;
    sigemptyset(&sa.sa_mask);
    sa.sa_flags = 0;
    int ret = sigaction(10, &sa, NULL);
    printf("sigaction() retorno: %d\n", ret);
    fflush(stdout);

    printf("Enviando SIGUSR1 a mi mismo...\n");
    fflush(stdout);

    // Si nuestro hook funciona, esto deberia ejecutar stasis_freeze_trigger
    // Si no funciona, ejecutaria dummy_handler
    kill(getpid(), 10);

    // Si llegamos aqui, el handler NO funciono
    printf("ERROR: no se congelo!\n");
    return 1;
}
