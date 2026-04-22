#include <unistd.h>

int main() {
    write(1, "Hola desde el programa de prueba\n", 33);
    write(2, "Esto es un error\n", 18);
    return 0;
}
