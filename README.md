# STASIS — Stateful Task Suspension & Instant Snapshots

Librería Rust `#![no_std]` inyectable vía `LD_PRELOAD` para congelar procesos en Android (ARM64) sin root.

---

## Estado Actual

| Componente | Estado | Notas |
|---|---|---|
| Hook `write()` | ✅ Funciona | Intercepta stdout, log via syscall directa |
| Hook `pthread_create()` | ✅ Funciona | Registra threads, captura TIDs via gettid() |
| Hook `sigaction()` / `signal()` | ✅ Funciona | Bloquea sobrescritura de handlers |
| Thread tracking | ✅ Funciona | Hasta 128 threads, TIDs capturados |
| **Signal handler SIGUSR2** | ❌ **BLOQUEADO** | Ver abajo |
| Freeze global | ❌ Depende del handler | Funciona en x86_64, no en Bionic |
| Unfreeze | ❌ No implementado | |
| Snapshot de memoria | ❌ No implementado | |

---

## 🚧 Bloqueo Crítico: Signal Handlers en Bionic

### Síntoma
```
[STASIS] Handler SIGUSR2 instalado OK        ← sigaction() retornó 0 (éxito)
[STASIS FATAL] SIGUSR2 handler no coincide   ← pero al leerlo de vuelta, NO es nuestro
[STASIS FATAL] SIGUSR2 handler IMPOSIBLE de instalar
...
User signal 2                                ← señal recibida con handler DEFAULT → proceso muere
```

### Qué significa
- `sigaction()` retorna éxito (0) pero **el handler no se instala realmente**
- Ni SIGUSR1 ni SIGUSR2 funcionan en Bionic/Android
- En x86_64 con glibc funciona perfectamente

### Causas probables (en orden de posibilidad)

1. **Struct `sigaction` incorrecto para Bionic arm64**
   - Nuestro struct incluye `sa_restorer` para aarch64, pero Bionic arm64
     puede NO tener ese campo (el kernel arm64 usa vDSO para signal return)
   - Si el struct es más grande de lo esperado, Bionic lee basura
   - **Fix**: Verificar el struct `sigaction` exacto de Bionic arm64
     (`/bionic/libc/kernel/uapi/asm-arm64/asm/signal.h`)

2. **Bionic no permite instalar handlers desde LD_PRELOAD**
   - El runtime de Android podría ignorar handlers instalados fuera del
     proceso principal (sandbox de señales)
   - **Fix**: Probar instalación lazy (después de `main()`, no en `.init_array`)

3. **Bionic envuelve sigaction internamente**
   - Podría instalar sus propios wrappers que traducen/validan handlers
   - Nuestro `dlsym(RTLD_NEXT, "sigaction")` resuelve el wrapper, no el syscall
   - **Fix**: Usar raw syscall `rt_sigaction` (nr 134 en arm64) directamente,
     incluyendo un `sa_restorer` válido apuntando al vDSO `__kernel_rt_sigreturn`

### Cómo diagnosticar

```bash
# En Termux, verificar el tamaño real de struct sigaction:
cat > /tmp/test_size.c << 'EOF'
#include <signal.h>
#include <stdio.h>
int main() {
    printf("sizeof(struct sigaction) = %zu\n", sizeof(struct sigaction));
    printf("offset sa_handler = %zu\n", offsetof(struct sigaction, sa_handler));
    printf("offset sa_mask = %zu\n", offsetof(struct sigaction, sa_mask));
    printf("offset sa_flags = %zu\n", offsetof(struct sigaction, sa_flags));
    return 0;
}
EOF
clang /tmp/test_size.c -o /tmp/test_size && /tmp/test_size
```

### Alternativas si sigaction no funciona

1. **Raw syscall `rt_sigaction`** — hablar directo al kernel, sin pasar por Bionic
   - Necesita un `sa_restorer` válido en arm64 (vDSO `__kernel_rt_sigreturn`)
   - Más complejo pero bypasa cualquier wrapper de Bionic

2. **Señal en tiempo real (`SIGRTMIN+1`)** — menos probabilidad de conflicto
   - `SIGRTMIN` en Bionic suele ser 32 o 36
   - Probar con señal 36 o 37

3. **`/proc/self/mem` + `ptrace`** — enfoque completamente diferente
   - Escribir directamente en memoria del proceso para inyectar un breakpoint
   - Más invasivo pero no depende de señales

4. **Futex-based freeze** — usar `futex(FUTEX_WAIT)` en vez de señales
   - Los threads hacen pause en un futex compartido
   - Se activa escribiendo el futex desde fuera
   - No requiere signal handlers

---

## Lo que funciona (x86_64 / glibc)

```
$ LD_PRELOAD=./target/release/liblibstasis.so ./test_auto_bin
[STASIS] Handler SIGUSR2 instalado OK
[STASIS] SIGUSR2 handler verificado OK
[STASIS] Hook activo. kill -12 <pid> = freeze all
Creando 3 threads...
Thread 1 corriendo...
Thread 2 corriendo...
Thread 3 corriendo...
Main corriendo... (3/3)
>>> Auto-freeze en 3... 2... 1... <<<
[STASIS] Signal recibida
[STASIS] >>> FREEZE GLOBAL INICIADO <<<
[STASIS] Signal recibida
[STASIS FREEZE] Thread congelado
[STASIS FREEZE] Thread congelado
[STASIS FREEZE] Thread congelado
(--- proceso congelado, CPU ~0% ---)
```

---

## Arquitectura

```
┌─────────────────────────────────────────────────────┐
│  Programa objetivo                                  │
│  write() / pthread_create() / sigaction()           │
└──────────┬──────────────────────────────────────────┘
           │ LD_PRELOAD intercepta
           ▼
┌─────────────────────────────────────────────────────┐
│  libstasis.so  (#![no_std], cdylib)                 │
│                                                     │
│  .init_array → stasis_init()                        │
│    ├── raw_getpid() → STASIS_PID                    │
│    ├── raw_gettid() → THREAD_TIDS[0]               │
│    ├── dlsym(RTLD_NEXT, "write")      → REAL_WRITE  │
│    ├── dlsym(RTLD_NEXT, "pthread_create") → REAL_PC │
│    ├── dlsym(RTLD_NEXT, "sigaction")   → REAL_SA    │
│    └── install_signal_handler(SIGUSR2, handler)     │
│                                                     │
│  Hooks:                                             │
│    write()        → intercepta fd==1, passthrough   │
│    pthread_create() → wrapper captura TID           │
│    sigaction()   → bloquea cambios a SIGUSR2        │
│    signal()      → bloquea cambios a SIGUSR2        │
│                                                     │
│  Handler (solo raw syscalls):                       │
│    stasis_freeze_handler(SIGUSR2):                  │
│      1. Primer thread: broadcast SIGUSR2 a TIDs     │
│      2. Todos: nanosleep loop (CPU ~0%)             │
└─────────────────────────────────────────────────────┘
```

---

## Compilar

### Linux x86_64
```bash
cargo build --release
```

### Android ARM64 (Termux)
```bash
cargo build --release
# Output: target/release/liblibstasis.so
```

### Tests
```bash
# Auto-freeze (se congela solo a los 3s)
clang tests/test_freeze_auto.c -o test_auto_bin -lpthread
LD_PRELOAD=./target/release/liblibstasis.so ./test_auto_bin

# Freeze via kill externo (fork-based)
clang tests/test_kill_freeze_mt.c -o test_kill_mt_bin -lpthread
LD_PRELOAD=./target/release/liblibstasis.so ./test_kill_mt_bin

# Verificar que signal/sigaction están protegidos
clang tests/test_guard.c -o test_guard_bin
LD_PRELOAD=./target/release/liblibstasis.so ./test_guard_bin

# Manual (kill -12 desde otra terminal)
clang tests/test_freeze.c -o test_manual_bin -lpthread
LD_PRELOAD=./target/release/liblibstasis.so ./test_manual_bin
# Otra terminal: kill -12 <pid>
```

---

## Roadmap — Lo que falta para ser funcional

### Fase 1: Signal handlers en Bionic ❌ (BLOQUEO ACTUAL)
- [ ] Diagnosticar struct `sigaction` exacto de Bionic arm64
- [ ] Probar raw syscall `rt_sigaction` con `sa_restorer` del vDSO
- [ ] Probar señales en tiempo real (SIGRTMIN+1) como alternativa
- [ ] Si nada funciona: evaluar enfoque futex-based freeze

### Fase 2: Freeze real
- [ ] Handler funcional en Bionic (depende de Fase 1)
- [ ] Nanosleep loop → reemplazar con `futex(FUTEX_WAIT)` para CPU 0%
- [ ] Verificar que el freeze no corrompe estado

### Fase 3: Unfreeze
- [ ] Mecanismo para despertar threads congelados
- [ ] `futex(FUTEX_WAKE)` o señal de unfreeze
- [ ] Verificar que threads reanudan correctamente

### Fase 4: Snapshot
- [ ] Leer `/proc/<pid>/maps` para obtener regiones de memoria
- [ ] Dump de memoria via `/proc/<pid>/mem`
- [ ] Guardar registros via `ptrace(PTRACE_GETREGS)`
- [ ] Serializar a archivo

### Fase 5: Restore
- [ ] Restaurar memoria desde snapshot
- [ ] Restaurar registros
- [ ] Re-crear threads si es necesario

---

## Decisiones técnicas

| Decisión | Razón |
|---|---|
| `#![no_std]` | Sin dependencias ocultas de libc, sin recursión |
| Syscalls directas via `naked_asm!` | Evitar llamar libc dentro de hooks |
| `dlsym(RTLD_NEXT)` via `extern "C"` | Funciona en glibc y Bionic |
| `AtomicBool` para IN_HOOK y FREEZE_BROADCAST_DONE | Thread-safe sin mutex |
| `.init_array` | Ejecución antes de `main()` |
| `panic = "abort"` | `no_std` no soporta unwinding |
| Solo SIGUSR2 | SIGUSR1 secuestrado por Bionic |
| Hook de `sigaction`/`signal` | Proteger handler contra sobrescritura |
| Bionic `sa_mask` = 8 bytes vs glibc 128 bytes | Layout diferente por libc |

## Stack

- Rust 1.88+ (estable)
- Zero dependencias de crates
- Compatible: Linux x86_64 (verificado) / Android ARM64 (bloqueado en signal handlers)
