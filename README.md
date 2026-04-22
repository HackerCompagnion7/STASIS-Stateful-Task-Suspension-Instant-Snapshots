# STASIS — Stateful Task Suspension & Instant Snapshots

**Paso 0 completado:** Intercepción limpia de syscalls en espacio de usuario sin root mediante `LD_PRELOAD`.

## ¿Qué hace?

`libstasis.so` se inyecta vía `LD_PRELOAD` en cualquier proceso y hookea la syscall `write` de forma transparente:

- Intercepta llamadas a `write()` en stdout (fd 1)
- Loguea la intercepción por stderr usando syscall directa al kernel (cero recursión)
- Pasa la llamada original a la función real de glibc/Bionic
- Se inicializa antes del `main()` del programa objetivo via `.init_array`

## Resultado verificado (Escenario C)

```text
[STASIS] Hook activo. Interceptando write()
[STASIS HOOK] Capturado write en stdout
Hola desde el programa de prueba
Esto es un error
```

## Compilar

### Linux x86_64

```bash
cargo build --release
```

### Android ARM64 (Termux)

```bash
cargo build --release
```

El código detecta la arquitectura automáticamente via `#[cfg(target_arch)]` y usa el ASM correcto.

## Probar

```bash
# Compilar binario de prueba
gcc tests/test.c -o test_bin

# Ejecutar inyectando la librería
LD_PRELOAD=./target/release/liblibstasis.so ./test_bin
```

## Arquitectura

```
┌──────────────────────────────────────────────┐
│  Programa objetivo                           │
│  write(1, "datos", len)                      │
└──────────────┬───────────────────────────────┘
               │ LD_PRELOAD intercepta
               ▼
┌──────────────────────────────────────────────┐
│  libstasis.so                                │
│  ├── .init_array → stasis_init()             │
│  │   └── dlsym(RTLD_NEXT, "write") → ptr    │
│  ├── write() hook                            │
│  │   ├── Si fd==1: log via raw syscall       │
│  │   └── Passthrough a write() real          │
│  └── raw_syscall_write()                     │
│      └── syscall directa al kernel           │
│          (sin pasar por libc = sin recursión) │
└──────────────────────────────────────────────┘
```

## Decisiones técnicas

| Decisión | Razón |
|----------|-------|
| `#![no_std]` | Eliminar dependencias ocultas de libc que causen recursión |
| Syscall directa via `naked_asm!` | Evitar llamar `write` dentro del hook (recursión infinita) |
| `dlsym(RTLD_NEXT)` vía `extern "C"` | Funciona en glibc y Bionic sin hacks de ASM |
| Flag `IN_HOOK` anti-recursión | Protección si algo en el path de inicialización llama `write` |
| `.init_array` | Ejecución garantizada antes de `main()` |
| `panic = "abort"` | `no_std` no soporta unwinding |

## Stack

- Rust 1.88+ (estable) — `#[unsafe(naked)]` y `naked_asm!` son estables
- Zero dependencias de crates
- Compatible: Linux x86_64 / Android ARM64
