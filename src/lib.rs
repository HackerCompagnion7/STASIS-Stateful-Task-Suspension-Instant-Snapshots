#![no_std] // Sin librería estándar para evitar dependencias ocultas

use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, Ordering};

// ============================================================================
// Constantes
// ============================================================================
//
// SOLO usamos SIGUSR2 (senal 12).
//
// RAZON: En Android/Bionic, SIGUSR1 (senal 10) es usado internamente
// por el runtime. Incluso si sigaction() retorna exito, el handler
// no se respeta. SIGUSR2 es limpio y confiable en Bionic.
//
// Flujo:
//   kill -12 <pid>  →  SIGUSR2 llega a un thread
//   Ese thread es el "trigger": envía SIGUSR2 a todos los demas TIDs
//   Los otros threads reciben SIGUSR2 y se congelan
//   El trigger se congela al final
//   Resultado: freeze global
//
// El guard FREEZE_BROADCAST_DONE evita que los threads que reciben
// SIGUSR2 del broadcast intenten hacer otro broadcast (tormenta).
// ============================================================================

const RTLD_NEXT: *const c_void = -1isize as *const c_void;
const MAX_THREADS: usize = 128;
const SIGUSR2: i32 = 12; // UNICA señal para freeze

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
type FnSigaction = unsafe extern "C" fn(i32, *const c_void, *mut c_void) -> i32;
type FnSignal = unsafe extern "C" fn(i32, *const c_void) -> *const c_void;

// ============================================================================
// Estado global
// ============================================================================

static mut REAL_WRITE: Option<FnWrite> = None;
static mut REAL_PTHREAD_CREATE: Option<FnPthreadCreate> = None;
static mut REAL_SIGACTION: Option<FnSigaction> = None;
static mut REAL_SIGNAL: Option<FnSignal> = None;
static IN_HOOK: AtomicBool = AtomicBool::new(false);

// Thread tracking
static mut SLOT_ROUTINES: [*const c_void; MAX_THREADS] = [core::ptr::null(); MAX_THREADS];
static mut SLOT_ARGS: [*mut c_void; MAX_THREADS] = [core::ptr::null_mut(); MAX_THREADS];
static mut SLOT_ACTIVE: [bool; MAX_THREADS] = [false; MAX_THREADS];
static mut THREAD_TIDS: [i32; MAX_THREADS] = [0; MAX_THREADS];
static mut THREAD_COUNT: usize = 0;
static mut STASIS_PID: i32 = 0;

// Guard: solo el PRIMER thread en recibir SIGUSR2 hace el broadcast
static FREEZE_BROADCAST_DONE: AtomicBool = AtomicBool::new(false);

// Flag: handlers instalados y listos
static mut HANDLERS_INSTALLED: bool = false;

// ============================================================================
// raw_syscall_write - Naked, funciona en ambas arquitecturas
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
#[allow(dead_code)]
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
#[allow(dead_code)]
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
// Syscall helpers por arquitectura
// ============================================================================
//
// gettid:    x86_64=186, aarch64=178
// getpid:    x86_64=39,  aarch64=172
// tgkill:    x86_64=234, aarch64=131
// nanosleep: x86_64=35,  aarch64=101
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
// Signal handler installation via libc sigaction()
// ============================================================================
//
// Layout por libc:
//   - glibc:  sa_mask = 128 bytes (16 x u64)
//   - Bionic: sa_mask = 8 bytes   (1 x u64)
// ============================================================================

#[cfg(all(target_arch = "x86_64", target_env = "gnu"))]
#[repr(C)]
struct LibcSigaction {
    sa_handler: *const c_void,
    sa_mask: [u64; 16],
    sa_flags: i32,
    _pad: i32,
    sa_restorer: *const c_void,
}

#[cfg(target_arch = "aarch64")]
#[repr(C)]
struct LibcSigaction {
    sa_handler: *const c_void,
    sa_mask: [u64; 1],
    sa_flags: i32,
    _pad: i32,
    sa_restorer: *const c_void,
}

#[cfg(all(not(all(target_arch = "x86_64", target_env = "gnu")), not(target_arch = "aarch64")))]
#[repr(C)]
struct LibcSigaction {
    sa_handler: *const c_void,
    sa_mask: [u64; 16],
    sa_flags: i32,
    _pad: i32,
    sa_restorer: *const c_void,
}

const SA_RESTART: i32 = 0x10000000;

#[cfg(all(target_arch = "x86_64", target_env = "gnu"))]
unsafe fn install_signal_handler(sig: i32, handler: *const c_void) -> i32 {
    let act = LibcSigaction {
        sa_handler: handler,
        sa_mask: [0; 16],
        sa_flags: SA_RESTART,
        _pad: 0,
        sa_restorer: core::ptr::null(),
    };
    match REAL_SIGACTION {
        Some(func) => func(sig, &act as *const _ as *const c_void, core::ptr::null_mut()),
        None => -1,
    }
}

#[cfg(target_arch = "aarch64")]
unsafe fn install_signal_handler(sig: i32, handler: *const c_void) -> i32 {
    let act = LibcSigaction {
        sa_handler: handler,
        sa_mask: [0; 1],
        sa_flags: SA_RESTART,
        _pad: 0,
        sa_restorer: core::ptr::null(),
    };
    match REAL_SIGACTION {
        Some(func) => func(sig, &act as *const _ as *const c_void, core::ptr::null_mut()),
        None => -1,
    }
}

#[cfg(all(not(all(target_arch = "x86_64", target_env = "gnu")), not(target_arch = "aarch64")))]
unsafe fn install_signal_handler(sig: i32, handler: *const c_void) -> i32 {
    let act = LibcSigaction {
        sa_handler: handler,
        sa_mask: [0; 16],
        sa_flags: SA_RESTART,
        _pad: 0,
        sa_restorer: core::ptr::null(),
    };
    match REAL_SIGACTION {
        Some(func) => func(sig, &act as *const _ as *const c_void, core::ptr::null_mut()),
        None => -1,
    }
}

// Verificar que nuestro handler sigue instalado
unsafe fn verify_handler(sig: i32, expected: *const c_void) -> bool {
    let mut old_act: LibcSigaction = core::mem::zeroed();
    match REAL_SIGACTION {
        Some(func) => {
            func(sig, core::ptr::null(), &mut old_act as *mut _ as *mut c_void);
            old_act.sa_handler == expected
        }
        None => false,
    }
}

// Reinstalar handler (respaldo)
unsafe fn reinstall_handler() {
    install_signal_handler(SIGUSR2, stasis_freeze_handler as *const c_void);
}

// ============================================================================
// Helper: log a stderr via syscall directa
// ============================================================================

unsafe fn log_raw(msg: &[u8]) {
    raw_syscall_write(2, msg.as_ptr(), msg.len());
}

// ============================================================================
// Importar funciones de libc
// ============================================================================

extern "C" {
    fn dlsym(handle: *const c_void, symbol: *const u8) -> *const c_void;
}

// ============================================================================
// UNICO Signal handler: SIGUSR2 (senal 12)
// ============================================================================
//
// Cuando un thread recibe SIGUSR2:
//   1. Si es el PRIMERO (FREEZE_BROADCAST_DONE == false):
//      - Hace broadcast de SIGUSR2 a todos los otros TIDs
//      - Marca FREEZE_BROADCAST_DONE = true
//   2. Todos los threads (incluido el trigger) entran en nanosleep loop
//      CPU cae a ~0%. Thread congelado.
//
// Este handler solo usa raw syscalls. Cero libc, cero recursion.
// ============================================================================

#[unsafe(no_mangle)]
unsafe extern "C" fn stasis_freeze_handler(_sig: i32) {
    log_raw(b"[STASIS] Signal recibida\n");

    // Solo el primer thread hace el broadcast
    if FREEZE_BROADCAST_DONE.compare_exchange(
        false, true, Ordering::SeqCst, Ordering::SeqCst
    ).is_ok() {
        log_raw(b"[STASIS] >>> FREEZE GLOBAL INICIADO <<<\n");

        let count = THREAD_COUNT;
        let pid = STASIS_PID;
        let my_tid = raw_gettid();

        // Broadcast SIGUSR2 a todos los TIDs excepto este
        for i in 0..count {
            let tid = THREAD_TIDS[i];
            if tid > 0 && tid != my_tid {
                raw_tgkill(pid, tid, SIGUSR2);
            }
        }

        // Pequena pausa para que los otros threads entren al handler
        raw_nanosleep();
    }

    // Todos los threads: congelar
    log_raw(b"[STASIS FREEZE] Thread congelado\n");
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
        let tid = raw_gettid();

        if THREAD_COUNT < MAX_THREADS {
            THREAD_TIDS[THREAD_COUNT] = tid;
            THREAD_COUNT += 1;
        }

        log_raw(b"[STASIS] Thread registrado (TID capturado)\n");

        let routine_ptr = SLOT_ROUTINES[slot_idx];
        let arg = SLOT_ARGS[slot_idx];
        SLOT_ACTIVE[slot_idx] = false;

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
    STASIS_PID = raw_getpid();

    let main_tid = raw_gettid();
    THREAD_TIDS[0] = main_tid;
    THREAD_COUNT = 1;

    // Resolver funciones originales via dlsym
    // IMPORTANTE: resolver sigaction ANTES de instalar handlers

    let real_sigaction_ptr = dlsym(RTLD_NEXT, b"sigaction\0".as_ptr());
    if real_sigaction_ptr.is_null() {
        log_raw(b"[STASIS FATAL] dlsym(sigaction) failed\n");
        return;
    }
    REAL_SIGACTION = Some(core::mem::transmute(real_sigaction_ptr));

    let real_signal_ptr = dlsym(RTLD_NEXT, b"signal\0".as_ptr());
    if real_signal_ptr.is_null() {
        log_raw(b"[STASIS WARN] dlsym(signal) failed\n");
    } else {
        REAL_SIGNAL = Some(core::mem::transmute(real_signal_ptr));
    }

    let real_write_ptr = dlsym(RTLD_NEXT, b"write\0".as_ptr());
    if real_write_ptr.is_null() {
        log_raw(b"[STASIS FATAL] dlsym(write) failed\n");
        return;
    }
    REAL_WRITE = Some(core::mem::transmute(real_write_ptr));

    let real_pthread_ptr = dlsym(RTLD_NEXT, b"pthread_create\0".as_ptr());
    if real_pthread_ptr.is_null() {
        log_raw(b"[STASIS WARN] dlsym(pthread_create) failed\n");
    } else {
        REAL_PTHREAD_CREATE = Some(core::mem::transmute(real_pthread_ptr));
    }

    // Instalar UNICO handler: SIGUSR2
    let ret = install_signal_handler(SIGUSR2, stasis_freeze_handler as *const c_void);
    if ret != 0 {
        log_raw(b"[STASIS FATAL] sigaction(SIGUSR2) fallo\n");
    } else {
        log_raw(b"[STASIS] Handler SIGUSR2 instalado OK\n");
    }

    // Verificar
    if verify_handler(SIGUSR2, stasis_freeze_handler as *const c_void) {
        log_raw(b"[STASIS] SIGUSR2 handler verificado OK\n");
    } else {
        log_raw(b"[STASIS FATAL] SIGUSR2 handler no coincide - reintentando\n");
        install_signal_handler(SIGUSR2, stasis_freeze_handler as *const c_void);
        if verify_handler(SIGUSR2, stasis_freeze_handler as *const c_void) {
            log_raw(b"[STASIS] SIGUSR2 handler reinstalado OK\n");
        } else {
            log_raw(b"[STASIS FATAL] SIGUSR2 handler IMPOSIBLE de instalar\n");
        }
    }

    HANDLERS_INSTALLED = true;

    log_raw(b"[STASIS] Hook activo. kill -12 <pid> = freeze all\n");
}

#[link_section = ".init_array"]
#[used]
static INIT_ARRAY: unsafe extern "C" fn() = stasis_init;

// ============================================================================
// Hook: write
// ============================================================================

#[no_mangle]
pub unsafe extern "C" fn write(fd: i32, buf: *const c_void, count: usize) -> isize {
    if fd == 1 && !IN_HOOK.load(Ordering::Relaxed) {
        IN_HOOK.store(true, Ordering::Relaxed);
        log_raw(b"[STASIS HOOK] Capturado write en stdout\n");
        IN_HOOK.store(false, Ordering::Relaxed);
    }

    match REAL_WRITE {
        Some(func) => func(fd, buf, count),
        None => raw_syscall_write(fd, buf as *const u8, count),
    }
}

// ============================================================================
// Hook: pthread_create - Registra threads + captura TIDs + reinstala handler
// ============================================================================

#[no_mangle]
pub unsafe extern "C" fn pthread_create(
    thread: *mut c_void,
    attr: *const c_void,
    start_routine: extern "C" fn(*mut c_void) -> *mut c_void,
    arg: *mut c_void,
) -> i32 {
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

    SLOT_ROUTINES[slot_idx] = start_routine as *const c_void;
    SLOT_ARGS[slot_idx] = arg;
    SLOT_ACTIVE[slot_idx] = true;

    let result = match REAL_PTHREAD_CREATE {
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
    };

    // RESPALDO: Reinstalar handler despues de cada pthread_create
    if HANDLERS_INSTALLED {
        reinstall_handler();
    }

    result
}

// ============================================================================
// Hook: sigaction - Protege SIGUSR2 contra sobrescritura
// ============================================================================

#[no_mangle]
pub unsafe extern "C" fn sigaction(
    sig: i32,
    act: *const c_void,
    oldact: *mut c_void,
) -> i32 {
    if sig == SIGUSR2 {
        if !oldact.is_null() {
            match REAL_SIGACTION {
                Some(func) => {
                    func(sig, core::ptr::null(), oldact);
                }
                None => {}
            }
        }
        if !act.is_null() {
            log_raw(b"[STASIS GUARD] sigaction blocked on SIGUSR2\n");
        }
        return 0;
    }

    match REAL_SIGACTION {
        Some(func) => func(sig, act, oldact),
        None => -1,
    }
}

// ============================================================================
// Hook: signal - Protege SIGUSR2 contra sobrescritura
// ============================================================================

#[no_mangle]
pub unsafe extern "C" fn signal(
    sig: i32,
    handler: *const c_void,
) -> *const c_void {
    if sig == SIGUSR2 {
        let mut old_act: LibcSigaction = core::mem::zeroed();
        match REAL_SIGACTION {
            Some(func) => {
                func(sig, core::ptr::null(), &mut old_act as *mut _ as *mut c_void);
            }
            None => {}
        }
        log_raw(b"[STASIS GUARD] signal() blocked on SIGUSR2\n");
        return old_act.sa_handler;
    }

    match REAL_SIGNAL {
        Some(func) => func(sig, handler),
        None => core::ptr::null(),
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
