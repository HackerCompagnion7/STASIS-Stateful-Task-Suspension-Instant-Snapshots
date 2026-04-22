#![no_std] // No queremos la librería estándar de Rust para evitar dependencias ocultas

use core::ffi::c_void;

// Definimos la constante de RTLD_NEXT de glibc/Bionic
const RTLD_NEXT: *const c_void = -1isize as *const c_void;

// ============================================================================
// Tipos de función para los hooks
// ============================================================================

type FnWrite = unsafe extern "C" fn(i32, *const c_void, usize) -> isize;

type FnPthreadCreate = unsafe extern "C" fn(
    *mut c_void,                          // thread (pthread_t*)
    *const c_void,                        // attr (pthread_attr_t*)
    extern "C" fn(*mut c_void) -> *mut c_void, // start_routine
    *mut c_void,                          // arg
) -> i32;

// ============================================================================
// Variables globales estáticas para guardar punteros a funciones reales
// ============================================================================

static mut REAL_WRITE: Option<FnWrite> = None;
static mut REAL_PTHREAD_CREATE: Option<FnPthreadCreate> = None;

// Flag atómico para prevenir recursión durante la inicialización
static mut IN_HOOK: bool = false;

// ============================================================================
// Syscall directa al kernel - evita cualquier función de C que cause recursión
// ============================================================================
//
// x86_64 Linux:  syscall __NR_write = 1
//   rax = número de syscall, rdi = fd, rsi = buf, rdx = count
//
// ARM64 Android (Bionic): syscall __NR_write = 64
//   x8 = número de syscall, x0 = fd, x1 = buf, x2 = count
// ============================================================================

#[cfg(target_arch = "x86_64")]
#[unsafe(naked)]
unsafe extern "C" fn raw_syscall_write(fd: i32, buf: *const u8, len: usize) -> isize {
    core::arch::naked_asm!(
        "mov rax, 1",       // __NR_write = 1 en x86_64
        "syscall",
        "ret",
    )
}

#[cfg(target_arch = "aarch64")]
#[unsafe(naked)]
unsafe extern "C" fn raw_syscall_write(fd: i32, buf: *const u8, len: usize) -> isize {
    core::arch::naked_asm!(
        "mov x8, #64",      // __NR_write = 64 en ARM64
        "svc #0",
        "ret",
    )
}

// ============================================================================
// Helper: escribir string estático a stderr via syscall directa
// ============================================================================
unsafe fn log_raw(msg: &[u8]) {
    raw_syscall_write(2, msg.as_ptr(), msg.len());
}

// ============================================================================
// Importar dlsym desde libdl - funciona tanto en glibc como en Bionic
// ============================================================================
extern "C" {
    fn dlsym(handle: *const c_void, symbol: *const u8) -> *const c_void;
}

// ============================================================================
// Constructor de la librería - Se ejecuta ANTES que el main() del programa
// ============================================================================
#[no_mangle]
pub unsafe extern "C" fn stasis_init() {
    // Resolver write() original
    let real_write_ptr = dlsym(RTLD_NEXT, b"write\0".as_ptr());
    if real_write_ptr.is_null() {
        log_raw(b"[STASIS FATAL] dlsym(write) failed\n");
        return;
    }
    REAL_WRITE = Some(core::mem::transmute::<*const c_void, FnWrite>(real_write_ptr));

    // Resolver pthread_create() original
    let real_pthread_ptr = dlsym(RTLD_NEXT, b"pthread_create\0".as_ptr());
    if real_pthread_ptr.is_null() {
        log_raw(b"[STASIS WARN] dlsym(pthread_create) failed\n");
    } else {
        REAL_PTHREAD_CREATE = Some(core::mem::transmute::<*const c_void, FnPthreadCreate>(real_pthread_ptr));
    }

    log_raw(b"[STASIS] Hook activo. Interceptando write() + pthread_create()\n");
}

// Atributo para asegurar que el linker llame a nuestro init antes de main()
#[link_section = ".init_array"]
#[used]
static INIT_ARRAY: unsafe extern "C" fn() = stasis_init;

// ============================================================================
// Hook de la función write - Intercepta stdout, pasa todo lo demás
// ============================================================================
#[no_mangle]
pub unsafe extern "C" fn write(fd: i32, buf: *const c_void, count: usize) -> isize {
    // Solo interceptamos stdout (fd 1) para no causar tormentas de logs
    if fd == 1 && !IN_HOOK {
        IN_HOOK = true;
        log_raw(b"[STASIS HOOK] Capturado write en stdout\n");
        IN_HOOK = false;
    }

    // Llamamos a la función REAL de glibc/Bionic
    match REAL_WRITE {
        Some(func) => func(fd, buf, count),
        None => raw_syscall_write(fd, buf as *const u8, count), // Fallback crudo
    }
}

// ============================================================================
// Hook de pthread_create - Registra todos los threads del proceso
// ============================================================================
//
// Este es el hook que define si STASIS vive o muere.
// Si podemos interceptar la creación de threads:
//   - Podemos ver TODOS los threads del proceso
//   - Podemos enviar señales a todos
//   - Podemos coordinar "stop-the-world"
//
#[no_mangle]
pub unsafe extern "C" fn pthread_create(
    thread: *mut c_void,
    attr: *const c_void,
    start_routine: extern "C" fn(*mut c_void) -> *mut c_void,
    arg: *mut c_void,
) -> i32 {
    // Log via syscall directa - cero riesgo de recursión
    log_raw(b"[STASIS] pthread_create interceptado\n");

    // Passthrough a la función real de glibc/Bionic
    match REAL_PTHREAD_CREATE {
        Some(func) => func(thread, attr, start_routine, arg),
        None => {
            log_raw(b"[STASIS FATAL] pthread_create real no disponible\n");
            -1
        }
    }
}

// ============================================================================
// Panic handler mínimo para no depender de std
// ============================================================================
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    unsafe { log_raw(b"[STASIS PANIC]\n") };
    loop {}
}
