/*
 * test_guard.c - Verifica que los hooks protegen SIGUSR2
 *
 * El programa intenta sobrescribir el handler de SIGUSR2 con signal()
 * y sigaction(). Nuestro hook deberia bloquear el cambio.
 * Luego envia SIGUSR2 a si mismo - si funciona, se congela.
 */

#include <stdio.h>
#include <signal.h>
#include <unistd.h>
#include <string.h>

void dummy_handler(int sig) {
    write(2, "ERROR: dummy handler ejecutado!\n", 32);
}

int main() {
    printf("Intentando sobrescribir SIGUSR2 handler...\n");
    fflush(stdout);

    // Intentar sobrescribir con signal()
    void *old = signal(12, dummy_handler);
    printf("signal(12) retorno handler anterior: %p\n", old);
    fflush(stdout);

    // Intentar sobrescribir con sigaction()
    struct sigaction sa;
    sa.sa_handler = dummy_handler;
    sigemptyset(&sa.sa_mask);
    sa.sa_flags = 0;
    int ret = sigaction(12, &sa, NULL);
    printf("sigaction(12) retorno: %d\n", ret);
    fflush(stdout);

    printf("Enviando SIGUSR2 a mi mismo...\n");
    fflush(stdout);

    // Si nuestro hook funciona, se congela
    kill(getpid(), 12);

    printf("ERROR: no se congelo!\n");
    return 1;
}
