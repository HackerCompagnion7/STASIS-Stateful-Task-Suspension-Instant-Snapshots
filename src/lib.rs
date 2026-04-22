#![no_std] // No queremos la librería estándar de Rust para evitar dependencias ocultas

use core::ffi::c_void;

// Definimos la constante de RTLD_NEXT de glibc/Bionic
const RTLD_NEXT: *const c_void = -1isize as *const c_void;

// Estructura para mapear la firma de la función write de C
type FnWrite = unsafe extern "C" fn(i32, *const c_void, usize) -> isize;

// Variable global estática para guardar el puntero a la función real
static mut REAL_WRITE: Option<FnWrite> = None;

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

// Importar dlsym desde libdl - funciona tanto en glibc como en Bionic
extern "C" {
    fn dlsym(handle: *const c_void, symbol: *const u8) -> *const c_void;
}

// ============================================================================
// Constructor de la librería - Se ejecuta ANTES que el main() del programa
// ============================================================================
#[no_mangle]
pub unsafe extern "C" fn stasis_init() {
    // Buscamos la dirección de la función 'write' original en glibc/Bionic
    let real_write_ptr = dlsym(RTLD_NEXT, b"write\0".as_ptr());

    if real_write_ptr.is_null() {
        let err = b"[STASIS FATAL] dlsym failed\n";
        raw_syscall_write(2, err.as_ptr(), err.len());
        return;
    }

    REAL_WRITE = Some(core::mem::transmute::<*const c_void, FnWrite>(real_write_ptr));

    let msg = b"[STASIS] Hook activo. Interceptando write()\n";
    raw_syscall_write(2, msg.as_ptr(), msg.len());
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
        let msg = b"[STASIS HOOK] Capturado write en stdout\n";
        raw_syscall_write(2, msg.as_ptr(), msg.len());
        IN_HOOK = false;
    }

    // Llamamos a la función REAL de glibc/Bionic
    match REAL_WRITE {
        Some(func) => func(fd, buf, count),
        None => raw_syscall_write(fd, buf as *const u8, count), // Fallback crudo
    }
}

// Panic handler mínimo para no depender de std
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    let msg = b"[STASIS PANIC]\n";
    unsafe { raw_syscall_write(2, msg.as_ptr(), msg.len()) };
    loop {}
}
