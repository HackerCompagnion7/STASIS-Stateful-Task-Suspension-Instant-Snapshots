#![no_std] // Sin librería estándar para evitar dependencias ocultas

use core::ffi::c_void;

// ============================================================================
// Constantes
// ============================================================================

const RTLD_NEXT: *const c_void = -1isize as *const c_void;
const MAX_THREADS: usize = 128;
const SIGUSR1: i32 = 10; // Trigger: freeze all
const SIGUSR2: i32 = 12; // Handler: freeze este thread

// ============================================================================
// Tipos de función para los hooks
// ============================================================================

type FnWrite = unsafe extern "C" fn(i32, *const c_void, usize) -> isize;
type FnPthreadCreate = unsafe extern "C" fn(
    *mut c_void,
    *const c_void,
    extern "C" fn(*mut c_void) -> *mut c_void,
    *mut c_void,
) -> i32;

// ============================================================================
// Estado global
// ============================================================================

static mut REAL_WRITE: Option<FnWrite> = None;
static mut REAL_PTHREAD_CREATE: Option<FnPthreadCreate> = None;
static mut IN_HOOK: bool = false;

// Thread tracking
static mut SLOT_ROUTINES: [*const c_void; MAX_THREADS] = [core::ptr::null(); MAX_THREADS];
static mut SLOT_ARGS: [*mut c_void; MAX_THREADS] = [core::ptr::null_mut(); MAX_THREADS];
static mut SLOT_ACTIVE: [bool; MAX_THREADS] = [false; MAX_THREADS];
static mut THREAD_TIDS: [i32; MAX_THREADS] = [0; MAX_THREADS];
static mut THREAD_COUNT: usize = 0;
static mut STASIS_PID: i32 = 0;

// ============================================================================
// raw_syscall_write - Naked, ya funciona en ambas arquitecturas
// ============================================================================

#[cfg(target_arch = "x86_64")]
#[unsafe(naked)]
unsafe extern "C" fn raw_syscall_write(fd: i32, buf: *const u8, len: usize) -> isize {
    core::arch::naked_asm!(
        "mov rax, 1",
        "syscall",
        "ret",
    )
}

#[cfg(target_arch = "aarch64")]
#[unsafe(naked)]
unsafe extern "C" fn raw_syscall_write(fd: i32, buf: *const u8, len: usize) -> isize {
    core::arch::naked_asm!(
        "mov x8, #64",
        "svc #0",
        "ret",
    )
}

// ============================================================================
// Generic syscall wrappers via inline asm
// ============================================================================

#[cfg(target_arch = "x86_64")]
unsafe fn syscall0(nr: i64) -> i64 {
    let ret: i64;
    core::arch::asm!(
        "syscall",
        inlateout("rax") nr => ret,
        lateout("rcx") _,
        lateout("r11") _,
        options(nostack)
    );
    ret
}

#[cfg(target_arch = "x86_64")]
unsafe fn syscall2(nr: i64, a1: i64, a2: i64) -> i64 {
    let ret: i64;
    core::arch::asm!(
        "syscall",
        inlateout("rax") nr => ret,
        in("rdi") a1,
        in("rsi") a2,
        lateout("rcx") _,
        lateout("r11") _,
        options(nostack)
    );
    ret
}

#[cfg(target_arch = "x86_64")]
unsafe fn syscall3(nr: i64, a1: i64, a2: i64, a3: i64) -> i64 {
    let ret: i64;
    core::arch::asm!(
        "syscall",
        inlateout("rax") nr => ret,
        in("rdi") a1,
        in("rsi") a2,
        in("rdx") a3,
        lateout("rcx") _,
        lateout("r11") _,
        options(nostack)
    );
    ret
}

#[cfg(target_arch = "x86_64")]
unsafe fn syscall4(nr: i64, a1: i64, a2: i64, a3: i64, a4: i64) -> i64 {
    let ret: i64;
    core::arch::asm!(
        "syscall",
        inlateout("rax") nr => ret,
        in("rdi") a1,
        in("rsi") a2,
        in("rdx") a3,
        in("r10") a4,
        lateout("rcx") _,
        lateout("r11") _,
        options(nostack)
    );
    ret
}

#[cfg(target_arch = "aarch64")]
unsafe fn syscall0(nr: i64) -> i64 {
    let ret: i64;
    core::arch::asm!(
        "svc #0",
        in("x8") nr,
        out("x0") ret,
        options(nostack)
    );
    ret
}

#[cfg(target_arch = "aarch64")]
unsafe fn syscall2(nr: i64, a1: i64, a2: i64) -> i64 {
    let ret: i64;
    core::arch::asm!(
        "svc #0",
        in("x8") nr,
        inlateout("x0") a1 => ret,
        in("x1") a2,
        options(nostack)
    );
    ret
}

#[cfg(target_arch = "aarch64")]
unsafe fn syscall3(nr: i64, a1: i64, a2: i64, a3: i64) -> i64 {
    let ret: i64;
    core::arch::asm!(
        "svc #0",
        in("x8") nr,
        inlateout("x0") a1 => ret,
        in("x1") a2,
        in("x2") a3,
        options(nostack)
    );
    ret
}

#[cfg(target_arch = "aarch64")]
unsafe fn syscall4(nr: i64, a1: i64, a2: i64, a3: i64, a4: i64) -> i64 {
    let ret: i64;
    core::arch::asm!(
        "svc #0",
        in("x8") nr,
        inlateout("x0") a1 => ret,
        in("x1") a2,
        in("x2") a3,
        in("x3") a4,
        options(nostack)
    );
    ret
}

// ============================================================================
// Syscall helpers específicos por arquitectura
// ============================================================================
//
// gettid:    x86_64=186, aarch64=178
// getpid:    x86_64=39,  aarch64=172
// tgkill:    x86_64=234, aarch64=131
// nanosleep: x86_64=35,  aarch64=101
// rt_sigaction: x86_64=13, aarch64=134
// ============================================================================

#[cfg(target_arch = "x86_64")]
unsafe fn raw_gettid() -> i32 { syscall0(186) as i32 }
#[cfg(target_arch = "aarch64")]
unsafe fn raw_gettid() -> i32 { syscall0(178) as i32 }

#[cfg(target_arch = "x86_64")]
unsafe fn raw_getpid() -> i32 { syscall0(39) as i32 }
#[cfg(target_arch = "aarch64")]
unsafe fn raw_getpid() -> i32 { syscall0(172) as i32 }

#[cfg(target_arch = "x86_64")]
unsafe fn raw_tgkill(tgid: i32, tid: i32, sig: i32) -> i32 {
    syscall3(234, tgid as i64, tid as i64, sig as i64) as i32
}
#[cfg(target_arch = "aarch64")]
unsafe fn raw_tgkill(tgid: i32, tid: i32, sig: i32) -> i32 {
    syscall3(131, tgid as i64, tid as i64, sig as i64) as i32
}

// nanosleep - duerme 1 hora, CPU casi 0
#[repr(C)]
struct Timespec {
    tv_sec: i64,
    tv_nsec: i64,
}

static SLEEP_SPEC: Timespec = Timespec { tv_sec: 3600, tv_nsec: 0 };

#[cfg(target_arch = "x86_64")]
unsafe fn raw_nanosleep() -> i32 {
    syscall2(35, &SLEEP_SPEC as *const _ as i64, 0) as i32
}
#[cfg(target_arch = "aarch64")]
unsafe fn raw_nanosleep() -> i32 {
    syscall2(101, &SLEEP_SPEC as *const _ as i64, 0) as i32
}

// ============================================================================
// Signal handler installation via glibc sigaction
// ============================================================================
//
// glibc's sigaction() traduce los layouts internamente y maneja
// sa_restorer automaticamente. Mas fiable que el raw syscall para
// la instalacion del handler.
//
// Layout de glibc sigaction en x86_64/aarch64:
//   sa_handler:  union (8 bytes)
//   sa_mask:     sigset_t (128 bytes)
//   sa_flags:    i32 (4 bytes + 4 padding)
//   sa_restorer: *const c_void (8 bytes, solo x86_64)
// ============================================================================

#[repr(C)]
struct GlibcSigaction {
    sa_handler: *const c_void,
    sa_mask: [u64; 16], // 128 bytes, glibc sigset_t
    sa_flags: i32,
    #[cfg(target_arch = "x86_64")]
    _pad: i32,          // padding antes de sa_restorer
    #[cfg(target_arch = "x86_64")]
    sa_restorer: *const c_void,
}

const SA_RESTART: i32 = 0x10000000;

extern "C" {
    fn sigaction(sig: i32, act: *const GlibcSigaction, oldact: *mut GlibcSigaction) -> i32;
}

unsafe fn install_signal_handler(sig: i32, handler: *const c_void) -> i32 {
    let act = GlibcSigaction {
        sa_handler: handler,
        sa_mask: [0; 16],
        sa_flags: SA_RESTART,
        #[cfg(target_arch = "x86_64")]
        _pad: 0,
        #[cfg(target_arch = "x86_64")]
        sa_restorer: core::ptr::null(), // glibc lo rellena automaticamente
    };
    sigaction(sig, &act, core::ptr::null_mut())
}

// ============================================================================
// Helper: log a stderr via syscall directa
// ============================================================================

unsafe fn log_raw(msg: &[u8]) {
    raw_syscall_write(2, msg.as_ptr(), msg.len());
}

// ============================================================================
// Importar funciones de libc (solo dlsym, nada de signal)
// ============================================================================

extern "C" {
    fn dlsym(handle: *const c_void, symbol: *const u8) -> *const c_void;
}

// ============================================================================
// Signal handlers
// ============================================================================
//
// Estos handlers se ejecutan en el stack del thread que recibe la señal.
// Solo usan syscalls directas (raw_syscall_write, raw_nanosleep, raw_tgkill).
// Cero libc, cero recursión, cero riesgo.
// ============================================================================

// SIGUSR2 (señal 12): congela ESTE thread
// Cada thread que recibe SIGUSR2 entra en un loop de nanosleep.
// CPU cae a casi 0. Thread congelado.
#[unsafe(no_mangle)]
unsafe extern "C" fn stasis_freeze_handler(sig: i32) {
    // Log signal number para debugging
    if sig == SIGUSR2 {
        log_raw(b"[STASIS FREEZE] SIGUSR2 - Thread congelado\n");
    } else {
        log_raw(b"[STASIS FREEZE] Signal desconocido - Thread congelado\n");
    }
    loop {
        raw_nanosleep();
    }
}

// SIGUSR1 (señal 10): broadcast SIGUSR2 a TODOS los threads conocidos
// Luego congela este thread también. Resultado: freeze global.
#[unsafe(no_mangle)]
unsafe extern "C" fn stasis_freeze_trigger(_: i32) {
    log_raw(b"[STASIS] >>> FREEZE GLOBAL INICIADO <<<\n");

    // Enviar SIGUSR2 a todos los TIDs registrados
    let count = THREAD_COUNT;
    let pid = STASIS_PID;

    // Log cuántos threads estamos congelando
    if count == 0 {
        log_raw(b"[STASIS WARN] No hay threads registrados\n");
    } else if count == 1 {
        log_raw(b"[STASIS] Congelando 1 thread\n");
    } else if count == 2 {
        log_raw(b"[STASIS] Congelando 2 threads\n");
    } else if count == 3 {
        log_raw(b"[STASIS] Congelando 3 threads\n");
    } else if count == 4 {
        log_raw(b"[STASIS] Congelando 4 threads\n");
    } else {
        log_raw(b"[STASIS] Congelando N threads\n");
    }

    for i in 0..count {
        let tid = THREAD_TIDS[i];
        if tid > 0 && tid != raw_gettid() {
            // No enviar señal al thread actual, se congela al final
            let ret = raw_tgkill(pid, tid, SIGUSR2);
            if ret != 0 {
                log_raw(b"[STASIS WARN] tgkill fallo\n");
            }
        }
    }

    // Pequeña pausa para que los otros threads entren al handler
    raw_nanosleep();

    // Congelar este thread también
    log_raw(b"[STASIS FREEZE] Thread trigger congelado\n");
    loop {
        raw_nanosleep();
    }
}

// ============================================================================
// Thread wrapper - captura TIDs via gettid()
// ============================================================================

extern "C" fn stasis_thread_wrapper(slot_idx_ptr: *mut c_void) -> *mut c_void {
    let slot_idx = slot_idx_ptr as usize;

    unsafe {
        // Obtener TID del kernel
        let tid = raw_gettid();

        // Almacenar TID
        if THREAD_COUNT < MAX_THREADS {
            THREAD_TIDS[THREAD_COUNT] = tid;
            THREAD_COUNT += 1;
        }

        log_raw(b"[STASIS] Thread registrado (TID capturado)\n");

        // Leer y liberar el slot
        let routine_ptr = SLOT_ROUTINES[slot_idx];
        let arg = SLOT_ARGS[slot_idx];
        SLOT_ACTIVE[slot_idx] = false;

        // Llamar la start_routine original con el arg original
        if !routine_ptr.is_null() {
            let routine: extern "C" fn(*mut c_void) -> *mut c_void =
                core::mem::transmute(routine_ptr);
            routine(arg)
        } else {
            core::ptr::null_mut()
        }
    }
}

// ============================================================================
// Constructor - Se ejecuta ANTES que main()
// ============================================================================

#[no_mangle]
pub unsafe extern "C" fn stasis_init() {
    // Obtener PID del proceso
    STASIS_PID = raw_getpid();

    // Registrar TID del thread principal
    let main_tid = raw_gettid();
    THREAD_TIDS[0] = main_tid;
    THREAD_COUNT = 1;

    // Resolver write() original
    let real_write_ptr = dlsym(RTLD_NEXT, b"write\0".as_ptr());
    if real_write_ptr.is_null() {
        log_raw(b"[STASIS FATAL] dlsym(write) failed\n");
        return;
    }
    REAL_WRITE = Some(core::mem::transmute(real_write_ptr));

    // Resolver pthread_create() original
    let real_pthread_ptr = dlsym(RTLD_NEXT, b"pthread_create\0".as_ptr());
    if real_pthread_ptr.is_null() {
        log_raw(b"[STASIS WARN] dlsym(pthread_create) failed\n");
    } else {
        REAL_PTHREAD_CREATE = Some(core::mem::transmute(real_pthread_ptr));
    }

    // Instalar signal handlers via glibc sigaction
    // SIGUSR2: congela el thread que lo recibe
    let ret2 = install_signal_handler(SIGUSR2, stasis_freeze_handler as *const c_void);
    if ret2 != 0 {
        log_raw(b"[STASIS WARN] sigaction(SIGUSR2) fallo\n");
    }

    // SIGUSR1: broadcast SIGUSR2 a todos los threads (freeze global)
    let ret1 = install_signal_handler(SIGUSR1, stasis_freeze_trigger as *const c_void);
    if ret1 != 0 {
        log_raw(b"[STASIS WARN] sigaction(SIGUSR1) fallo\n");
    }

    log_raw(b"[STASIS] Hook activo. kill -10 <pid> = freeze all\n");
}

#[link_section = ".init_array"]
#[used]
static INIT_ARRAY: unsafe extern "C" fn() = stasis_init;

// ============================================================================
// Hook: write
// ============================================================================

#[no_mangle]
pub unsafe extern "C" fn write(fd: i32, buf: *const c_void, count: usize) -> isize {
    if fd == 1 && !IN_HOOK {
        IN_HOOK = true;
        log_raw(b"[STASIS HOOK] Capturado write en stdout\n");
        IN_HOOK = false;
    }

    match REAL_WRITE {
        Some(func) => func(fd, buf, count),
        None => raw_syscall_write(fd, buf as *const u8, count),
    }
}

// ============================================================================
// Hook: pthread_create - Registra threads + captura TIDs
// ============================================================================

#[no_mangle]
pub unsafe extern "C" fn pthread_create(
    thread: *mut c_void,
    attr: *const c_void,
    start_routine: extern "C" fn(*mut c_void) -> *mut c_void,
    arg: *mut c_void,
) -> i32 {
    // Buscar slot libre
    let mut slot_idx: usize = MAX_THREADS;
    for i in 0..MAX_THREADS {
        if !SLOT_ACTIVE[i] {
            slot_idx = i;
            break;
        }
    }

    if slot_idx >= MAX_THREADS {
        log_raw(b"[STASIS WARN] No hay slots libres\n");
        return match REAL_PTHREAD_CREATE {
            Some(func) => func(thread, attr, start_routine, arg),
            None => -1,
        };
    }

    // Guardar routine y arg en el slot
    SLOT_ROUTINES[slot_idx] = start_routine as *const c_void;
    SLOT_ARGS[slot_idx] = arg;
    SLOT_ACTIVE[slot_idx] = true;

    // Llamar pthread_create REAL con nuestro wrapper y slot_idx como arg
    match REAL_PTHREAD_CREATE {
        Some(func) => func(
            thread,
            attr,
            stasis_thread_wrapper,
            slot_idx as *mut c_void,
        ),
        None => {
            SLOT_ACTIVE[slot_idx] = false;
            log_raw(b"[STASIS FATAL] pthread_create real no disponible\n");
            -1
        }
    }
}

// ============================================================================
// Panic handler
// ============================================================================

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    unsafe { log_raw(b"[STASIS PANIC]\n") };
    loop {}
}
