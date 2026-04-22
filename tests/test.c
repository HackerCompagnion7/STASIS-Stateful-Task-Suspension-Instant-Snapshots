#include <unistd.h>
#include <pthread.h>

void* thread_func(void* arg) {
    long id = (long)arg;
    char msg[64];
    // Escribir directamente sin snprintf para mantenerlo simple
    write(1, "Thread ejecutandose\n", 20);
    return (void*)id;
}

int main() {
    pthread_t threads[4];

    write(1, "Creando 4 threads...\n", 21);

    for (long i = 0; i < 4; i++) {
        pthread_create(&threads[i], 0, thread_func, (void*)i);
    }

    for (int i = 0; i < 4; i++) {
        pthread_join(threads[i], 0);
    }

    write(1, "Todos los threads terminaron\n", 30);
    write(2, "Esto es un error\n", 18);
    return 0;
}
